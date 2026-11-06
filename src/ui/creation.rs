use super::*;
use crate::core::archive::ArchiveFormat;

pub struct PendingCreation {
    pub tab: u64,
    pub folder: PathBuf,
    pub paths: Vec<PathBuf>,
    pub rename: bool,
    pub expires: Instant,
}

impl Controller {
    pub(super) fn start_creation(&self, request: Operation, origin: (u64, PathBuf)) {
        let rename = request.name.is_none()
            && matches!(
                request.kind,
                OperationKind::CreateFolder
                    | OperationKind::CreateTextFile
                    | OperationKind::CreateFile(_)
            );
        if let Some(id) = self.start_operation(request) {
            self.ivars()
                .state
                .borrow_mut()
                .creation_jobs
                .insert(id, (origin.0, origin.1, rename));
        }
    }
    pub(super) fn create_item(&self, kind: OperationKind, name: Option<Vec<u8>>) {
        if !self.with_tab(|t| t.can_write()) || self.views().window.attachedSheet().is_some() {
            return;
        }
        let origin = self.with_tab(|t| (t.id, t.path.clone()));
        self.start_creation(
            Operation {
                completed_sources: vec![],
                renames: vec![],
                kind,
                sources: vec![],
                destination: Some(StoredPath::new(&origin.1)),
                name,
            },
            origin,
        );
    }
    pub(super) fn choose_file_name(&self) {
        if !self.with_tab(|t| t.can_write()) || self.views().window.attachedSheet().is_some() {
            return;
        }
        let origin = self.with_tab(|t| (t.id, t.path.clone()));
        unsafe {
            let field =
                NSTextField::initWithFrame(NSTextField::alloc(self.mtm()), rect(0., 0., 350., 28.));
            single_line_editor(&field);
            field.setStringValue(ns_string!("새 파일"));
            field.setAccessibilityLabel(Some(ns_string!("파일 이름과 확장자")));
            let alert = NSAlert::new(self.mtm());
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(ns_string!("빈 파일 만들기"));
            alert.setInformativeText(ns_string!(
                "파일 이름과 원하는 확장자를 입력하세요. 같은 이름이 있으면 번호를 붙입니다."
            ));
            alert.addButtonWithTitle(ns_string!("만들기"));
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.setAccessoryView(Some(&field));
            let controller = self.retained();
            let input = field.clone();
            let done = block2::RcBlock::new(move |response: isize| {
                if response != 1000 {
                    return;
                }
                let name = input.stringValue().to_string().into_bytes();
                if let Err(error) = valid_name(&name) {
                    controller.notice(&error.to_string(), None);
                    return;
                }
                let mut request = Operation::transfer(
                    OperationKind::CreateFile(FileTemplate::Empty),
                    vec![],
                    origin.1.clone(),
                );
                request.name = Some(name);
                controller.start_creation(request, origin.clone());
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&done));
            alert.window().makeFirstResponder(Some(&field));
            field.selectText(None);
        }
    }
    pub(super) fn compress_dialog(&self, format: ArchiveFormat) {
        let sources = self.selected_paths();
        if sources.is_empty()
            || !self.with_tab(|t| t.can_write())
            || self.views().window.attachedSheet().is_some()
        {
            return;
        }
        let origin = self.with_tab(|t| (t.id, t.path.clone()));
        let stem = if sources.len() == 1 {
            sources[0]
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "압축 파일".into())
        } else {
            "압축 파일".into()
        };
        unsafe {
            let field =
                NSTextField::initWithFrame(NSTextField::alloc(self.mtm()), rect(0., 0., 350., 28.));
            field.setStringValue(&NSString::from_str(&format!(
                "{stem}.{}",
                format.extension()
            )));
            field.setAccessibilityLabel(Some(ns_string!("압축파일 이름")));
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(&NSString::from_str(&format!(
                "{} 압축하기",
                if format == ArchiveFormat::Zip {
                    "ZIP으로"
                } else {
                    "7z로"
                }
            )));
            alert.setInformativeText(&NSString::from_str(&format!("선택한 {}개 항목을 현재 폴더에 압축합니다. 원본은 유지하며 같은 이름이 있으면 번호를 붙입니다.", sources.len())));
            alert.addButtonWithTitle(ns_string!("압축하기"));
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.setAccessoryView(Some(&field));
            let controller = self.retained();
            let callback = block2::RcBlock::new(move |response: isize| {
                if response != 1000 {
                    return;
                }
                let mut name = field.stringValue().to_string();
                if name.trim().is_empty() {
                    controller.notice("압축파일 이름을 입력하세요", None);
                    return;
                }
                if !name
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{}", format.extension()))
                {
                    name.push('.');
                    name.push_str(format.extension());
                }
                controller.start_creation(
                    Operation {
                        completed_sources: vec![],
                        renames: vec![],
                        kind: if format == ArchiveFormat::Zip {
                            OperationKind::CompressZip
                        } else {
                            OperationKind::Compress7z
                        },
                        sources: sources.iter().map(|p| StoredPath::new(p)).collect(),
                        destination: Some(StoredPath::new(&origin.1)),
                        name: Some(name.into_bytes()),
                    },
                    origin.clone(),
                );
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
        }
    }
    pub(super) fn follow_created_item(&self) {
        let (row, ids, rename) = {
            let state = self.ivars().state.borrow();
            let Some(pending) = state.pending_creation.as_ref() else {
                return;
            };
            let tab = &state.tabs[state.active];
            if tab.id != pending.tab
                || tab.path != pending.folder
                || Instant::now() >= pending.expires
                || !tab.query.is_empty()
            {
                drop(state);
                self.ivars().state.borrow_mut().pending_creation = None;
                return;
            }
            if tab.loading
                || tab.refreshing
                || state.renaming.is_some()
                || state.dragging
                || self.views().window.attachedSheet().is_some()
                || !self.views().window.isKeyWindow()
                || unsafe {
                    NSRunLoop::mainRunLoop()
                        .currentMode()
                        .is_some_and(|mode| *mode != *NSDefaultRunLoopMode)
                }
            {
                return;
            }
            let ids: HashSet<_> = pending.paths.iter().map(|p| path_id(p)).collect();
            let Some(row) = tab
                .visible
                .iter()
                .position(|&i| ids.contains(&tab.entries[i].id))
            else {
                return;
            };
            (row, ids, pending.rename)
        };
        self.ivars().state.borrow_mut().pending_creation = None;
        self.with_tab_mut(|t| {
            t.selected = ids;
        });
        self.reload();
        unsafe {
            if self.ivars().state.borrow().prefs.view == ViewMode::Icons {
                let index = NSIndexPath::indexPathForItem_inSection(row as isize, 0);
                self.views()
                    .collection
                    .scrollToItemsAtIndexPaths_scrollPosition(
                        &NSSet::from_retained_slice(&[index]),
                        NSCollectionViewScrollPosition::NearestVerticalEdge,
                    );
                self.views().collection.layoutSubtreeIfNeeded();
            } else {
                self.views().table.scrollRowToVisible(row as isize);
                self.views().table.layoutSubtreeIfNeeded();
            }
        }
        self.focus_list();
        if rename {
            self.begin_rename();
        }
    }
    pub(super) fn can_send_bluetooth(&self) -> bool {
        self.with_tab(|t| {
            t.selected.len() == 1
                && t.entries
                    .iter()
                    .any(|e| t.selected.contains(&e.id) && e.kind == Kind::File)
        })
    }
    pub(super) fn send_bluetooth(&self) {
        let paths = self.selected_paths();
        if paths.len() != 1 || !std::fs::symlink_metadata(&paths[0]).is_ok_and(|m| m.is_file()) {
            self.message("Bluetooth 파일 교환", "파일 하나를 선택하세요. 폴더나 여러 파일은 ZIP 또는 7z로 압축한 뒤 보낼 수 있습니다.");
            return;
        }
        let workspace = NSWorkspace::sharedWorkspace();
        let Some(application) = workspace
            .URLForApplicationWithBundleIdentifier(ns_string!("com.apple.BluetoothFileExchange"))
        else {
            self.message(
                "Bluetooth 파일 교환을 찾을 수 없습니다",
                "이 Mac에서 Bluetooth 파일 교환 앱을 사용할 수 없습니다.",
            );
            return;
        };
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
                        title: "Bluetooth 파일 교환을 열 수 없습니다".into(),
                        detail: error.localizedDescription().to_string(),
                    });
                }
            });
        workspace.openURLs_withApplicationAtURL_configuration_completionHandler(
            &NSArray::from_retained_slice(&[platform::file_url(&paths[0])]),
            &application,
            &configuration,
            Some(&callback),
        );
    }
    pub(super) fn send_airdrop(&self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        unsafe {
            let Some(service) =
                NSSharingService::sharingServiceNamed(NSSharingServiceNameSendViaAirDrop)
            else {
                self.message(
                    "AirDrop을 사용할 수 없습니다",
                    "이 Mac의 AirDrop 지원 여부를 확인하세요.",
                );
                return;
            };
            let urls: Vec<Retained<AnyObject>> = paths
                .iter()
                .map(|path| Retained::into_super(Retained::into_super(platform::file_url(path))))
                .collect();
            let items = NSArray::from_retained_slice(&urls);
            if !service.canPerformWithItems(Some(&items)) {
                self.message("AirDrop", "선택한 항목을 AirDrop으로 보낼 수 없습니다.");
                return;
            }
            self.ivars().state.borrow_mut().sharing_service = Some(service.clone());
            service.setDelegate(Some(ProtocolObject::from_ref(self)));
            service.performWithItems(&items);
        }
    }
}
