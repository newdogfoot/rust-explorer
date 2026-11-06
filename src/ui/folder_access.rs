use super::*;

pub enum PendingAccess {
    Settings,
    Picker(Option<PathBuf>),
}

impl Controller {
    pub(super) fn dismiss_access_intro(&self) {
        let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
        prefs.access_intro_dismissed = true;
        self.commit_preferences(prefs);
    }
    pub(super) fn access_list_text(&self) -> String {
        let paths = self.ivars().runtime.folder_access.lock().unwrap().paths();
        if paths.is_empty() {
            return "아직 선택한 폴더가 없습니다.".into();
        }
        paths
            .iter()
            .map(|p| Self::display_location(p))
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub(super) unsafe fn access_form(&self) -> Retained<NSView> {
        let form = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0., 0., 490., 210.));
        let label = text(self.mtm(), "선택한 폴더 기록", 12.);
        label.setFrame(rect(0., 185., 490., 20.));
        form.addSubview(&label);
        let scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(self.mtm()), rect(0., 124., 490., 58.));
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        let list =
            NSTextView::initWithFrame(NSTextView::alloc(self.mtm()), rect(0., 0., 470., 58.));
        list.setVerticallyResizable(true);
        list.setHorizontallyResizable(false);
        list.setMinSize(NSSize::new(0., 58.));
        list.setMaxSize(NSSize::new(490., 1_000_000.));
        list.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        if let Some(container) = list.textContainer() {
            container.setContainerSize(NSSize::new(470., 1_000_000.));
            container.setWidthTracksTextView(true);
        }
        list.setEditable(false);
        list.setSelectable(true);
        list.setRichText(false);
        list.setDrawsBackground(false);
        list.setFont(Some(&NSFont::systemFontOfSize(12.)));
        list.setTextColor(Some(&NSColor::labelColor()));
        list.setString(&NSString::from_str(&self.access_list_text()));
        scroll.setDocumentView(Some(&list));
        form.addSubview(&scroll);
        self.ivars().state.borrow_mut().access_list = Some(list);
        let button = |title: &str, action, x, w| {
            let b = NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(self),
                Some(action),
                self.mtm(),
            );
            b.setFrame(rect(x, 94., w, 28.));
            form.addSubview(&b);
        };
        button("파일 및 폴더 설정…", sel!(openFilesPrivacy:), 0., 230.);
        button(
            "전체 디스크 접근 설정…",
            sel!(openFullDiskPrivacy:),
            240.,
            250.,
        );
        let help = NSTextField::wrappingLabelWithString(
            ns_string!(
                "전체 디스크 접근은 선택 사항입니다. 허용하면 문서 외에 메일 등 앱 데이터에도 접근할 수 있습니다. 파일 소유 권한과 시스템 보호는 별도로 적용됩니다."
            ),
            self.mtm(),
        );
        help.setSelectable(false);
        help.setFont(Some(&NSFont::systemFontOfSize(11.)));
        help.setTextColor(Some(&NSColor::secondaryLabelColor()));
        help.setFrame(rect(0., 36., 490., 52.));
        form.addSubview(&help);
        let forget = NSButton::buttonWithTitle_target_action(
            ns_string!("선택 기록 지우기"),
            Some(self),
            Some(sel!(forgetFolderAccess:)),
            self.mtm(),
        );
        forget.setFrame(rect(0., 0., 150., 26.));
        forget.setToolTip(Some(ns_string!(
            "앱이 기억한 목록만 지웁니다. macOS의 접근 허용은 변경하지 않습니다."
        )));
        form.addSubview(&forget);
        let reveal = NSButton::buttonWithTitle_target_action(
            ns_string!("현재 앱 위치 보기"),
            Some(self),
            Some(sel!(revealPermissionApp:)),
            self.mtm(),
        );
        reveal.setFrame(rect(320., 0., 170., 26.));
        reveal.setToolTip(Some(ns_string!(
            "시스템 설정에 추가할 현재 Rust Explorer 앱을 Finder에서 표시합니다."
        )));
        form.addSubview(&reveal);
        form
    }
    pub(super) unsafe fn access_alert(&self) -> Retained<NSAlert> {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("폴더 접근 설정"));
        alert.setInformativeText(ns_string!("문서·데스크탑·다운로드 등 사용할 폴더를 한 번에 선택할 수 있습니다. macOS의 파일 선택 창에서 ⌘ 키로 여러 폴더를 고르세요. 전체 디스크 접근을 변경했다면 앱을 다시 열어야 할 수 있습니다."));
        alert.setAccessoryView(Some(&self.access_form()));
        alert.addButtonWithTitle(ns_string!("폴더 선택…"));
        alert.addButtonWithTitle(ns_string!("닫기"));
        alert
    }
    pub(crate) fn folder_access_settings(&self) {
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        unsafe {
            let alert = self.access_alert();
            let controller = self.retained();
            let done = block2::RcBlock::new(move |response: isize| {
                let mut state = controller.ivars().state.borrow_mut();
                state.access_list = None;
                if response == 1000 {
                    state.pending_access = Some(PendingAccess::Picker(None));
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&done));
        }
    }
    pub(super) unsafe fn access_picker(&self, target: Option<&Path>) -> Retained<NSOpenPanel> {
        let panel = NSOpenPanel::openPanel(self.mtm());
        panel.setTitle(Some(ns_string!("사용할 폴더 선택")));
        panel.setPrompt(Some(ns_string!("선택한 폴더 사용")));
        panel.setMessage(Some(ns_string!("문서·데스크탑·다운로드 등을 ⌘ 키로 함께 선택할 수 있습니다. 선택한 폴더의 macOS 접근 허용을 사용합니다.")));
        panel.setCanChooseFiles(false);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(target.is_none());
        panel.setTreatsFilePackagesAsDirectories(false);
        // No exists()/read_dir probe here: it could prompt before selection or
        // stall on a disconnected network volume.
        let start = target
            .filter(|_| {
                !self.with_tab(|t| t.read_only && t.error.as_ref().is_none_or(|e| !e.access_denied))
            })
            .map(Path::to_owned)
            .unwrap_or_else(super::super::home_dir);
        panel.setDirectoryURL(Some(&platform::file_url(&start)));
        panel
    }
    pub(super) fn choose_access_folders(&self, target: Option<PathBuf>) {
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        unsafe {
            let panel = self.access_picker(target.as_deref());
            let pin = NSButton::checkboxWithTitle_target_action(
                ns_string!("선택한 폴더를 즐겨찾기에 추가"),
                None,
                None,
                self.mtm(),
            );
            pin.setFrame(rect(0., 0., 330., 26.));
            pin.setState(NSControlStateValueOn);
            panel.setAccessoryView(Some(&pin));
            let origin = target.map(|_| self.with_tab(|t| (t.id, t.path.clone())));
            let selected = panel.clone();
            let controller = self.retained();
            let done = block2::RcBlock::new(move |response: isize| {
                if response != 1 {
                    return;
                }
                let urls = selected.URLs().iter().collect::<Vec<_>>();
                if urls.is_empty() {
                    return;
                }
                if urls.len() > platform::folder_access::MAX_FOLDERS {
                    controller.notice("한 번에 최대 64개 폴더를 선택할 수 있습니다", None);
                    return;
                }
                if let Err(error) = controller.ivars().runtime.remember_access(
                    controller.id(),
                    origin.clone(),
                    urls,
                    pin.state() == NSControlStateValueOn,
                ) {
                    controller.notice(error, None);
                }
            });
            panel.beginSheetModalForWindow_completionHandler(&self.views().window, &done);
        }
    }
    pub(super) fn folder_access_changed(
        &self,
        origin: Option<(u64, PathBuf)>,
        paths: Vec<PathBuf>,
        detail: String,
        add_favorites: bool,
    ) {
        if add_favorites && !paths.is_empty() {
            self.pin_paths(PinGroup::Favorites, &paths);
        }
        if !paths.is_empty() {
            self.dismiss_access_intro();
        }
        if let Some(list) = self.ivars().state.borrow().access_list.clone() {
            list.setString(&NSString::from_str(&self.access_list_text()));
        }
        self.notice(&detail, None);
        if origin.is_some_and(|(id, path)| self.with_tab(|t| t.id == id && t.path == path)) {
            self.load(false);
        }
    }
    pub(super) fn open_files_privacy(&self, full_disk: bool) {
        let route = if full_disk {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
        } else {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders"
        };
        let origin = self.with_tab(|t| (t.id, t.path.clone()));
        self.ivars().state.borrow_mut().access_return = Some(origin);
        unsafe {
            let opened = NSURL::URLWithString(&NSString::from_str(route))
                .is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url));
            if !opened {
                self.ivars().state.borrow_mut().access_return = None;
                self.notice("시스템 설정 → 개인정보 보호 및 보안에서 파일 및 폴더 또는 전체 디스크 접근을 확인하세요",None);
            }
        }
    }
    pub(super) fn reveal_permission_app(&self) {
        unsafe {
            NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_slice(
                &[&*NSBundle::mainBundle().bundleURL()],
            ));
        }
    }
    pub(crate) fn resume_folder_access(&self) {
        let origin = self.ivars().state.borrow_mut().access_return.take();
        if origin.is_some_and(|(id, path)| {
            self.with_tab(|t| {
                t.id == id && t.path == path && t.error.as_ref().is_some_and(|e| e.access_denied)
            })
        }) {
            if self.ivars().state.borrow().dragging
                || self.ivars().state.borrow().renaming.is_some()
            {
                self.with_tab_mut(|t| t.watch_due = Some(Instant::now()));
            } else {
                self.load(false);
            }
        }
    }
    pub(super) fn folder_error_details(&self) {
        let Some(error) = self.with_tab(|t| t.error.as_ref().map(ToString::to_string)) else {
            return;
        };
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        unsafe {
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("폴더를 열 수 없습니다"));
            alert.setInformativeText(&NSString::from_str(&error));
            alert.addButtonWithTitle(ns_string!("닫기"));
            alert.addButtonWithTitle(ns_string!("오류 내용 복사"));
            let controller = self.retained();
            let done = block2::RcBlock::new(move |response: isize| {
                if response == 1001 {
                    controller.copy_text(&error);
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&done));
        }
    }
}
