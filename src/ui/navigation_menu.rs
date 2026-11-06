use super::*;
use crate::core::shortcuts::PinGroup;

pub(super) const FAVORITES: &str = "즐겨찾기";
pub(super) const LOCATIONS: &str = "고정한 위치";
pub(super) const RECENT: &str = "최근 폴더";

impl AppDelegate {
    pub(super) fn navigation_window(&self) -> Option<Retained<Controller>> {
        self.reveal_window();
        self.active_controller()
            .filter(|controller| controller.views().window.attachedSheet().is_none())
    }
    pub(super) unsafe fn folder_submenu(&self, title: &str, live: bool) -> Retained<NSMenuItem> {
        let item = NSMenuItem::new(self.mtm());
        let label = NSString::from_str(title);
        item.setTitle(&label);
        let menu = NSMenu::new(self.mtm());
        item.setSubmenu(Some(&menu));
        menu.setTitle(&label);
        menu.setAutoenablesItems(false);
        if live {
            menu.setDelegate(Some(ProtocolObject::from_ref(self)));
        } else {
            self.fill_folder_menu(&menu);
        }
        item
    }
    pub(super) fn fill_folder_menu(&self, menu: &NSMenu) {
        let title = menu.title().to_string();
        let group = match title.as_str() {
            FAVORITES => Some(PinGroup::Favorites),
            LOCATIONS => Some(PinGroup::Locations),
            RECENT => None,
            _ => return,
        };
        let entries: Vec<(String, PathBuf)> = if let Some(group) = group {
            let prefs = self.ivars().runtime.preferences.lock().unwrap();
            prefs
                .shortcuts(group, &home_dir())
                .into_iter()
                .filter(|item| prefs.sidebar.visible(&item.key))
                .map(|item| (item.label, item.path))
                .collect()
        } else {
            self.ivars()
                .runtime
                .recent_folders
                .lock()
                .unwrap()
                .iter()
                .filter(|item| !item.0.contains(&0) && item.path().is_absolute())
                .map(|item| {
                    let path = item.path();
                    (Controller::display_location(&path), path)
                })
                .collect()
        };
        unsafe {
            menu.removeAllItems();
            let mut names = HashMap::new();
            for (name, _) in &entries {
                *names.entry(name).or_insert(0) += 1;
            }
            for (name, path) in &entries {
                let title = if names[name] > 1 {
                    Controller::display_location(path)
                } else {
                    name.clone()
                };
                menu.addItem(&self.folder_item(&title, path));
            }
            if entries.is_empty() {
                let row = NSMenuItem::new(self.mtm());
                row.setTitle(ns_string!("표시할 폴더가 없습니다"));
                row.setEnabled(false);
                menu.addItem(&row);
            }
            if let Some(group) = group {
                menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
                let add = widgets::item(
                    "폴더 추가…",
                    sel!(addNavigationFolder:),
                    Some(self),
                    "",
                    self.mtm(),
                );
                add.setTag(group.section() as isize);
                menu.addItem(&add);
            }
        }
    }
    unsafe fn folder_item(&self, title: &str, path: &std::path::Path) -> Retained<NSMenuItem> {
        let row = widgets::item(
            title,
            sel!(openNavigationFolder:),
            Some(self),
            "",
            self.mtm(),
        );
        row.setRepresentedObject(Some(&crate::platform::file_url(path)));
        row.setToolTip(Some(&NSString::from_str(&path.to_string_lossy())));
        row
    }
    pub(super) unsafe fn dock_menu(&self) -> Retained<NSMenu> {
        let menu = NSMenu::new(self.mtm());
        for (title, action) in [
            ("새 창", sel!(newWindow:)),
            ("폴더로 이동…", sel!(showNavigationGo:)),
            ("서버에 연결…", sel!(showNavigationServer:)),
        ] {
            menu.addItem(&widgets::item(title, action, Some(self), "", self.mtm()));
        }
        menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
        let quick = {
            let prefs = self.ivars().runtime.preferences.lock().unwrap();
            prefs
                .shortcuts(PinGroup::Favorites, &home_dir())
                .into_iter()
                .filter(|item| {
                    item.key.starts_with("favorite:") && prefs.sidebar.visible(&item.key)
                })
                .take(3)
                .collect::<Vec<_>>()
        };
        if !quick.is_empty() {
            for shortcut in &quick {
                let name = if quick
                    .iter()
                    .any(|other| other.key != shortcut.key && other.label == shortcut.label)
                {
                    Controller::display_location(&shortcut.path)
                } else {
                    shortcut.label.clone()
                };
                menu.addItem(&self.folder_item(&name, &shortcut.path));
            }
            menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
        }
        for title in [FAVORITES, LOCATIONS, RECENT] {
            menu.addItem(&self.folder_submenu(title, false));
        }
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::ClassType;

    #[test]
    fn appkit_decodes_the_declared_dock_resource_from_the_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle_path = tmp.path().join("Test Explorer.app");
        let contents = bundle_path.join("Contents");
        let resources = contents.join("Resources");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            include_bytes!("../../resources/Info.plist"),
        )
        .unwrap();
        std::fs::write(
            resources.join("ExplorerFolder.icns"),
            include_bytes!("../../resources/AppIcon.icns"),
        )
        .unwrap();
        let bundle =
            NSBundle::bundleWithPath(&NSString::from_str(bundle_path.to_str().unwrap())).unwrap();
        let image = bundled_dock_icon(&bundle)
            .expect("the same bundle lookup used at launch must decode the ICNS");
        assert!(
            image
                .representations()
                .iter()
                .any(|rep| rep.pixelsWide() >= 512 && rep.pixelsHigh() >= 512)
        );
    }

    // Menu actions are Objective-C selectors: Rust does not type-check a
    // selector string against the object AppKit will send it to.
    #[test]
    fn dock_and_sidebar_targets_register_their_native_actions() {
        for action in [
            sel!(applicationDockMenu:),
            sel!(newWindow:),
            sel!(showNavigationGo:),
            sel!(showNavigationServer:),
            sel!(openNavigationFolder:),
            sel!(addNavigationFolder:),
        ] {
            assert!(
                AppDelegate::class().responds_to(action),
                "missing Dock selector {action}"
            );
        }
        for action in [
            sel!(pickFavoriteFolders:),
            sel!(pickLocationFolders:),
            sel!(renameSidebarShortcut:),
            sel!(retargetSidebarShortcut:),
            sel!(unpinSidebarShortcut:),
            sel!(connectToServer:),
            sel!(cancelServerConnection:),
            sel!(disconnectSidebarServer:),
        ] {
            assert!(
                Controller::class().responds_to(action),
                "missing sidebar selector {action}"
            );
        }
    }
}
