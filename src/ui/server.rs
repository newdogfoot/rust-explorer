use super::*;
use crate::platform::server::{MountResult, MountStart, mount_server, server_url};

impl Controller {
    pub(super) fn disconnect_context_server(&self) {
        let Some(path) = self.sidebar_context_path() else {
            return;
        };
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("서버 연결을 해제할까요?"));
        alert.setInformativeText(&NSString::from_str(&path.to_string_lossy()));
        alert.addButtonWithTitle(ns_string!("취소"));
        alert.addButtonWithTitle(ns_string!("연결 해제"));
        let controller = self.retained();
        let done = block2::RcBlock::new(move |response: isize| {
            if response != 1001 {
                return;
            }
            let path = path.clone();
            let window = controller.id();
            let sender = controller.ivars().runtime.sender.clone();
            if let Err(error) = controller.ivars().runtime.io.submit(move || {
                let result = platform::server::disconnect_network_volume(&path);
                let _ = sender.send(Event::NetworkUnmounted { window, result });
            }) {
                controller.notice(error, None);
            }
        });
        unsafe {
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&done));
        }
    }
    pub(crate) fn connect_server_dialog(&self) {
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        if self.ivars().state.borrow().mount_request.is_some() {
            self.show_connection_in_progress();
            return;
        }
        self.cancel_inline_rename();
        unsafe {
            let field =
                NSComboBox::initWithFrame(NSComboBox::alloc(self.mtm()), rect(0., 0., 430., 28.));
            field.setPlaceholderString(Some(ns_string!("smb://nas/공유폴더")));
            field.setAccessibilityLabel(Some(ns_string!("서버 주소")));
            for address in self.ivars().runtime.recent_servers.lock().unwrap().iter() {
                field.addItemWithObjectValue(&NSString::from_str(address));
            }
            let alert = NSAlert::new(self.mtm());
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(ns_string!("서버에 연결"));
            alert.setInformativeText(ns_string!("SMB, NFS, WebDAV 주소 또는 Windows 공유 경로를 입력하세요. 필요한 계정 정보는 macOS 로그인 창에서 입력합니다."));
            alert.addButtonWithTitle(ns_string!("연결"));
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.setAccessoryView(Some(&field));
            let controller = self.retained();
            let input = field.clone();
            let done = block2::RcBlock::new(move |response: isize| {
                if response != 1000 {
                    return;
                }
                match server_url(&input.stringValue().to_string()) {
                    Ok(url) => controller.start_mount(&url),
                    Err(error) => controller.message("서버 주소를 확인하세요", &error),
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&done));
            alert.window().makeFirstResponder(Some(&field));
        }
    }
    fn start_mount(&self, url: &NSURL) {
        let id = self.ivars().runtime.next_id();
        let address = url
            .absoluteString()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let origin = self.with_tab(|t| (t.id, t.path.clone()));
        self.ivars().state.borrow_mut().folder_request = id;
        let window = self.id();
        let callback_origin = origin.clone();
        let callback_address = address.clone();
        let finished = move |result| {
            // NetFS delivers on the main queue. Its block owns only ordinary
            // Rust data, so releasing the block cannot destroy an AppKit object
            // on a NetFS worker thread.
            let target = super::super::APPLICATION.with(|cell| {
                cell.borrow().as_ref().and_then(|app| {
                    app.ivars()
                        .windows
                        .borrow()
                        .iter()
                        .find(|c| c.id() == window)
                        .cloned()
                })
            });
            if let Some(controller) = target {
                {
                    let mut state = controller.ivars().state.borrow_mut();
                    if state
                        .mount_request
                        .as_ref()
                        .is_none_or(|(active, _)| *active != id)
                    {
                        return;
                    }
                    state.mount_request = None;
                }
                controller.finish_mount(id, &callback_origin, &callback_address, result);
            }
        };
        match mount_server(url, finished) {
            Ok(MountStart::Pending(request)) => {
                self.ivars().state.borrow_mut().mount_request = Some((id, request));
                self.notice("서버 연결 중… · 이동 메뉴에서 취소할 수 있습니다", None);
            }
            Ok(MountStart::Finished(outcome)) => {
                self.finish_mount(id, &origin, &address, Ok(outcome))
            }
            Err(error) => self.message("서버 연결 실패", &error),
        }
    }
    fn finish_mount(
        &self,
        id: u64,
        origin: &(u64, PathBuf),
        address: &str,
        result: Result<MountResult, String>,
    ) {
        let paths = match result {
            Ok(MountResult::Connected(paths)) => paths,
            Ok(MountResult::AlreadyMounted) => Vec::new(),
            Ok(MountResult::Cancelled) => {
                self.notice("서버 연결 취소", None);
                return;
            }
            Err(error) => {
                self.message("서버 연결 실패", &error);
                return;
            }
        };
        self.ivars().runtime.remember_server(address.to_owned());
        self.ivars().runtime.volumes();
        self.notice("서버 연결됨 · 사이드바의 위치에서 열 수 있습니다", None);
        if let Some(path) = paths.into_iter().next()
            && self.ivars().state.borrow().folder_request == id
            && self.with_tab(|t| t.id == origin.0 && t.path == origin.1)
            && self.views().window.attachedSheet().is_none()
            && self.ivars().state.borrow().renaming.is_none()
            && !self
                .views()
                .window
                .firstResponder()
                .is_some_and(|view| view.isKindOfClass(NSTextView::class()))
        {
            self.request_folder(path);
        }
    }
    pub(super) fn open_server_address(&self, input: &str) -> bool {
        if !platform::server::is_server_address(input) {
            return false;
        }
        if self.ivars().state.borrow().mount_request.is_some() {
            self.show_connection_in_progress();
            return true;
        }
        self.focus_list();
        match server_url(input) {
            Ok(url) => self.start_mount(&url),
            Err(error) => self.message("서버 주소를 확인하세요", &error),
        }
        true
    }
    pub(crate) fn abandon_mount(&self) {
        let request = self.ivars().state.borrow_mut().mount_request.take();
        if let Some((_, handle)) = request {
            let _ = handle.cancel();
        }
    }
    pub(super) fn cancel_mount(&self) {
        let request = self.ivars().state.borrow_mut().mount_request.take();
        if let Some((_, handle)) = request {
            let message = if handle.cancel() {
                "서버 연결 취소"
            } else {
                "연결 상태가 변경되었습니다. 사이드바의 위치를 확인하세요."
            };
            self.notice(message, None);
        }
    }
    fn show_connection_in_progress(&self) {
        let alert = NSAlert::new(self.mtm());
        alert.setAlertStyle(NSAlertStyle::Informational);
        alert.setMessageText(ns_string!("서버에 연결하는 중입니다"));
        alert.addButtonWithTitle(ns_string!("계속 기다리기"));
        alert.addButtonWithTitle(ns_string!("연결 취소"));
        let controller = self.retained();
        let id = self
            .ivars()
            .state
            .borrow()
            .mount_request
            .as_ref()
            .map(|(id, _)| *id);
        let done = block2::RcBlock::new(move |response: isize| {
            if response == 1001
                && controller
                    .ivars()
                    .state
                    .borrow()
                    .mount_request
                    .as_ref()
                    .map(|(id, _)| *id)
                    == id
            {
                controller.cancel_mount();
            }
        });
        unsafe {
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&done));
        }
    }
}
