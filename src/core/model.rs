use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use unicode_normalization::UnicodeNormalization;

pub fn valid_path_input(path: &Path) -> bool {
    !path.as_os_str().is_empty() && !path.as_os_str().as_bytes().contains(&0)
}

pub type EntryId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Package,
    File,
    Symlink,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub path: PathBuf,
    pub name: String,
    pub key: String,
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<i64>,
    pub metadata_loaded: bool,
    pub readonly: bool,
}

impl Entry {
    pub fn from_dirent(item: fs::DirEntry) -> std::io::Result<Self> {
        let path = item.path();
        let ty = item.file_type()?;
        let name = item.file_name().to_string_lossy().into_owned();
        let kind = if ty.is_symlink() {
            Kind::Symlink
        } else if ty.is_dir() && is_package(&path) {
            Kind::Package
        } else if ty.is_dir() {
            Kind::Folder
        } else {
            Kind::File
        };
        Ok(Self {
            id: path_id(&path),
            key: search_key(&name),
            path,
            name,
            kind,
            size: None,
            modified: None,
            metadata_loaded: false,
            readonly: false,
        })
    }
    pub fn load_metadata(&mut self) {
        if let Ok(m) = fs::symlink_metadata(&self.path) {
            self.size = (self.kind != Kind::Folder).then_some(m.len());
            self.modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|v| v.as_secs() as i64);
            self.readonly = m.permissions().readonly();
        }
        self.metadata_loaded = true;
    }
    pub fn kind_label(&self) -> String {
        match self.kind {
            Kind::Folder => "폴더".into(),
            Kind::Package => "응용 프로그램 · 패키지".into(),
            Kind::Symlink => "심볼릭 링크".into(),
            Kind::File => self
                .path
                .extension()
                .map(|s| format!("{} 파일", s.to_string_lossy().to_uppercase()))
                .unwrap_or_else(|| "파일".into()),
        }
    }
}

pub fn path_id(path: &Path) -> EntryId {
    path.as_os_str()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100000001b3)
        })
}
pub fn search_key(s: &str) -> String {
    s.nfc().flat_map(char::to_lowercase).collect()
}
pub fn is_package(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_lowercase().as_str(),
            "app" | "bundle" | "framework" | "photoslibrary" | "pages" | "numbers" | "key" | "rtfd"
        )
    })
}
pub fn hidden(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|s| s.as_bytes().starts_with(b"."))
}
pub fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}
pub fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    std::ffi::OsString::from_vec(bytes).into()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortField {
    #[default]
    Name,
    Modified,
    Size,
    Kind,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyboardProfile {
    #[default]
    Windows,
    Finder,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    #[default]
    Details,
    Icons,
}

pub fn sort_entries(entries: &mut [Entry], field: SortField, descending: bool) {
    entries.sort_by(|a, b| {
        let folders = (b.kind == Kind::Folder).cmp(&(a.kind == Kind::Folder));
        if folders != Ordering::Equal {
            return folders;
        }
        let primary = match field {
            SortField::Name => natural_cmp(&a.key, &b.key),
            SortField::Modified => a.modified.cmp(&b.modified),
            SortField::Size => a.size.cmp(&b.size),
            SortField::Kind => a.kind_label().cmp(&b.kind_label()),
        }
        .then_with(|| natural_cmp(&a.key, &b.key))
        .then_with(|| a.path.cmp(&b.path));
        if descending {
            primary.reverse()
        } else {
            primary
        }
    });
}

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (sa, sb) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let (mut za, mut zb) = (sa, sb);
            while za + 1 < i && a[za] == b'0' {
                za += 1;
            }
            while zb + 1 < j && b[zb] == b'0' {
                zb += 1;
            }
            let c = (i - za)
                .cmp(&(j - zb))
                .then_with(|| a[za..i].cmp(&b[zb..j]))
                .then_with(|| (i - sa).cmp(&(j - sb)));
            if c != Ordering::Equal {
                return c;
            }
        } else {
            let c = a[i].cmp(&b[j]);
            if c != Ordering::Equal {
                return c;
            }
            i += 1;
            j += 1;
        }
    }
    a.len().cmp(&b.len())
}

pub fn format_size(size: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut n = size as f64;
    let mut i = 0;
    while n >= 1024.0 && i < units.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{size} B")
    } else {
        format!("{n:.1} {}", units[i])
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime: i64,
    pub mtime_ns: i64,
    pub ctime: i64,
    pub ctime_ns: i64,
    pub mode: u32,
}
impl Fingerprint {
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let m = fs::symlink_metadata(path)?;
        Ok(Self::from_metadata(&m))
    }
    pub(crate) fn from_metadata(m: &fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            size: m.len(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
            mode: m.mode(),
        }
    }
    pub fn matches(&self, path: &Path) -> bool {
        matches!(Self::read(path),Ok(ref f) if f==self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: super::theme::ThemeId,
    pub keyboard: KeyboardProfile,
    pub system_f2_rename: bool,
    pub access_intro_dismissed: bool,
    pub hidden: bool,
    pub compact: bool,
    pub view: ViewMode,
    pub preview: bool,
    pub favorites: Vec<Vec<u8>>,
    pub pinned_locations: Vec<Vec<u8>>,
    pub shortcut_labels: std::collections::BTreeMap<String, String>,
    pub icon_size: IconSize,
    pub sidebar: SidebarPreferences,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum IconSize {
    #[default]
    Small,
    Medium,
    Large,
}
impl IconSize {
    pub fn index(self) -> isize {
        match self {
            Self::Small => 0,
            Self::Medium => 1,
            Self::Large => 2,
        }
    }
    pub fn from_index(index: isize) -> Self {
        match index {
            0 => Self::Small,
            1 => Self::Medium,
            _ => Self::Large,
        }
    }
    pub fn dimensions(self) -> (f64, f64, f64) {
        match self {
            Self::Small => (136., 112., 54.),
            Self::Medium => (168., 154., 96.),
            Self::Large => (224., 210., 152.),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidebarPreferences {
    pub hidden: Vec<String>,
    pub order: Vec<String>,
    pub collapsed: Vec<String>,
}
impl SidebarPreferences {
    pub fn visible(&self, key: &str) -> bool {
        !self.hidden.iter().any(|k| k == key)
    }
    pub fn toggle(&mut self, key: &str) {
        if self.visible(key) {
            self.hidden.push(key.into());
        } else {
            self.hidden.retain(|k| k != key);
        }
    }
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: super::theme::ThemeId::default(),
            keyboard: KeyboardProfile::Windows,
            system_f2_rename: false,
            access_intro_dismissed: false,
            hidden: false,
            compact: false,
            view: ViewMode::Details,
            preview: false,
            favorites: vec![],
            pinned_locations: vec![],
            shortcut_labels: Default::default(),
            icon_size: IconSize::Small,
            sidebar: SidebarPreferences::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedTab {
    pub path: Vec<u8>,
    pub sort: SortField,
    pub descending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_sort_naturally() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("x9", "x0009"), Ordering::Less);
        assert_eq!(
            natural_cmp("a999999999999999999999", "a1000000000000000000000"),
            Ordering::Less
        );
    }
    #[test]
    fn unicode_search_does_not_rewrite_paths() {
        let a = "한글.TXT";
        assert_eq!(search_key(a), search_key("한글.txt"));
        assert_eq!(path_from_bytes(path_bytes(Path::new(a))), Path::new(a));
    }
    #[test]
    fn raw_names_roundtrip() {
        let p = path_from_bytes(vec![b'/', 0xff, b'a']);
        assert_eq!(path_bytes(&p), vec![b'/', 0xff, b'a']);
    }
}
