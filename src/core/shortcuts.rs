use super::model::{Preferences, path_bytes, path_from_bytes, path_id};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinGroup {
    Favorites,
    Locations,
}

impl PinGroup {
    pub fn section(self) -> u64 {
        match self {
            Self::Favorites => 1,
            Self::Locations => 3,
        }
    }
    pub fn from_section(section: u64) -> Option<Self> {
        match section {
            1 => Some(Self::Favorites),
            3 => Some(Self::Locations),
            _ => None,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Favorites => "즐겨찾기",
            Self::Locations => "위치",
        }
    }
    fn key(self, path: &Path) -> String {
        let prefix = match self {
            Self::Favorites => "favorite",
            Self::Locations => "pinned-location",
        };
        format!("{prefix}:{:x}", path_id(path))
    }
    fn collapsed_key(self) -> &'static str {
        match self {
            Self::Favorites => "favorites",
            Self::Locations => "locations",
        }
    }
}

pub struct Shortcut {
    pub key: String,
    pub label: String,
    pub path: PathBuf,
    pub symbol: &'static str,
}

impl Preferences {
    pub fn shortcuts(&self, group: PinGroup, home: &Path) -> Vec<Shortcut> {
        let mut result = Vec::new();
        if group == PinGroup::Favorites {
            for (key, label, path, symbol) in [
                ("home", "홈", home.to_owned(), "house"),
                (
                    "desktop",
                    "데스크탑",
                    home.join("Desktop"),
                    "desktopcomputer",
                ),
                (
                    "downloads",
                    "다운로드",
                    home.join("Downloads"),
                    "arrow.down.circle",
                ),
                ("documents", "문서", home.join("Documents"), "doc"),
                ("pictures", "사진", home.join("Pictures"), "photo"),
                (
                    "applications",
                    "응용 프로그램",
                    "/Applications".into(),
                    "square.grid.2x2",
                ),
            ] {
                result.push(Shortcut {
                    key: key.into(),
                    label: label.into(),
                    path,
                    symbol,
                });
            }
        }
        let paths = match group {
            PinGroup::Favorites => &self.favorites,
            PinGroup::Locations => &self.pinned_locations,
        };
        for bytes in paths {
            let path = path_from_bytes(bytes.clone());
            if !path.is_absolute()
                || bytes.contains(&0)
                || result.iter().any(|item| item.path == path)
            {
                continue;
            }
            let key = group.key(&path);
            let label = self.shortcut_labels.get(&key).cloned().unwrap_or_else(|| {
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            });
            result.push(Shortcut {
                key,
                label,
                path,
                symbol: "folder",
            });
        }
        result.sort_by_key(|item| {
            self.sidebar
                .order
                .iter()
                .position(|key| key == &item.key)
                .unwrap_or(usize::MAX)
        });
        result
    }

    pub fn pin_folders(&mut self, group: PinGroup, paths: &[PathBuf], home: &Path) {
        let known = self.shortcuts(group, home);
        for path in paths {
            let key = if let Some(item) = known.iter().find(|item| item.path == *path) {
                item.key.clone()
            } else {
                let saved = match group {
                    PinGroup::Favorites => &mut self.favorites,
                    PinGroup::Locations => &mut self.pinned_locations,
                };
                let bytes = path_bytes(path);
                if !saved.contains(&bytes) {
                    saved.push(bytes);
                }
                group.key(path)
            };
            self.sidebar.hidden.retain(|hidden| hidden != &key);
        }
        self.sidebar
            .collapsed
            .retain(|key| key != group.collapsed_key());
    }

    pub fn retarget_folder(
        &mut self,
        group: PinGroup,
        old: &Path,
        new: &Path,
        home: &Path,
    ) -> Result<(), String> {
        if old == new {
            return Ok(());
        }
        if self
            .shortcuts(group, home)
            .iter()
            .any(|item| item.path == new)
        {
            return Err("이미 이 그룹에 있는 폴더입니다.".into());
        }
        let saved = match group {
            PinGroup::Favorites => &mut self.favorites,
            PinGroup::Locations => &mut self.pinned_locations,
        };
        let old_bytes = path_bytes(old);
        let entry = saved
            .iter_mut()
            .find(|path| **path == old_bytes)
            .ok_or("바로가기가 이미 제거되었습니다.")?;
        *entry = path_bytes(new);
        let old_key = group.key(old);
        let new_key = group.key(new);
        for key in self
            .sidebar
            .order
            .iter_mut()
            .chain(&mut self.sidebar.hidden)
        {
            if *key == old_key {
                *key = new_key.clone();
            }
        }
        if let Some(label) = self.shortcut_labels.remove(&old_key) {
            self.shortcut_labels.insert(new_key, label);
        }
        Ok(())
    }
    pub fn unpin_folder(&mut self, group: PinGroup, path: &Path) {
        let saved = match group {
            PinGroup::Favorites => &mut self.favorites,
            PinGroup::Locations => &mut self.pinned_locations,
        };
        let bytes = path_bytes(path);
        saved.retain(|entry| entry != &bytes);
        let key = group.key(path);
        self.sidebar.order.retain(|entry| entry != &key);
        self.sidebar.hidden.retain(|entry| entry != &key);
        self.shortcut_labels.remove(&key);
    }
}
