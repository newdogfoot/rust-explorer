use super::*;
use crate::platform::{Location, LocationKind};

#[derive(Clone, Copy)]
pub enum SidebarAction {
    AirDrop,
    ICloudUnavailable,
}
pub(super) struct Spec {
    key: String,
    label: String,
    path: Option<PathBuf>,
    symbol: &'static str,
    group: u64,
    action: Option<SidebarAction>,
}
fn update_mounted_tabs(tabs: &mut [Tab], previous: &[Location], current: &[Location]) -> Vec<u64> {
    if previous.is_empty() {
        return vec![];
    }
    fn mounts(list: &[Location]) -> HashSet<&Path> {
        list.iter()
            .filter(|location| {
                matches!(
                    location.kind,
                    LocationKind::Network | LocationKind::External
                )
            })
            .map(|location| location.path.as_path())
            .collect()
    }
    let old = mounts(previous);
    let new = mounts(current);
    let mut lost = Vec::new();
    for tab in tabs {
        if old.difference(&new).any(|root| tab.path.starts_with(root)) {
            tab.volume_disconnected();
            lost.push(tab.id);
        } else if new.difference(&old).any(|root| tab.path.starts_with(root)) {
            tab.watch_due = Some(Instant::now());
        }
    }
    lost
}

impl Controller {
    pub(super) fn activate_selected_sidebar(&self, actions: bool) {
        if self.ivars().state.borrow().rebuilding_sidebar {
            return;
        }
        let id = unsafe {
            let outline = &self.views().sidebar;
            if outline.selectedRow() < 0 {
                return;
            }
            outline
                .itemAtRow(outline.selectedRow())
                .and_then(|n| node_id(Some(&n)))
        };
        let target = {
            let s = self.ivars().state.borrow();
            id.filter(|id| {
                s.nodes.get(id).is_some_and(|n| {
                    !n.section
                        && if actions {
                            n.action.is_some()
                        } else {
                            n.path.is_some()
                        }
                })
            })
        };
        if let Some(id) = target {
            self.sidebar_activate(id);
        }
    }
    fn sidebar_specs(&self) -> Vec<Spec> {
        let state = self.ivars().state.borrow();
        let home = super::super::home_dir();
        let shortcuts = |group: PinGroup| {
            state
                .prefs
                .shortcuts(group, &home)
                .into_iter()
                .map(move |item| Spec {
                    key: item.key,
                    label: item.label,
                    path: Some(item.path),
                    symbol: item.symbol,
                    group: group.section(),
                    action: None,
                })
        };
        let mut specs: Vec<_> = shortcuts(PinGroup::Favorites).collect();
        if !state.locations.iter().any(|l| l.path == Path::new("/")) {
            specs.push(Spec {
                key: format!("location:{:x}", path_id(Path::new("/"))),
                label: "Mac 저장장치".into(),
                path: Some("/".into()),
                symbol: "internaldrive",
                group: 3,
                action: None,
            });
        }
        let icloud = state
            .locations
            .iter()
            .find(|l| l.name == "iCloud Drive" && l.kind == LocationKind::Cloud);
        specs.push(Spec {
            key: "icloud".into(),
            label: "iCloud Drive".into(),
            path: icloud.map(|l| l.path.clone()),
            symbol: "icloud",
            group: 2,
            action: icloud.is_none().then_some(SidebarAction::ICloudUnavailable),
        });
        specs.extend(shortcuts(PinGroup::Locations));
        for l in &state.locations {
            if l.kind != LocationKind::Cloud
                && specs
                    .iter()
                    .any(|s| s.group == 3 && s.path.as_ref() == Some(&l.path))
            {
                continue;
            }
            if l.kind == LocationKind::Cloud && l.name == "iCloud Drive" {
                continue;
            }
            let (group, symbol) = match l.kind {
                LocationKind::Cloud => (2, "cloud"),
                LocationKind::Internal => (3, "internaldrive"),
                LocationKind::External => (3, "externaldrive"),
                LocationKind::Network => (3, "network"),
            };
            specs.push(Spec {
                key: format!("location:{:x}", path_id(&l.path)),
                label: l.name.clone(),
                path: Some(l.path.clone()),
                symbol,
                group,
                action: None,
            });
        }
        specs.push(Spec {
            key: "airdrop".into(),
            label: "AirDrop".into(),
            path: None,
            symbol: "airdrop",
            group: 3,
            action: Some(SidebarAction::AirDrop),
        });
        specs
    }
    pub(crate) fn rebuild_sidebar(&self, locations: Vec<Location>) {
        let (lost, active_lost) = {
            let mut state = self.ivars().state.borrow_mut();
            let previous = std::mem::take(&mut state.locations);
            let lost = update_mounted_tabs(&mut state.tabs, &previous, &locations);
            state.locations = locations;
            let active = state.tabs[state.active].id;
            let active_lost = lost.contains(&active);
            (lost, active_lost)
        };
        for tab in lost {
            self.ivars().runtime.unwatch(self.id(), Some(tab));
        }
        if active_lost {
            self.cancel_inline_rename();
            self.cancel_thumbnails();
            self.reload();
        }
        self.refresh_sidebar();
    }
    pub(super) fn refresh_sidebar(&self) {
        let prefs = self.ivars().state.borrow().prefs.sidebar.clone();
        let mut specs = self.sidebar_specs();
        specs.retain(|s| prefs.visible(&s.key));
        specs.sort_by_key(|s| {
            prefs
                .order
                .iter()
                .position(|k| k == &s.key)
                .unwrap_or(usize::MAX)
        });
        let mut nodes = HashMap::new();
        for (id, key, label) in [
            (1, "favorites", "즐겨찾기"),
            (2, "cloud", "클라우드"),
            (3, "locations", "위치"),
        ] {
            nodes.insert(
                id,
                SidebarNode {
                    object: OutlineNode::new(self.mtm(), id),
                    label: label.into(),
                    path: None,
                    children: vec![],
                    loaded: true,
                    loading: false,
                    section: true,
                    key: key.into(),
                    symbol: String::new(),
                    action: None,
                    group: id,
                },
            );
        }
        for spec in specs {
            let id = path_id(Path::new(&format!("sidebar:{}", spec.key)));
            nodes.get_mut(&spec.group).unwrap().children.push(id);
            let loaded = spec.path.is_none();
            nodes.insert(
                id,
                SidebarNode {
                    object: OutlineNode::new(self.mtm(), id),
                    label: spec.label,
                    path: spec.path,
                    children: vec![],
                    loaded,
                    loading: false,
                    section: false,
                    key: spec.key,
                    symbol: spec.symbol.into(),
                    action: spec.action,
                    group: spec.group,
                },
            );
        }
        {
            let mut state = self.ivars().state.borrow_mut();
            state.nodes = nodes;
            state.roots = vec![1, 2, 3];
            state.rebuilding_sidebar = true;
        }
        unsafe {
            self.views().sidebar.reloadData();
            for id in [1, 2, 3] {
                let (node, key) = {
                    let s = self.ivars().state.borrow();
                    (s.nodes[&id].object.clone(), s.nodes[&id].key.clone())
                };
                if !prefs.collapsed.contains(&key) {
                    self.views().sidebar.expandItem(Some(&node));
                }
            }
        }
        self.ivars().state.borrow_mut().rebuilding_sidebar = false;
    }
    pub(super) fn sidebar_activate(&self, id: u64) {
        let (path, action) = {
            let s = self.ivars().state.borrow();
            let Some(n) = s.nodes.get(&id) else { return };
            (n.path.clone(), n.action)
        };
        if let Some(path) = path {
            if path != self.current_path() {
                self.navigate(path, true);
            }
        } else if let Some(action) = action {
            match action {
            SidebarAction::AirDrop=>unsafe {
                let url=platform::file_url(Path::new("/System/Library/CoreServices/Finder.app/Contents/Applications/AirDrop.app"));
                if !NSWorkspace::sharedWorkspace().openURL(&url){self.message("AirDrop을 열 수 없습니다","Finder의 이동 → AirDrop에서 확인해 주세요.");}
            },
            SidebarAction::ICloudUnavailable=>self.message("iCloud Drive가 연결되어 있지 않습니다","시스템 설정 → Apple 계정 → iCloud에서 iCloud Drive를 켜면 이곳에서 파일을 탐색할 수 있습니다."),
        }
        }
    }
    pub(super) fn sidebar_expansion_changed(&self) {
        if self.ivars().state.borrow().rebuilding_sidebar {
            return;
        }
        let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
        prefs.sidebar.collapsed.clear();
        unsafe {
            for id in [1, 2, 3] {
                let s = self.ivars().state.borrow();
                if let Some(n) = s.nodes.get(&id)
                    && !self.views().sidebar.isItemExpanded(Some(&n.object))
                {
                    prefs.sidebar.collapsed.push(n.key.clone());
                }
            }
        }
        // Do not rebuild the outline while AppKit is delivering its expansion notification.
        self.ivars().state.borrow_mut().prefs.sidebar.collapsed = prefs.sidebar.collapsed.clone();
        self.ivars().runtime.save_preferences(prefs);
    }
    pub(super) fn sidebar_context_path(&self) -> Option<PathBuf> {
        let s = self.ivars().state.borrow();
        s.sidebar_context
            .and_then(|id| s.nodes.get(&id))
            .and_then(|n| n.path.clone())
    }
    pub(super) fn toggle_sidebar_item(&self, key: &str) {
        if key.is_empty() {
            return;
        }
        let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
        prefs.sidebar.toggle(key);
        self.commit_preferences(prefs);
    }
    pub(super) fn hide_sidebar_item(&self) {
        let key = {
            let s = self.ivars().state.borrow();
            s.sidebar_context
                .and_then(|id| s.nodes.get(&id))
                .map(|n| n.key.clone())
        };
        if let Some(key) = key {
            self.toggle_sidebar_item(&key);
        }
    }
    pub(super) fn move_sidebar_item(&self, down: bool) {
        let (key, group) = {
            let s = self.ivars().state.borrow();
            let Some(n) = s.sidebar_context.and_then(|id| s.nodes.get(&id)) else {
                return;
            };
            (n.key.clone(), n.group)
        };
        let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
        let mut specs = self.sidebar_specs();
        specs.sort_by_key(|s| {
            prefs
                .sidebar
                .order
                .iter()
                .position(|k| k == &s.key)
                .unwrap_or(usize::MAX)
        });
        let mut keys: Vec<_> = specs
            .into_iter()
            .filter(|s| s.group == group && prefs.sidebar.visible(&s.key))
            .map(|s| s.key)
            .collect();
        let Some(index) = keys.iter().position(|k| k == &key) else {
            return;
        };
        let target = if down {
            (index + 1).min(keys.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        keys.swap(index, target);
        prefs.sidebar.order.retain(|k| !keys.contains(k));
        prefs.sidebar.order.extend(keys);
        self.commit_preferences(prefs);
    }
    pub(super) fn pin_selected(&self, group: PinGroup) {
        let paths = self.with_tab(|t| {
            t.entries
                .iter()
                .filter(|entry| t.selected.contains(&entry.id) && entry.kind == Kind::Folder)
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>()
        });
        self.pin_paths(group, &paths);
    }
    pub(super) unsafe fn pin_drag_operation(
        &self,
        info: &ProtocolObject<dyn NSDraggingInfo>,
    ) -> NSDragOperation {
        if info
            .draggingPasteboard()
            .availableTypeFromArray(&NSArray::from_slice(&[NSPasteboardTypeFileURL]))
            .is_none()
        {
            return NSDragOperation::None;
        }
        let allowed = info.draggingSourceOperationMask();
        if allowed.contains(NSDragOperation::Link) {
            NSDragOperation::Link
        } else if allowed.contains(NSDragOperation::Copy) {
            NSDragOperation::Copy
        } else {
            NSDragOperation::None
        }
    }
    pub(super) unsafe fn accept_pin_drop(
        &self,
        info: &ProtocolObject<dyn NSDraggingInfo>,
        group: PinGroup,
    ) -> bool {
        if self.pin_drag_operation(info) == NSDragOperation::None {
            return false;
        }
        let paths = clipboard_paths(&info.draggingPasteboard());
        if paths.is_empty() {
            return false;
        }
        let window = self.id();
        let sender = self.ivars().runtime.sender.clone();
        let submitted = self.ivars().runtime.io.submit(move || {
            let result = if paths.iter().all(|p| p.is_dir() && !is_package(p)) {
                Ok(paths)
            } else {
                Err("바로가기를 추가하려면 폴더만 선택하세요.".into())
            };
            let _ = sender.send(Event::PinFolders {
                window,
                group,
                result,
            });
        });
        if let Err(error) = submitted {
            self.notice(error, None);
            return false;
        }
        true
    }
    pub(super) fn pin_paths(&self, group: PinGroup, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
        prefs.pin_folders(group, paths, &super::super::home_dir());
        self.commit_preferences(prefs);
        self.notice(
            &format!("{}에 폴더 {}개 고정", group.title(), paths.len()),
            None,
        );
    }
    pub(crate) fn pick_pin_folders(&self, group: PinGroup) {
        self.choose_pin_destination(group, None);
    }
    pub(super) fn retarget_context_shortcut(&self) {
        if let Some((group, _, path, _)) = self.context_shortcut() {
            self.choose_pin_destination(group, Some(path));
        }
    }
    fn choose_pin_destination(&self, group: PinGroup, replace: Option<PathBuf>) {
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        unsafe {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setTitle(Some(&NSString::from_str(&format!(
                "{}에 폴더 추가",
                group.title()
            ))));
            if replace.is_some() {
                panel.setTitle(Some(ns_string!("바로가기 대상 폴더 변경")));
                panel.setPrompt(Some(ns_string!("이 폴더 사용")));
            } else {
                panel.setPrompt(Some(ns_string!("추가")));
            }
            panel.setCanChooseFiles(false);
            panel.setCanChooseDirectories(true);
            panel.setTreatsFilePackagesAsDirectories(false);
            panel.setAllowsMultipleSelection(replace.is_none());
            let start = if replace.is_some() || self.with_tab(|t| t.error.is_some()) {
                super::super::home_dir()
            } else {
                self.current_path()
            };
            panel.setDirectoryURL(Some(&platform::file_url(&start)));
            let controller = self.retained();
            let selected = panel.clone();
            let done = block2::RcBlock::new(move |response: isize| {
                if response == 1 {
                    let paths = selected
                        .URLs()
                        .iter()
                        .filter(|url| url.isFileURL())
                        .map(|url| platform::url_path(&url))
                        .collect::<Vec<_>>();
                    if let Some(old) = &replace {
                        let Some(new) = paths.first() else {
                            return;
                        };
                        let mut prefs = controller
                            .ivars()
                            .runtime
                            .preferences
                            .lock()
                            .unwrap()
                            .clone();
                        match prefs.retarget_folder(group, old, new, &super::super::home_dir()) {
                            Ok(()) => controller.commit_preferences(prefs),
                            Err(error) => controller.notice(&error, None),
                        }
                    } else {
                        controller.pin_paths(group, &paths);
                    }
                }
            });
            panel.beginSheetModalForWindow_completionHandler(&self.views().window, &done);
        }
    }
    fn context_shortcut(&self) -> Option<(PinGroup, String, PathBuf, String)> {
        let state = self.ivars().state.borrow();
        let node = state.nodes.get(&state.sidebar_context?)?;
        let group = PinGroup::from_section(node.group)?;
        if !(node.key.starts_with("favorite:") || node.key.starts_with("pinned-location:")) {
            return None;
        }
        Some((
            group,
            node.key.clone(),
            node.path.clone()?,
            node.label.clone(),
        ))
    }
    pub(super) fn unpin_context_shortcut(&self) {
        if let Some((group, _, path, _)) = self.context_shortcut() {
            let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
            prefs.unpin_folder(group, &path);
            self.commit_preferences(prefs);
            self.notice(&format!("{}에서 바로가기 제거", group.title()), None);
        }
    }
    pub(super) fn rename_context_shortcut(&self) {
        let Some((group, key, path, label)) = self.context_shortcut() else {
            return;
        };
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        unsafe {
            let field =
                NSTextField::initWithFrame(NSTextField::alloc(self.mtm()), rect(0., 0., 340., 28.));
            single_line_editor(&field);
            field.setStringValue(&NSString::from_str(&label));
            field.setAccessibilityLabel(Some(ns_string!("바로가기 표시 이름")));
            let alert = NSAlert::new(self.mtm());
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(ns_string!("바로가기 표시 이름"));
            alert.setInformativeText(ns_string!(
                "사이드바와 Dock에서 사용할 이름을 입력하세요. 비우면 폴더 이름을 사용합니다."
            ));
            alert.addButtonWithTitle(ns_string!("적용"));
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.setAccessoryView(Some(&field));
            let controller = self.retained();
            let input = field.clone();
            let done = block2::RcBlock::new(move |response: isize| {
                if response != 1000 {
                    return;
                }
                let name = input.stringValue().to_string();
                let mut prefs = controller
                    .ivars()
                    .runtime
                    .preferences
                    .lock()
                    .unwrap()
                    .clone();
                if !prefs
                    .shortcuts(group, &super::super::home_dir())
                    .iter()
                    .any(|item| item.key == key && item.path == path)
                {
                    return;
                }
                if name.trim().is_empty() {
                    prefs.shortcut_labels.remove(&key);
                } else {
                    prefs
                        .shortcut_labels
                        .insert(key.clone(), name.trim().to_owned());
                }
                controller.commit_preferences(prefs);
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&done));
            alert.window().makeFirstResponder(Some(&field));
            field.selectText(None);
        }
    }
    pub(super) unsafe fn configure_pin_button(&self, view: &NSTableCellView, group: PinGroup) {
        const PIN_BUTTON: isize = 809;
        if let Some(button) = view
            .viewWithTag(PIN_BUTTON)
            .and_then(|v| v.downcast::<NSButton>().ok())
        {
            button.setContentTintColor(Some(&super::super::theme::color(
                self.views().palette.get().side_ink,
            )));
            return;
        }
        let button = NSButton::initWithFrame(
            NSButton::alloc(self.mtm()),
            rect(view.bounds().size.width - 26., 4., 22., 22.),
        );
        button.setTag(PIN_BUTTON);
        button.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin
                | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        button.setBordered(false);
        button.setTitle(ns_string!("+"));
        button.setContentTintColor(Some(&super::super::theme::color(
            self.views().palette.get().side_ink,
        )));
        button.setFont(Some(&NSFont::systemFontOfSize(16.)));
        let help = NSString::from_str(&format!("{}에 폴더 추가", group.title()));
        button.setToolTip(Some(&help));
        button.setAccessibilityLabel(Some(&help));
        button.setTarget(Some(self));
        button.setAction(Some(match group {
            PinGroup::Favorites => sel!(pickFavoriteFolders:),
            PinGroup::Locations => sel!(pickLocationFolders:),
        }));
        view.addSubview(&button);
    }
    pub(super) unsafe fn sidebar_menu(&self, menu: &NSMenu, background: bool) {
        let outline = &self.views().sidebar;
        let row = outline.clickedRow();
        let id = if background || row < 0 {
            None
        } else {
            outline.itemAtRow(row).and_then(|n| node_id(Some(&n)))
        };
        self.ivars().state.borrow_mut().sidebar_context = id;
        for (name, action) in [
            ("즐겨찾기에 폴더 추가…", sel!(pickFavoriteFolders:)),
            ("위치에 폴더 추가…", sel!(pickLocationFolders:)),
        ] {
            menu.addItem(&item(name, action, Some(self), "", self.mtm()));
        }
        menu.addItem(&item(
            "서버에 연결…",
            sel!(connectToServer:),
            Some(self),
            "",
            self.mtm(),
        ));
        menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
        if self.context_shortcut().is_some() {
            menu.addItem(&item(
                "바로가기 대상 폴더 변경…",
                sel!(retargetSidebarShortcut:),
                Some(self),
                "",
                self.mtm(),
            ));
            menu.addItem(&item(
                "바로가기 표시 이름 변경…",
                sel!(renameSidebarShortcut:),
                Some(self),
                "",
                self.mtm(),
            ));
            menu.addItem(&item(
                "사이드바에서 바로가기 제거",
                sel!(unpinSidebarShortcut:),
                Some(self),
                "",
                self.mtm(),
            ));
        }
        if let Some((path, key, section)) = {
            let s = self.ivars().state.borrow();
            id.and_then(|id| s.nodes.get(&id))
                .map(|n| (n.path.clone(), n.key.clone(), n.section))
        } {
            if let Some(path) = &path {
                menu.addItem(&item(
                    "새 탭에서 열기",
                    sel!(sidebarOpenTab:),
                    Some(self),
                    "",
                    self.mtm(),
                ));
                let prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
                for group in [PinGroup::Favorites, PinGroup::Locations] {
                    if !prefs
                        .shortcuts(group, &super::super::home_dir())
                        .iter()
                        .any(|shortcut| {
                            &shortcut.path == path && prefs.sidebar.visible(&shortcut.key)
                        })
                    {
                        let pin = item(
                            &format!("이 폴더를 {}에 고정", group.title()),
                            sel!(pinSidebarFolder:),
                            Some(self),
                            "",
                            self.mtm(),
                        );
                        pin.setTag(group.section() as isize);
                        menu.addItem(&pin);
                    }
                }
            }
            if let Some(path) = &path {
                let kind = self
                    .ivars()
                    .state
                    .borrow()
                    .locations
                    .iter()
                    .find(|l| &l.path == path)
                    .map(|l| l.kind);
                if kind.is_some_and(|kind| kind != LocationKind::Cloud) || path == Path::new("/") {
                    menu.addItem(&item(
                        "드라이브 정보…",
                        sel!(sidebarVolumeInfo:),
                        Some(self),
                        "",
                        self.mtm(),
                    ));
                }
                if kind == Some(LocationKind::Internal) || path == Path::new("/") {
                    menu.addItem(&item(
                        "저장 공간 관리…",
                        sel!(manageStorage:),
                        Some(self),
                        "",
                        self.mtm(),
                    ));
                }
            }
            if path.as_ref().is_some_and(|p| {
                self.ivars()
                    .state
                    .borrow()
                    .locations
                    .iter()
                    .any(|l| l.kind == LocationKind::Network && &l.path == p)
            }) {
                menu.addItem(&item(
                    "서버 연결 해제…",
                    sel!(disconnectSidebarServer:),
                    Some(self),
                    "",
                    self.mtm(),
                ));
            }
            if !section && !key.is_empty() {
                menu.addItem(&item(
                    "사이드바에서 숨기기",
                    sel!(sidebarHide:),
                    Some(self),
                    "",
                    self.mtm(),
                ));
                for (title, action) in [
                    ("위로 이동", sel!(sidebarMoveUp:)),
                    ("아래로 이동", sel!(sidebarMoveDown:)),
                ] {
                    menu.addItem(&item(title, action, Some(self), "", self.mtm()));
                }
            }
            menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
        }
        let settings = NSMenuItem::new(self.mtm());
        settings.setTitle(ns_string!("사이드바 표시 항목"));
        let submenu = NSMenu::new(self.mtm());
        let prefs = self.ivars().state.borrow().prefs.sidebar.clone();
        let mut previous = 0;
        for spec in self.sidebar_specs() {
            if previous != 0 && previous != spec.group {
                submenu.addItem(&NSMenuItem::separatorItem(self.mtm()));
            }
            previous = spec.group;
            let row = item(
                &spec.label,
                sel!(sidebarToggle:),
                Some(self),
                "",
                self.mtm(),
            );
            row.setRepresentedObject(Some(&NSString::from_str(&spec.key)));
            row.setState(if prefs.visible(&spec.key) {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            submenu.addItem(&row);
        }
        settings.setSubmenu(Some(&submenu));
        menu.addItem(&settings);
        menu.addItem(&item(
            "사이드바 기본 배치로 복원",
            sel!(sidebarReset:),
            Some(self),
            "",
            self.mtm(),
        ));
        menu.addItem(&item(
            "환경 설정…",
            sel!(settings:),
            Some(self),
            ",",
            self.mtm(),
        ));
    }
}

impl Controller {
    pub(super) fn open_storage_settings(&self) {
        let url = NSURL::URLWithString(ns_string!(
            "x-apple.systempreferences:com.apple.settings.Storage"
        ));
        if !url.is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url)) {
            self.message(
                "저장 공간 설정을 열 수 없습니다",
                "시스템 설정 → 일반 → 저장 공간에서 확인할 수 있습니다.",
            );
        }
    }
    pub(super) fn show_volume_info(&self) {
        let target = {
            let s = self.ivars().state.borrow();
            s.sidebar_context
                .and_then(|id| s.nodes.get(&id))
                .and_then(|n| n.path.clone().map(|p| (p, n.label.clone())))
        };
        let Some((path, label)) = target else { return };
        let sender = self.ivars().runtime.sender.clone();
        let window = self.id();
        let _=self.ivars().runtime.io.submit(move || {
            let detail=platform::volume_capacity(&path).map(|(total,available)|format!("위치: {}\n전체 용량: {}\n사용 가능: {}\n\nAPFS 볼륨은 같은 컨테이너의 여유 공간을 공유합니다.",path.display(),format_capacity(total),format_capacity(available))).unwrap_or_else(|e|format!("드라이브 정보를 읽을 수 없습니다: {e}"));
            let _=sender.send(Event::Message{window,title:format!("{label} — 드라이브 정보"),detail});
        });
    }
}

// Storage settings and disk manufacturers use decimal capacity units.
fn format_capacity(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 {
        format!("{:.2} TB", bytes as f64 / 1_000_000_000_000.)
    } else {
        format!("{:.2} GB", bytes as f64 / 1_000_000_000.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnect_invalidates_only_tabs_on_that_mount_and_reconnect_queues_refresh() {
        let local = Location {
            path: "/".into(),
            name: "Mac".into(),
            kind: LocationKind::Internal,
        };
        let remote = Location {
            path: "/Volumes/팀".into(),
            name: "팀".into(),
            kind: LocationKind::Network,
        };
        let connected = [local.clone(), remote];
        let disconnected = [local];
        let mut tabs = [
            Tab::new(1, "/Volumes/팀/사진".into()),
            Tab::new(2, "/Volumes/팀 백업".into()),
        ];
        tabs[0].generation = 12;
        tabs[0].loading = true;
        tabs[0].selected.insert(100);
        let pending = tabs[0].cancel.clone();
        assert_eq!(
            update_mounted_tabs(&mut tabs, &connected, &disconnected),
            vec![1]
        );
        assert!(pending.cancelled() && tabs[0].generation > 12);
        assert!(tabs[0].selected.is_empty() && !tabs[0].can_write() && !tabs[0].loading);
        assert!(tabs[1].can_write() && tabs[1].watch_due.is_none());
        assert!(update_mounted_tabs(&mut tabs, &disconnected, &connected).is_empty());
        assert!(tabs[0].watch_due.is_some() && tabs[1].watch_due.is_none());
    }
}
