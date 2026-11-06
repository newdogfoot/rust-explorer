//! Remember only folders explicitly selected in the system open panel.
//! Resolving saved bookmarks never opens UI or mounts a volume.
use super::checked_file_url_path;
use crate::core::model::{path_bytes, path_from_bytes, valid_path_input};
use objc2::{rc::Retained, runtime::Bool};
use objc2_foundation::{
    NSData, NSURL, NSURLBookmarkCreationOptions as Create,
    NSURLBookmarkResolutionOptions as Resolve,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

pub const MAX_FOLDERS: usize = 64;
const MAX_BOOKMARK_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedFolder {
    pub path: Vec<u8>,
    pub bookmark: Vec<u8>,
    pub scoped: bool,
}
impl SavedFolder {
    pub fn valid(&self) -> bool {
        let path = path_from_bytes(self.path.clone());
        path.is_absolute()
            && valid_path_input(&path)
            && !self.bookmark.is_empty()
            && self.bookmark.len() <= MAX_BOOKMARK_BYTES
    }
}

pub struct Scope {
    url: Retained<NSURL>,
    path: PathBuf,
    active: bool,
}
impl Scope {
    fn new(url: Retained<NSURL>, path: PathBuf) -> Self {
        // Non-sandboxed apps can already have implicit consent. A false
        // return is not a reliable TCC denial and must not be labelled one.
        let active = unsafe { url.startAccessingSecurityScopedResource() };
        Self { url, path, active }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                self.url.stopAccessingSecurityScopedResource();
            }
        }
    }
}

pub fn remember(url: Retained<NSURL>) -> Result<(SavedFolder, Scope), String> {
    let path = checked_file_url_path(&url).ok_or("올바른 로컬 폴더 주소가 아닙니다")?;
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    let scope = Scope::new(url, path.clone());
    let (data, scoped) = match scope
        .url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            Create::WithSecurityScope,
            None,
            None,
        ) {
        Ok(data) => (data, true),
        Err(_) => (
            scope
                .url
                .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                    Create::empty(),
                    None,
                    None,
                )
                .map_err(|e| e.localizedDescription().to_string())?,
            false,
        ),
    };
    let saved = SavedFolder {
        path: path_bytes(&path),
        bookmark: data.to_vec(),
        scoped,
    };
    if !saved.valid() {
        return Err("폴더 접근 정보를 저장할 수 없습니다".into());
    }
    Ok((saved, scope))
}

pub fn restore(saved: &SavedFolder) -> Result<(SavedFolder, Scope), String> {
    if !saved.valid() {
        return Err("저장된 폴더 정보가 올바르지 않습니다".into());
    }
    let mut stale = Bool::NO;
    // Scope::new starts access explicitly; avoid a second implicit start for
    // ordinary bookmarks so every acquisition has a matching stop.
    let mut options =
        Resolve::WithoutUI | Resolve::WithoutMounting | Resolve::WithoutImplicitStartAccessing;
    if saved.scoped {
        options |= Resolve::WithSecurityScope;
    }
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &NSData::from_vec(saved.bookmark.clone()),
            options,
            None,
            &mut stale,
        )
    }
    .map_err(|e| e.localizedDescription().to_string())?;
    let path = checked_file_url_path(&url).ok_or("저장된 폴더 주소가 올바르지 않습니다")?;
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    if stale.as_bool() {
        remember(url)
    } else {
        let mut saved = saved.clone();
        saved.path = path_bytes(&path);
        Ok((saved, Scope::new(url, path)))
    }
}

pub fn equivalent(a: &Path, b: &Path) -> bool {
    a == b
        || a.to_str()
            .zip(b.to_str())
            .is_some_and(|(a, b)| a.nfc().eq(b.nfc()))
}

#[derive(Default)]
pub struct Memory {
    pub saved: Vec<SavedFolder>,
    // Keep existing process grants alive even if their remembered list is
    // cleared, so clearing settings cannot interrupt an in-flight file job.
    scopes: Vec<Scope>,
}
impl Memory {
    pub fn load(saved: Vec<SavedFolder>) -> Self {
        let mut memory = Self::default();
        for record in saved
            .into_iter()
            .filter(SavedFolder::valid)
            .take(MAX_FOLDERS)
        {
            if let Ok((updated, scope)) = restore(&record) {
                let _ = memory.insert(updated, scope);
            } else {
                memory.saved.push(record);
            }
        }
        memory
    }
    pub fn insert(&mut self, saved: SavedFolder, scope: Scope) -> Result<(), String> {
        let path = path_from_bytes(saved.path.clone());
        if self.scopes.len() >= 256 && !self.scopes.iter().any(|s| equivalent(s.path(), &path)) {
            return Err(
                "이번 실행에서 선택한 폴더가 너무 많습니다. 작업을 마친 뒤 앱을 다시 열어 주세요."
                    .into(),
            );
        }
        self.saved
            .retain(|s| !equivalent(&path_from_bytes(s.path.clone()), &path));
        // Create the new grant before dropping a duplicate existing grant.
        self.scopes.retain(|s| !equivalent(s.path(), &path));
        self.saved.insert(0, saved);
        self.saved.truncate(MAX_FOLDERS);
        self.scopes.push(scope);
        Ok(())
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        self.saved
            .iter()
            .map(|s| path_from_bytes(s.path.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_folder_bookmark_roundtrip_and_corrupt_records_are_safe() {
        objc2::rc::autoreleasepool(|_| {
            let root = tempfile::tempdir().unwrap();
            let folder = root.path().join("한글😀 폴더");
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("file"), b"keep").unwrap();
            let (saved, scope) = remember(super::super::file_url(&folder)).unwrap();
            drop(scope);
            let (_, restored) = restore(&saved).unwrap();
            assert_eq!(
                std::fs::read(restored.path().join("file")).unwrap(),
                b"keep"
            );
            let mut invalid = saved.clone();
            invalid.bookmark = vec![1, 2, 3];
            assert!(restore(&invalid).is_err());
            invalid.path = b"/bad\0path".to_vec();
            assert!(restore(&invalid).is_err());
            let mut memory = Memory::load(vec![saved]);
            let (saved, scope) = remember(super::super::file_url(&folder)).unwrap();
            memory.insert(saved, scope).unwrap();
            assert_eq!(memory.paths().len(), 1);
            assert_eq!(memory.scopes.len(), 1);
        });
    }
}
