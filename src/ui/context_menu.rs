use super::*;

impl Controller {
    pub(super) fn prepare_context_menu(&self, menu: &NSMenu) {
        unsafe {
            menu.removeAllItems();
            let title = menu.title().to_string();
            if title == "sidebar" || title == "sidebar-background" {
                self.sidebar_menu(menu, title == "sidebar-background");
                return;
            }
            let background = title == "background";
            if background {
                let view = self.ivars().state.borrow().prefs.view;
                match view {
                    ViewMode::Details => self.views().table.deselectAll(None),
                    ViewMode::Icons => self
                        .views()
                        .collection
                        .setSelectionIndexPaths(&NSSet::new()),
                }
                self.focus_list();
            }
            // NSCollectionView does not notify its delegate when menuForEvent
            // changes selection programmatically. Read it before using the
            // model, so commands apply to the item that was right-clicked.
            self.selection_changed();
            let windows = self.ivars().state.borrow().prefs.keyboard == KeyboardProfile::Windows;
            let primary = if windows {
                NSEventModifierFlags::Control
            } else {
                NSEventModifierFlags::Command
            };
            let paths = self.selected_paths();
            let folders = self.with_tab(|t| {
                !t.selected.is_empty()
                    && t.entries
                        .iter()
                        .filter(|e| t.selected.contains(&e.id))
                        .all(|e| e.kind == Kind::Folder)
            });
            let add = |title: &str, action, icon: &str, key: &str, modifiers| {
                let row = item(title, action, Some(self), key, self.mtm());
                row.setKeyEquivalentModifierMask(modifiers);
                let image = symbol(icon);
                if let Some(image) = &image {
                    image.setSize(NSSize::new(16., 16.));
                }
                row.setImage(image.as_deref());
                menu.addItem(&row);
            };
            let none = NSEventModifierFlags::empty();
            let sep = || menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
            if !background && !paths.is_empty() {
                add(
                    "열기",
                    sel!(openSelected:),
                    "arrow.up.forward.app",
                    if windows { "\r" } else { "\u{f701}" },
                    if windows { none } else { primary },
                );
                if paths.iter().all(|p| crate::core::archive::is_archive(p)) {
                    add(
                        "압축 풀기 · 결과 폴더 열기",
                        sel!(extractArchive:),
                        "archivebox",
                        "",
                        none,
                    );
                }
                if self.can_open_with() {
                    add(
                        "연결 프로그램 선택…",
                        sel!(openWith:),
                        "arrow.up.forward.app",
                        "",
                        none,
                    );
                }
                let compress = NSMenuItem::new(self.mtm());
                compress.setTitle(ns_string!("압축하기"));
                compress.setImage(symbol("archivebox").as_deref());
                let formats = NSMenu::new(self.mtm());
                for (name, action) in [
                    ("ZIP 파일…", sel!(compressZip:)),
                    ("7z 파일…", sel!(compress7z:)),
                ] {
                    formats.addItem(&item(name, action, Some(self), "", self.mtm()));
                }
                compress.setSubmenu(Some(&formats));
                menu.addItem(&compress);
                let send = NSMenuItem::new(self.mtm());
                send.setTitle(ns_string!("보내기"));
                send.setImage(symbol("square.and.arrow.up").as_deref());
                let destinations = NSMenu::new(self.mtm());
                let bluetooth = item(
                    "Bluetooth 파일 교환…",
                    sel!(sendBluetooth:),
                    Some(self),
                    "",
                    self.mtm(),
                );
                bluetooth.setToolTip(Some(ns_string!("파일 하나를 기기 선택 창으로 전달합니다. 폴더·여러 파일은 먼저 압축하세요. iPhone·iPad는 AirDrop을 이용하세요.")));
                destinations.addItem(&bluetooth);
                destinations.addItem(&item(
                    "AirDrop…",
                    sel!(sendAirDrop:),
                    Some(self),
                    "",
                    self.mtm(),
                ));
                send.setSubmenu(Some(&destinations));
                menu.addItem(&send);
                if folders {
                    add(
                        "새 탭에서 열기",
                        sel!(openInNewTab:),
                        "plus.square.on.square",
                        "",
                        none,
                    );
                    add(
                        if windows {
                            "즐겨찾기에 고정"
                        } else {
                            "즐겨찾기에 추가"
                        },
                        sel!(addSelectedFavorites:),
                        "pin",
                        "",
                        none,
                    );
                    add("위치에 고정", sel!(addSelectedLocations:), "pin", "", none);
                }
                if !windows {
                    add("빠른 보기", sel!(quickLook:), "eye", " ", none);
                }
                sep();
                if windows {
                    add("잘라내기", sel!(cutFiles:), "scissors", "x", primary);
                }
                add("복사", sel!(copyFiles:), "doc.on.doc", "c", primary);
                add(
                    "복제 · 같은 위치에 복사",
                    sel!(duplicateFiles:),
                    "plus.square.on.square",
                    "d",
                    NSEventModifierFlags::Command,
                );
                add(
                    "붙여넣기",
                    sel!(pasteFiles:),
                    "doc.on.clipboard",
                    "v",
                    primary,
                );
                add(
                    if windows {
                        "경로로 복사"
                    } else {
                        "경로 이름 복사"
                    },
                    sel!(copyPath:),
                    "link",
                    "",
                    none,
                );
                sep();
                add(
                    if windows {
                        "이름 바꾸기"
                    } else {
                        "이름 변경"
                    },
                    sel!(renameFile:),
                    "pencil",
                    if windows { "\u{f705}" } else { "\r" },
                    none,
                );
                if paths.len() > 1 {
                    add(
                        "이름 일괄 변경…",
                        sel!(batchRename:),
                        "pencil.line",
                        "",
                        none,
                    );
                }
                add(
                    if windows {
                        "삭제 · 휴지통으로 이동"
                    } else {
                        "휴지통으로 이동"
                    },
                    sel!(trashFiles:),
                    "trash",
                    if windows { "\u{f728}" } else { "\u{8}" },
                    if windows { none } else { primary },
                );
                sep();
                if windows {
                    add("미리보기", sel!(quickLook:), "eye", " ", none);
                }
                add(
                    "Finder에서 보기",
                    sel!(revealInFinder:),
                    "macwindow",
                    "",
                    none,
                );
                add("선택 항목 크기 계산", sel!(calculateSize:), "sum", "", none);
                add(
                    if windows {
                        "속성"
                    } else {
                        "정보 가져오기"
                    },
                    sel!(showProperties:),
                    "info.circle",
                    if windows { "" } else { "i" },
                    primary,
                );
            } else {
                menu.addItem(&creation_submenu(Some(self), primary, self.mtm()));
                sep();
                let view = NSMenuItem::new(self.mtm());
                view.setTitle(ns_string!("보기"));
                let choices = NSMenu::new(self.mtm());
                for (name, action) in [
                    ("자세히", sel!(detailsView:)),
                    ("작은 아이콘", sel!(smallIcons:)),
                    ("중간 아이콘", sel!(mediumIcons:)),
                    ("큰 아이콘", sel!(largeIcons:)),
                ] {
                    choices.addItem(&item(name, action, Some(self), "", self.mtm()));
                }
                view.setSubmenu(Some(&choices));
                menu.addItem(&view);
                let sort = NSMenuItem::new(self.mtm());
                sort.setTitle(ns_string!("정렬 기준"));
                let criteria = NSMenu::new(self.mtm());
                for (title, action) in [
                    ("이름", sel!(sortByName:)),
                    ("수정한 날짜", sel!(sortByDate:)),
                    ("크기", sel!(sortBySize:)),
                ] {
                    criteria.addItem(&item(title, action, Some(self), "", self.mtm()));
                }
                sort.setSubmenu(Some(&criteria));
                menu.addItem(&sort);
                add(
                    "새로 고침",
                    sel!(refresh:),
                    "arrow.clockwise",
                    if windows { "\u{f708}" } else { "r" },
                    if windows { none } else { primary },
                );
                sep();
                add(
                    "붙여넣기",
                    sel!(pasteFiles:),
                    "doc.on.clipboard",
                    "v",
                    primary,
                );
                add(
                    "실행 취소",
                    sel!(undoOperation:),
                    "arrow.uturn.backward",
                    "z",
                    primary,
                );
                sep();
                add("현재 폴더 경로 복사", sel!(copyAddress:), "link", "", none);
                add(
                    "폴더로 이동…",
                    sel!(goToFolder:),
                    "folder",
                    "g",
                    NSEventModifierFlags::Command | NSEventModifierFlags::Shift,
                );
                add(
                    "현재 폴더를 즐겨찾기에 고정",
                    sel!(addFavorite:),
                    "pin",
                    "",
                    none,
                );
                add(
                    "현재 폴더를 위치에 고정",
                    sel!(pinCurrentLocation:),
                    "pin",
                    "",
                    none,
                );
                sep();
                add("환경 설정…", sel!(settings:), "gearshape", ",", primary);
            }
            if windows {
                arrange_windows_menu(menu, background, self.mtm());
            }
        }
    }
    pub(super) fn show_properties(&self) {
        let entries: Vec<Entry> = self.with_tab(|t| {
            t.entries
                .iter()
                .filter(|e| t.selected.contains(&e.id))
                .map(|e| t.metadata.get(&e.id).unwrap_or(e).clone())
                .collect()
        });
        if entries.is_empty() {
            return;
        }
        let title = if self.ivars().state.borrow().prefs.keyboard == KeyboardProfile::Windows {
            "속성"
        } else {
            "정보 가져오기"
        };
        let detail = if entries.len() == 1 {
            let e = &entries[0];
            format!(
                "이름: {}\n종류: {}\n위치: {}\n크기: {}\n수정: {}",
                e.name,
                e.kind_label(),
                e.path.to_string_lossy(),
                e.size
                    .map(format_size)
                    .unwrap_or_else(|| "폴더 크기는 ‘선택 항목 크기 계산’으로 확인".into()),
                e.modified
                    .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                    .map(|d| d
                        .with_timezone(&chrono::Local)
                        .format("%Y.%m.%d %H:%M:%S")
                        .to_string())
                    .unwrap_or_else(|| "확인 중".into())
            )
        } else {
            format!(
                "{}개 항목 선택\n{}",
                entries.len(),
                entries
                    .iter()
                    .take(12)
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        self.message(title, &detail);
    }
}

// Windows 11's primary file commands first, followed by open/location/archive
// commands. Keep labels visible and Properties at the bottom.
fn arrange_windows_menu(menu: &NSMenu, background: bool, mtm: MainThreadMarker) {
    let mut entries: Vec<_> = menu
        .itemArray()
        .iter()
        .filter(|i| !i.isSeparatorItem())
        .collect();
    let extras = NSMenu::new(mtm);
    if !background {
        let extra_actions = [
            sel!(duplicateFiles:),
            sel!(revealInFinder:),
            sel!(calculateSize:),
        ];
        let moved: Vec<_> = entries
            .iter()
            .filter(|i| i.action().is_some_and(|a| extra_actions.contains(&a)))
            .cloned()
            .collect();
        entries.retain(|i| !i.action().is_some_and(|a| extra_actions.contains(&a)));
        for entry in moved {
            menu.removeItem(&entry);
            extras.addItem(&entry);
        }
        if extras.numberOfItems() > 0 {
            let entry = NSMenuItem::new(mtm);
            entry.setTitle(ns_string!("Mac 기능"));
            entry.setSubmenu(Some(&extras));
            entries.push(entry);
        }
    }
    let rank = |entry: &NSMenuItem| -> u16 {
        if background {
            if entry.submenu().is_some() {
                return match entry.title().to_string().as_str() {
                    "보기" => 0,
                    "정렬 기준" => 1,
                    "새로 만들기" => 200,
                    _ => 300,
                };
            }
            for (action, rank) in [
                (sel!(refresh:), 2),
                (sel!(pasteFiles:), 100),
                (sel!(undoOperation:), 101),
                (sel!(copyAddress:), 300),
                (sel!(goToFolder:), 301),
                (sel!(addFavorite:), 302),
                (sel!(pinCurrentLocation:), 303),
                (sel!(settings:), 400),
            ] {
                if entry.action() == Some(action) {
                    return rank;
                }
            }
        } else {
            if entry.submenu().is_some() {
                return match entry.title().to_string().as_str() {
                    "보내기" => 5,
                    "압축하기" => 300,
                    "Mac 기능" => 400,
                    _ => 450,
                };
            }
            for (action, rank) in [
                (sel!(cutFiles:), 0),
                (sel!(copyFiles:), 1),
                (sel!(pasteFiles:), 2),
                (sel!(renameFile:), 3),
                (sel!(batchRename:), 4),
                (sel!(trashFiles:), 6),
                (sel!(openSelected:), 100),
                (sel!(openInNewTab:), 101),
                (sel!(openWith:), 102),
                (sel!(quickLook:), 103),
                (sel!(addSelectedFavorites:), 200),
                (sel!(addSelectedLocations:), 201),
                (sel!(copyPath:), 202),
                (sel!(extractArchive:), 301),
                (sel!(showProperties:), 500),
            ] {
                if entry.action() == Some(action) {
                    return rank;
                }
            }
        }
        450
    };
    entries.sort_by_key(|entry| rank(entry));
    menu.removeAllItems();
    let mut previous = None;
    for entry in entries {
        let group = rank(&entry) / 100;
        if previous.is_some_and(|old| old != group) {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        menu.addItem(&entry);
        previous = Some(group);
    }
}
