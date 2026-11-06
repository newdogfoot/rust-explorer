use super::model::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{OpenOptions, Permissions},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Store(Arc<Mutex<Connection>>);
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // Resolve intended directory aliases (macOS /var -> /private/var),
        // while SQLite and open(O_NOFOLLOW) still reject a symlink leaf.
        let path = std::fs::canonicalize(parent)
            .map_err(|e| e.to_string())?
            .join(path.file_name().ok_or("설정 파일 경로가 필요합니다")?);
        // The database contains private paths and operation history. Create it
        // atomically as owner-only and refuse a substituted symlink or device.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err("현재 사용자의 일반 설정 파일이 아닙니다".into());
        }
        file.set_permissions(Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        let db = Connection::open_with_flags(
            &path,
            (OpenFlags::default() & !OpenFlags::SQLITE_OPEN_URI) | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,created INTEGER NOT NULL DEFAULT(unixepoch()),status TEXT NOT NULL,payload TEXT NOT NULL); PRAGMA user_version=1;").map_err(|e|e.to_string())?;
        Ok(Self(Arc::new(Mutex::new(db))))
    }
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let db = self.0.lock().ok()?;
        let v: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
                r.get(0)
            })
            .optional()
            .ok()?;
        v.and_then(|s| serde_json::from_str(&s).ok())
    }
    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
        self.0.lock().map_err(|e|e.to_string())?.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,text]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn preferences(&self) -> Preferences {
        self.get("preferences").unwrap_or_default()
    }
    pub fn save_job<T: Serialize>(&self, id: &str, status: &str, job: &T) -> Result<(), String> {
        let value = serde_json::to_string(job).map_err(|e| e.to_string())?;
        self.0.lock().map_err(|e|e.to_string())?.execute("INSERT INTO jobs(id,status,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET status=excluded.status,payload=excluded.payload",params![id,status,value]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn jobs<T: DeserializeOwned>(&self) -> Vec<(String, String, T)> {
        let Ok(db) = self.0.lock() else { return vec![] };
        let Ok(mut st)=db.prepare("SELECT id,status,payload FROM jobs WHERE status != 'internal' ORDER BY created DESC,rowid DESC LIMIT 100")else{return vec![]};
        let Ok(rows) = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) else {
            return vec![];
        };
        rows.flatten()
            .filter_map(|(id, status, json)| {
                serde_json::from_str(&json)
                    .ok()
                    .map(|job| (id, status, job))
            })
            .collect()
    }
    pub fn interrupt_running(&self) -> Result<usize, String> {
        self.0.lock().map_err(|e|e.to_string())?.execute("UPDATE jobs SET status='interrupted' WHERE status IN ('queued','running','waiting')",[]).map_err(|e|e.to_string())
    }
    pub fn job<T: DeserializeOwned>(&self, id: &str) -> Option<T> {
        let db = self.0.lock().ok()?;
        let value: String = db
            .query_row("SELECT payload FROM jobs WHERE id=?", [id], |r| r.get(0))
            .ok()?;
        serde_json::from_str(&value).ok()
    }
    pub fn map_jobs<T: DeserializeOwned, R>(
        &self,
        unfinished: bool,
        mut map: impl FnMut(String, String, T) -> R,
    ) -> Vec<R> {
        let Ok(db) = self.0.lock() else { return vec![] };
        let query = if unfinished {
            "SELECT id,status,payload FROM jobs WHERE status IN ('queued','running','waiting','interrupted','undoing') ORDER BY created DESC,rowid DESC LIMIT 100"
        } else {
            "SELECT id,status,payload FROM jobs WHERE status != 'internal' ORDER BY created DESC,rowid DESC LIMIT 100"
        };
        let Ok(mut statement) = db.prepare(query) else {
            return vec![];
        };
        let Ok(rows) = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) else {
            return vec![];
        };
        rows.flatten()
            .filter_map(|(id, status, value)| {
                serde_json::from_str(&value)
                    .ok()
                    .map(|record| map(id, status, record))
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_and_raw_paths_persist() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("state.sqlite");
        let s = Store::open(&path).unwrap();
        let p = Preferences {
            keyboard: KeyboardProfile::Finder,
            favorites: vec![vec![b'/', 255]],
            ..Default::default()
        };
        s.set("preferences", &p).unwrap();
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.preferences().favorites, p.favorites);
        assert_eq!(s.preferences().keyboard, KeyboardProfile::Finder);
    }
    #[test]
    fn crash_recovery_marks_pending_jobs() {
        let t = tempfile::tempdir().unwrap();
        let s = Store::open(&t.path().join("state.sqlite")).unwrap();
        s.save_job("id", "running", &vec![1, 2]).unwrap();
        assert_eq!(s.interrupt_running().unwrap(), 1);
        assert_eq!(s.jobs::<Vec<u8>>()[0].1, "interrupted");
    }
    #[test]
    fn old_preferences_upgrade_without_losing_sidebar_or_keyboard_choices() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("state.sqlite")).unwrap();
        store.set("preferences",&serde_json::json!({"keyboard":"Finder","view":"Icons","favorites":[[47,116,109,112]]})).unwrap();
        let mut prefs = store.preferences();
        assert_eq!(prefs.keyboard, crate::core::model::KeyboardProfile::Finder);
        assert_eq!(prefs.favorites, vec![b"/tmp".to_vec()]);
        assert_eq!(prefs.icon_size, crate::core::model::IconSize::Small);
        prefs.icon_size = crate::core::model::IconSize::Large;
        prefs.sidebar.hidden.push("pictures".into());
        prefs.sidebar.order = vec!["documents".into(), "home".into()];
        store.set("preferences", &prefs).unwrap();
        let restored = store.preferences();
        assert_eq!(restored.icon_size, prefs.icon_size);
        assert_eq!(restored.sidebar, prefs.sidebar);
    }
}
