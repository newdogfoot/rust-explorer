use super::{model::*, worker::Cancellation};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub enum ScanUpdate {
    Batch(Vec<Entry>),
    Complete { entries: Vec<Entry>, skipped: usize },
    Failed(ScanFailure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanFailure {
    pub message: String,
    pub access_denied: bool,
}
impl ScanFailure {
    pub fn from_io(path: &Path, error: &io::Error) -> Self {
        Self {
            message: friendly_error(path, error),
            access_denied: error.kind() == io::ErrorKind::PermissionDenied,
        }
    }
}
impl From<String> for ScanFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            access_denied: false,
        }
    }
}
impl From<&str> for ScanFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl std::fmt::Display for ScanFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub fn scan_directory(
    path: &Path,
    show_hidden: bool,
    sort: SortField,
    descending: bool,
    cancel: &Cancellation,
    mut emit: impl FnMut(ScanUpdate),
) {
    if cancel.cancelled() {
        return;
    }
    let iter = match fs::read_dir(path) {
        Ok(i) => i,
        Err(e) => {
            emit(ScanUpdate::Failed(ScanFailure::from_io(path, &e)));
            return;
        }
    };
    let mut entries = Vec::new();
    let mut batch = Vec::with_capacity(128);
    let mut skipped = 0;
    let mut last = Instant::now();
    for item in iter {
        if cancel.cancelled() {
            return;
        }
        match item.and_then(Entry::from_dirent) {
            Ok(mut e) => {
                if !show_hidden && hidden(&e.path) {
                    continue;
                }
                if sort != SortField::Name {
                    e.load_metadata();
                }
                batch.push(e.clone());
                entries.push(e);
            }
            Err(_) => skipped += 1,
        }
        if batch.len() >= 128 || last.elapsed() >= Duration::from_millis(30) {
            if !batch.is_empty() {
                emit(ScanUpdate::Batch(std::mem::take(&mut batch)));
            }
            last = Instant::now();
        }
    }
    if cancel.cancelled() {
        return;
    }
    if !batch.is_empty() {
        emit(ScanUpdate::Batch(batch));
    }
    sort_entries(&mut entries, sort, descending);
    if !cancel.cancelled() {
        emit(ScanUpdate::Complete { entries, skipped });
    }
}

pub fn search_directory(
    root: &Path,
    query: &str,
    show_hidden: bool,
    cancel: &Cancellation,
    mut emit: impl FnMut(ScanUpdate),
) {
    let query = search_key(query);
    let mut stack = vec![root.to_owned()];
    let mut result = Vec::new();
    let mut batch = Vec::new();
    let mut skipped = 0;
    let mut last = Instant::now();
    while let Some(dir) = stack.pop() {
        if cancel.cancelled() {
            return;
        }
        let iter = match fs::read_dir(&dir) {
            Ok(i) => i,
            Err(e) => {
                if dir == root {
                    emit(ScanUpdate::Failed(ScanFailure::from_io(root, &e)));
                    return;
                }
                skipped += 1;
                continue;
            }
        };
        for item in iter {
            if cancel.cancelled() {
                return;
            }
            let e = match item.and_then(Entry::from_dirent) {
                Ok(e) => e,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if !show_hidden && hidden(&e.path) {
                continue;
            }
            if e.kind == Kind::Folder {
                stack.push(e.path.clone());
            }
            if e.key.contains(&query) {
                batch.push(e.clone());
                result.push(e);
            }
            if batch.len() >= 128 || last.elapsed() > Duration::from_millis(60) {
                if !batch.is_empty() {
                    emit(ScanUpdate::Batch(std::mem::take(&mut batch)));
                }
                last = Instant::now();
            }
        }
    }
    if !batch.is_empty() {
        emit(ScanUpdate::Batch(batch));
    }
    sort_entries(&mut result, SortField::Name, false);
    if !cancel.cancelled() {
        emit(ScanUpdate::Complete {
            entries: result,
            skipped,
        });
    }
}

pub fn friendly_error(path: &Path, e: &io::Error) -> String {
    let reason = match e.kind() {
        io::ErrorKind::PermissionDenied => "접근 권한이 없습니다",
        io::ErrorKind::NotFound => "폴더가 이동되었거나 저장장치의 연결이 해제되었습니다",
        io::ErrorKind::NotADirectory => "폴더가 아닙니다",
        _ => "폴더를 읽을 수 없습니다",
    };
    format!("{reason}\n{}\n{e}", path.display())
}

pub fn directory_size(root: &Path, cancel: &Cancellation) -> io::Result<(u64, u64)> {
    let mut dirs = vec![root.to_path_buf()];
    let (mut bytes, mut files) = (0u64, 0u64);
    while let Some(dir) = dirs.pop() {
        for e in fs::read_dir(dir)? {
            cancel.check()?;
            let e = e?;
            let m = e.path().symlink_metadata()?;
            if m.is_dir() {
                dirs.push(e.path());
            } else {
                bytes = bytes.saturating_add(m.len());
                files += 1;
            }
        }
    }
    Ok((bytes, files))
}

pub fn selection_size(mut paths: Vec<PathBuf>, cancel: &Cancellation) -> io::Result<(u64, u64)> {
    paths.sort();
    paths.dedup();
    let mut entries = Vec::with_capacity(paths.len());
    let mut directories = std::collections::HashSet::new();
    for path in paths {
        cancel.check()?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            directories.insert(path.clone());
        }
        entries.push((path, metadata));
    }
    let (mut bytes, mut files) = (0u64, 0u64);
    for (path, metadata) in entries {
        cancel.check()?;
        if path.ancestors().skip(1).any(|p| directories.contains(p)) {
            continue;
        }
        let (b, n) = if metadata.is_dir() {
            directory_size(&path, cancel)?
        } else {
            (metadata.len(), 1)
        };
        bytes = bytes.saturating_add(b);
        files += n;
    }
    Ok((bytes, files))
}

pub fn visible_metadata(entries: &[Entry], range: std::ops::Range<usize>) -> Vec<PathBuf> {
    entries
        .get(range)
        .unwrap_or_default()
        .iter()
        .filter(|e| !e.metadata_loaded)
        .map(|e| e.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn search_skips_links_packages_and_hidden() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("inside.app")).unwrap();
        fs::write(d.path().join("inside.app/hit.txt"), b"a").unwrap();
        fs::write(d.path().join(".hit"), b"b").unwrap();
        fs::write(d.path().join("hit.txt"), b"c").unwrap();
        symlink(d.path(), d.path().join("loop")).unwrap();
        let mut found = vec![];
        search_directory(d.path(), "hit", false, &Cancellation::default(), |u| {
            if let ScanUpdate::Complete { entries, .. } = u {
                found = entries
            }
        });
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "hit.txt");
    }
    #[test]
    fn cancellation_stops_scan() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), b"a").unwrap();
        let c = Cancellation::default();
        c.cancel();
        let mut count = 0;
        scan_directory(d.path(), false, SortField::Name, false, &c, |_| count += 1);
        assert_eq!(count, 0);
    }
    #[test]
    fn selection_size_does_not_double_count_nested_paths_or_follow_links() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("a"), b"1234").unwrap();
        fs::create_dir(folder.join("child")).unwrap();
        fs::write(folder.join("child/b"), b"123456").unwrap();
        symlink("..", folder.join("loop")).unwrap();
        let result = selection_size(
            vec![folder.clone(), folder.join("a"), folder.join("child")],
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(result, (12, 3));
    }
}
