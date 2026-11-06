use super::*;

pub struct ApplicationDialog {
    pub id: u64,
    pub path: PathBuf,
    pub cancel: Cancellation,
    pub popup: Retained<NSPopUpButton>,
    pub open: Retained<NSButton>,
    pub detail: Retained<NSTextField>,
    pub apps: Vec<platform::OpenApplication>,
}

impl Controller {
    pub(super) fn duplicate_selected(&self) {
        let paths = self.selected_paths_in_order();
        if paths.is_empty() || self.views().window.attachedSheet().is_some() {
            return;
        }
        let origin = self.with_tab(|t| (t.id, t.path.clone()));
        self.start_creation(
            Operation {
                completed_sources: vec![],
                renames: vec![],
                kind: OperationKind::Duplicate,
                sources: paths.iter().map(|p| StoredPath::new(p)).collect(),
                destination: None,
                name: None,
            },
            origin,
        );
    }
    pub(super) fn can_open_with(&self) -> bool {
        self.with_tab(|t| {
            t.selected.len() == 1
                && t.entries
                    .iter()
                    .any(|e| t.selected.contains(&e.id) && e.kind == Kind::File)
        })
    }
    pub(super) fn open_with_dialog(&self) {
        if !self.can_open_with() || self.views().window.attachedSheet().is_some() {
            return;
        }
        let Some(path) = self.selected_paths_in_order().into_iter().next() else {
            return;
        };
        self.cancel_inline_rename();
        self.ivars().state.borrow_mut().folder_request = 0;
        unsafe {
            let panel = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0., 0., 430., 64.));
            let popup = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(self.mtm()),
                rect(0., 33., 430., 28.),
                false,
            );
            popup.addItemWithTitle(ns_string!("앱 목록을 확인하는 중…"));
            popup.setEnabled(false);
            popup.setAccessibilityLabel(Some(ns_string!("연결 프로그램")));
            panel.addSubview(&popup);
            let detail = text(
                self.mtm(),
                "이번에만 이 앱으로 엽니다. 기본 앱 설정은 유지합니다.",
                12.,
            );
            detail.setFrame(rect(0., 0., 430., 25.));
            panel.addSubview(&detail);
            let alert = NSAlert::new(self.mtm());
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(ns_string!("연결 프로그램 선택"));
            alert.setInformativeText(&NSString::from_str(&format!(
                "‘{}’ 파일을 열 앱을 선택하세요.",
                path.file_name().unwrap_or_default().to_string_lossy()
            )));
            let open = alert.addButtonWithTitle(ns_string!("열기"));
            open.setEnabled(false);
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.addButtonWithTitle(ns_string!("다른 앱 선택…"));
            alert.setAccessoryView(Some(&panel));
            let id = self.ivars().runtime.next_id();
            let cancel = Cancellation::default();
            self.ivars().state.borrow_mut().application_dialog = Some(ApplicationDialog {
                id,
                path: path.clone(),
                cancel: cancel.clone(),
                popup,
                open,
                detail,
                apps: vec![],
            });
            let controller = self.retained();
            let callback = block2::RcBlock::new(move |response: isize| {
                let Some(dialog) = controller
                    .ivars()
                    .state
                    .borrow_mut()
                    .application_dialog
                    .take()
                else {
                    return;
                };
                dialog.cancel.cancel();
                if response == 1000 {
                    if let Some(app) = dialog.apps.get(dialog.popup.indexOfSelectedItem() as usize)
                    {
                        controller.open_file_with(&dialog.path, &app.path);
                    }
                } else if response == 1002 {
                    controller.choose_application(dialog.path);
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
            let window = self.id();
            let sender = self.ivars().runtime.sender.clone();
            if let Err(error) = self.ivars().runtime.io.submit(move || {
                let result =
                    platform::applications_for_file(&path, &cancel).map_err(|e| e.to_string());
                let _ = sender.send(Event::Applications {
                    window,
                    request: id,
                    result,
                });
            }) {
                self.show_applications(id, Err(error.into()));
            }
        }
    }
    pub(super) fn show_applications(
        &self,
        id: u64,
        result: Result<Vec<platform::OpenApplication>, String>,
    ) {
        let controls = {
            let mut state = self.ivars().state.borrow_mut();
            let Some(d) = state.application_dialog.as_mut().filter(|d| d.id == id) else {
                return;
            };
            let error = match result {
                Ok(apps) => {
                    d.apps = apps;
                    None
                }
                Err(error) => Some(error),
            };
            (
                d.popup.clone(),
                d.open.clone(),
                d.detail.clone(),
                d.apps.clone(),
                error,
            )
        };
        let (popup, open, detail, apps, error) = controls;
        unsafe {
            popup.removeAllItems();
            for app in &apps {
                let label = if apps.iter().filter(|other| other.name == app.name).count() > 1 {
                    format!(
                        "{} — {}",
                        app.name,
                        app.path.parent().unwrap_or(Path::new("")).display()
                    )
                } else {
                    app.name.clone()
                };
                popup.addItemWithTitle(&NSString::from_str(&format!(
                    "{}{}",
                    label,
                    if app.is_default { " (기본 앱)" } else { "" }
                )));
                if let Some(item) = popup.lastItem() {
                    item.setToolTip(Some(&NSString::from_str(&app.path.to_string_lossy())));
                }
            }
            if apps.is_empty() {
                popup.addItemWithTitle(ns_string!("추천 앱이 없습니다"));
            }
            popup.setEnabled(!apps.is_empty());
            open.setEnabled(!apps.is_empty());
            if let Some(error) = error {
                detail.setStringValue(&NSString::from_str(&error));
            } else if apps.is_empty() {
                detail.setStringValue(ns_string!(
                    "‘다른 앱 선택…’에서 응용 프로그램을 지정하세요."
                ));
            }
        }
    }
    fn choose_application(&self, path: PathBuf) {
        unsafe {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setTitle(Some(ns_string!("응용 프로그램 선택")));
            panel.setMessage(Some(ns_string!(
                "이 파일을 한 번 열 응용 프로그램을 선택하세요."
            )));
            panel.setPrompt(Some(ns_string!("선택")));
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(false);
            panel.setAllowsMultipleSelection(false);
            panel.setTreatsFilePackagesAsDirectories(false);
            panel.setAllowedContentTypes(&NSArray::from_slice(&[
                objc2_uniform_type_identifiers::UTTypeApplicationBundle,
            ]));
            panel.setDirectoryURL(Some(&platform::file_url(Path::new("/Applications"))));
            let owned = panel.clone();
            let controller = self.retained();
            let callback = block2::RcBlock::new(move |response: isize| {
                if response == 1
                    && let Some(url) = owned.URL()
                {
                    controller.open_file_with(&path, &platform::url_path(&url));
                }
            });
            panel.beginSheetModalForWindow_completionHandler(&self.views().window, &callback);
        }
    }
    fn open_file_with(&self, path: &Path, application: &Path) {
        let configuration = NSWorkspaceOpenConfiguration::configuration();
        configuration.setActivates(true);
        configuration.setPromptsUserIfNeeded(true);
        let sender = self.ivars().runtime.sender.clone();
        let window = self.id();
        let callback =
            block2::RcBlock::new(move |_: *mut NSRunningApplication, error: *mut NSError| {
                if let Some(error) = unsafe { error.as_ref() } {
                    let _ = sender.send(Event::Message {
                        window,
                        title: "선택한 앱으로 파일을 열 수 없습니다".into(),
                        detail: error.localizedDescription().to_string(),
                    });
                }
            });
        NSWorkspace::sharedWorkspace()
            .openURLs_withApplicationAtURL_configuration_completionHandler(
                &NSArray::from_retained_slice(&[platform::file_url(path)]),
                &platform::file_url(application),
                &configuration,
                Some(&callback),
            );
    }
    pub(crate) fn go_to_folder_dialog(&self) {
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        self.cancel_inline_rename();
        self.ivars().state.borrow_mut().folder_request = 0;
        unsafe {
            let combo =
                NSComboBox::initWithFrame(NSComboBox::alloc(self.mtm()), rect(0., 0., 460., 28.));
            combo.setCompletes(true);
            combo.setAccessibilityLabel(Some(ns_string!("이동할 폴더 경로")));
            let recent = self.ivars().runtime.recent_folders.lock().unwrap().clone();
            for folder in &recent {
                combo.addItemWithObjectValue(&NSString::from_str(&Self::display_location(
                    &folder.path(),
                )));
            }
            combo.setStringValue(&NSString::from_str(&Self::display_location(
                &self.current_path(),
            )));
            let alert = NSAlert::new(self.mtm());
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(ns_string!("폴더로 이동"));
            alert.setInformativeText(ns_string!("~/Downloads 같은 경로를 입력하거나 최근 폴더를 선택하세요. 상대 경로도 사용할 수 있습니다."));
            alert.addButtonWithTitle(ns_string!("이동"));
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.setAccessoryView(Some(&combo));
            let origin = self.current_path();
            let controller = self.retained();
            let field = combo.clone();
            let callback = block2::RcBlock::new(move |response: isize| {
                if response != 1000 {
                    return;
                }
                let text = field.stringValue().to_string();
                if controller.open_server_address(&text) {
                    return;
                }
                let selected = recent
                    .get(field.indexOfSelectedItem() as usize)
                    .filter(|folder| Self::display_location(&folder.path()) == text);
                let path = if let Some(folder) = selected {
                    folder.path()
                } else if text == Self::display_location(&origin) {
                    origin.clone()
                } else if text == "~" {
                    super::super::home_dir()
                } else if let Some(rest) = text.strip_prefix("~/") {
                    super::super::home_dir().join(rest)
                } else {
                    PathBuf::from(text)
                };
                if path.as_os_str().is_empty() {
                    return;
                }
                let path = if path.is_absolute() {
                    path
                } else {
                    origin.join(path)
                };
                controller.request_folder(path);
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
            alert.window().makeFirstResponder(Some(&combo));
            combo.selectText(None);
        }
    }
    pub(crate) fn display_location(path: &Path) -> String {
        let home = super::super::home_dir();
        if path == home {
            "~".into()
        } else if let Ok(relative) = path.strip_prefix(home) {
            format!("~/{}", relative.display())
        } else {
            path.to_string_lossy().into_owned()
        }
    }
    pub(crate) fn request_folder(&self, path: PathBuf) {
        self.focus_list();
        let request = self.ivars().runtime.next_id();
        self.ivars().state.borrow_mut().folder_request = request;
        let (tab, origin) = self.with_tab(|t| (t.id, t.path.clone()));
        if let Err(error) = self
            .ivars()
            .runtime
            .open_folder(self.id(), request, tab, origin, path)
        {
            self.ivars().state.borrow_mut().folder_request = 0;
            self.message("폴더를 열 수 없습니다", error);
        }
    }
    pub(super) fn folder_ready(
        &self,
        request: u64,
        tab: u64,
        origin: PathBuf,
        result: Result<PathBuf, String>,
    ) {
        if self.ivars().state.borrow().folder_request != request {
            return;
        }
        self.ivars().state.borrow_mut().folder_request = 0;
        if !self.with_tab(|t| t.id == tab && t.path == origin)
            || self.ivars().state.borrow().renaming.is_some()
            || self.views().window.attachedSheet().is_some()
            || self
                .views()
                .window
                .firstResponder()
                .is_some_and(|r| r.isKindOfClass(NSTextView::class()))
        {
            return;
        }
        match result {
            Ok(path) => {
                self.navigate(path, true);
                self.focus_list();
            }
            Err(error) => self.message("폴더를 열 수 없습니다", &error),
        }
    }
}
