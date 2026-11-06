#![allow(unsafe_op_in_unsafe_fn, unused_unsafe)]
mod appearance;
mod controller;
pub mod design;
mod navigation_menu;
mod runtime;
mod selection;
mod system_f2;
mod theme;
mod widgets;

use crate::core::{listing::ScanUpdate, model::*, store::Store};
use controller::Controller;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use runtime::{Event, Runtime};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

thread_local! {static APPLICATION:RefCell<Option<Retained<AppDelegate>>>=const{RefCell::new(None)};}

fn bundled_dock_icon(bundle: &NSBundle) -> Option<Retained<NSImage>> {
    let file = bundle
        .objectForInfoDictionaryKey(ns_string!("CFBundleIconFile"))
        .and_then(|value| value.downcast::<NSString>().ok())?;
    let url = bundle.URLForResource_withExtension(Some(&file), None)?;
    NSImage::initWithContentsOfURL(NSImage::alloc(), &url)
}

fn apply_bundled_dock_icon(app: &NSApplication) {
    if let Some(image) = bundled_dock_icon(&NSBundle::mainBundle()) {
        // Read this bundle's artwork directly. The Dock may still have an
        // earlier build's icon cached for the same application identifier.
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
#[derive(Default, serde::Serialize)]
struct Metrics {
    window_ready_ms: f64,
    listings: Vec<serde_json::Value>,
    inputs: Vec<serde_json::Value>,
}
struct AppIvars {
    runtime: Arc<Runtime>,
    windows: RefCell<Vec<Retained<Controller>>>,
    last_active_window: Cell<Option<u64>>,
    closed: RefCell<Vec<u64>>,
    timer: OnceCell<Retained<NSTimer>>,
    monitor: OnceCell<Retained<AnyObject>>,
    system_f2: RefCell<system_f2::Service>,
    metrics_path: Option<PathBuf>,
    metrics: RefCell<Metrics>,
    startup: Instant,
    initial: Vec<SavedTab>,
    quit_after: Option<Duration>,
    quit_when_idle: Cell<bool>,
    dark: bool,
    light: bool,
}
define_class!(
 #[unsafe(super=NSObject)]
 #[name="RERustExplorerApplicationDelegate"]
 #[thread_kind=MainThreadOnly]
 #[ivars=AppIvars]
 struct AppDelegate;
 unsafe impl NSObjectProtocol for AppDelegate{}
 unsafe impl NSMenuDelegate for AppDelegate {
  #[unsafe(method(menuNeedsUpdate:))]
  fn update_navigation_menu(&self, menu:&NSMenu){self.fill_folder_menu(menu);}
 }
 unsafe impl NSApplicationDelegate for AppDelegate{
  #[unsafe(method_id(applicationDockMenu:))]
  fn application_dock_menu(&self,_:&NSApplication)->Option<Retained<NSMenu>>{Some(unsafe{self.dock_menu()})}

  #[unsafe(method(applicationDidFinishLaunching:))]fn did_launch(&self,_:&NSNotification){unsafe{
   let mtm=self.mtm();let app=NSApplication::sharedApplication(mtm);
   apply_bundled_dock_icon(&app);
   if self.ivars().light{app.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameAqua).as_deref());}else if self.ivars().dark{app.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua).as_deref());}
   self.install_menu();self.open_window(self.ivars().initial.clone());
   let timer=NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(0.05,self,sel!(tick:),None,true);timer.setTolerance(0.01);NSRunLoop::mainRunLoop().addTimer_forMode(&timer,NSRunLoopCommonModes);self.ivars().timer.set(timer).ok();
   let block=block2::RcBlock::new(|event:std::ptr::NonNull<NSEvent>|->*mut NSEvent{let event=event.as_ref();let received=Instant::now();let measured=APPLICATION.with(|a|a.borrow().as_ref().is_some_and(|a|a.ivars().metrics_path.is_some()));let age=NSProcessInfo::processInfo().systemUptime()-event.timestamp();let queued=if event.timestamp()>0.&&(0. ..60.).contains(&age){age}else{0.};let busy=APPLICATION.with(|a|a.borrow().as_ref().is_some_and(|a|!a.ivars().runtime.jobs.lock().unwrap().is_empty()));let handled=APPLICATION.with(|a|a.borrow().as_ref().and_then(|app|app.active_controller()).is_some_and(|c|c.handle_input(event)));if measured{dispatch2::DispatchQueue::main().exec_async(move||{APPLICATION.with(|app|{if let Some(app)=app.borrow().as_ref(){if let Some(c)=app.active_controller(){c.views().window.displayIfNeeded();}let mut metrics=app.ivars().metrics.borrow_mut();if metrics.inputs.len()<4096{metrics.inputs.push(serde_json::json!({"latency_ms":(queued+received.elapsed().as_secs_f64())*1000.,"file_operation_active":busy}));}}});});}
  if handled{std::ptr::null_mut()}else{event as *const NSEvent as *mut NSEvent}});
   if let Some(monitor)=NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown | NSEventMask::KeyUp | NSEventMask::SystemDefined | NSEventMask::OtherMouseDown | NSEventMask::OtherMouseUp,&block){self.ivars().monitor.set(monitor).ok();}
   let nc=NSWorkspace::sharedWorkspace().notificationCenter();nc.addObserver_selector_name_object(self,sel!(volumesChanged:),Some(NSWorkspaceDidMountNotification),None);nc.addObserver_selector_name_object(self,sel!(volumesChanged:),Some(NSWorkspaceDidUnmountNotification),None);
   self.ivars().runtime.recover();
   app.activate();
   self.sync_system_f2(true);
   let ready=self.ivars().startup.elapsed().as_secs_f64()*1000.;self.ivars().metrics.borrow_mut().window_ready_ms=ready;if self.ivars().metrics_path.is_some(){eprintln!("RE_METRIC window_ready_ms={ready:.3}");}
  }}
  #[unsafe(method(applicationWillTerminate:))]fn will_terminate(&self,_:&NSNotification){for c in self.ivars().windows.borrow().iter(){c.abandon_mount();}self.ivars().system_f2.borrow_mut().stop();let _=self.ivars().runtime.persistence.flush(Duration::from_secs(2));if let Some(path)=&self.ivars().metrics_path&& let Ok(data)=serde_json::to_vec_pretty(&*self.ivars().metrics.borrow()){let _=std::fs::write(path,data);}}
  #[unsafe(method(applicationDidBecomeActive:))]fn became_active(&self,_:&NSNotification){self.sync_system_f2(true);for c in self.ivars().windows.borrow().iter(){c.refresh_input_diagnostics();c.resume_folder_access();}}
  #[unsafe(method(applicationDidResignActive:))]fn resigned_active(&self,_:&NSNotification){self.sync_system_f2(true);for c in self.ivars().windows.borrow().iter(){c.refresh_input_diagnostics();}}
  #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]fn terminate_after_close(&self,_:&NSApplication)->bool{false}
  #[unsafe(method(applicationShouldTerminate:))]fn should_terminate(&self,_:&NSApplication)->NSApplicationTerminateReply{unsafe{
   if self.ivars().runtime.jobs.lock().unwrap().is_empty(){if let Some(c)=self.active_controller(){c.save_tabs();}
  if !self.ivars().runtime.persistence.flush(Duration::from_secs(2)){return NSApplicationTerminateReply::TerminateCancel;}return NSApplicationTerminateReply::TerminateNow;}
   let alert=NSAlert::new(self.mtm());alert.setMessageText(ns_string!("파일 작업이 진행 중입니다"));alert.setInformativeText(ns_string!("작업을 취소하면 현재 파일을 정리한 후 종료합니다. 완료된 항목은 작업 기록에 남습니다."));alert.addButtonWithTitle(ns_string!("계속 작업"));alert.addButtonWithTitle(ns_string!("취소하고 종료"));if alert.runModal()==1001{for c in self.ivars().runtime.jobs.lock().unwrap().values(){c.cancel();}self.ivars().quit_when_idle.set(true);}NSApplicationTerminateReply::TerminateCancel
  }}
  #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]fn reopen(&self,_:&NSApplication,_:bool)->bool{self.reveal_window();false}
 }
 impl AppDelegate{
  #[unsafe(method(tick:))]fn tick(&self,_:&NSTimer){self.poll();}
  #[unsafe(method(volumesChanged:))]fn volumes_changed(&self,_:&NSNotification){self.ivars().runtime.volumes();}
  #[unsafe(method(newWindow:))]fn new_window(&self,_:Option<&AnyObject>){
      let app=NSApplication::sharedApplication(self.mtm());app.unhide(None);app.activate();
      self.open_window(vec![]);
  }
  #[unsafe(method(showNavigationGo:))]
  fn navigation_go(&self,_:Option<&AnyObject>){if let Some(c)=self.navigation_window(){c.go_to_folder_dialog();}}
  #[unsafe(method(showNavigationServer:))]
  fn navigation_server(&self,_:Option<&AnyObject>){if let Some(c)=self.navigation_window(){c.connect_server_dialog();}}
  #[unsafe(method(openNavigationFolder:))]
  fn navigation_open(&self,item:&NSMenuItem){
      if let Some(url)=item.representedObject().and_then(|object|object.downcast::<NSURL>().ok())
          && url.isFileURL() && let Some(c)=self.navigation_window(){c.request_folder(crate::platform::url_path(&url));}
  }
  #[unsafe(method(addNavigationFolder:))]
  fn navigation_add(&self,item:&NSMenuItem){
      if let Some(group)=crate::core::shortcuts::PinGroup::from_section(item.tag() as u64)
          && let Some(c)=self.navigation_window(){c.pick_pin_folders(group);}
  }
  #[unsafe(method(showExplorerWindow:))]fn show_window(&self,_:Option<&AnyObject>){self.reveal_window();}
  #[unsafe(method(settings:))]fn settings(&self,_:Option<&AnyObject>){if self.active_controller().is_none(){self.open_window(vec![]);}
  if let Some(c)=self.active_controller(){c.settings();}}
  #[unsafe(method(showFolderAccess:))]fn access_settings(&self,_:Option<&AnyObject>){if let Some(c)=self.navigation_window(){c.folder_access_settings();}}
 }
);
impl AppDelegate {
    fn sync_system_f2(&self, force: bool) {
        let enabled = self
            .ivars()
            .runtime
            .preferences
            .lock()
            .unwrap()
            .system_f2_rename;
        let active = NSApplication::sharedApplication(self.mtm()).isActive();
        self.ivars()
            .system_f2
            .borrow_mut()
            .sync(enabled, active, force);
    }
    fn active_controller(&self) -> Option<Retained<Controller>> {
        unsafe {
            let key = NSApplication::sharedApplication(self.mtm()).keyWindow();
            let windows = self.ivars().windows.borrow();
            let closed = self.ivars().closed.borrow();
            let available = |c: &&Retained<Controller>| !closed.contains(&c.id());
            windows
                .iter()
                .filter(available)
                .find(|c| key.as_ref().is_some_and(|w| **w == *c.views().window))
                .cloned()
                .or_else(|| {
                    windows
                        .iter()
                        .filter(available)
                        .find(|c| Some(c.id()) == self.ivars().last_active_window.get())
                        .cloned()
                })
                .or_else(|| windows.iter().rfind(available).cloned())
        }
    }
    fn reveal_window(&self) {
        unsafe {
            if self.active_controller().is_none() {
                self.open_window(vec![]);
            }
            if let Some(controller) = self.active_controller() {
                let window = &controller.views().window;
                let app = NSApplication::sharedApplication(self.mtm());
                app.unhide(None);
                if window.isMiniaturized() {
                    window.deminiaturize(None);
                }
                app.activate();
                window.makeMainWindow();
                window.makeKeyAndOrderFront(None);
                if let Some(sheet) = window.attachedSheet() {
                    sheet.makeKeyAndOrderFront(None);
                }
            }
        }
    }
    fn open_window(&self, paths: Vec<SavedTab>) {
        unsafe {
            let c = Controller::new(self.mtm(), self.ivars().runtime.clone(), paths);
            self.ivars().windows.borrow_mut().push(c);
            self.ivars().runtime.volumes();
        }
    }
    fn poll(&self) {
        self.sync_system_f2(false);
        let closed = std::mem::take(&mut *self.ivars().closed.borrow_mut());
        if !closed.is_empty() {
            self.ivars().windows.borrow_mut().retain(|c| {
                if closed.contains(&c.id()) {
                    unsafe {
                        c.views().window.setNextResponder(None);
                    }
                    false
                } else {
                    true
                }
            });
        }
        // Coalesce scan batches. Native views are reloaded at most once per delivery cycle.
        let mut batches: HashMap<(u64, u64, u64), Vec<Entry>> = HashMap::new();
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(8) {
            let Ok(event) = self.ivars().runtime.receiver.try_recv() else {
                break;
            };
            if let Event::Scan {
                window,
                tab,
                generation,
                update: ScanUpdate::Batch(entries),
            } = event
            {
                batches
                    .entry((window, tab, generation))
                    .or_default()
                    .extend(entries);
                continue;
            }
            if let Event::Scan {
                window,
                tab,
                generation,
                ..
            } = &event
            {
                batches.remove(&(*window, *tab, *generation));
            }
            self.route(event);
        }
        for ((window, tab, generation), batch) in batches {
            self.route(Event::Scan {
                window,
                tab,
                generation,
                update: ScanUpdate::Batch(batch),
            });
        }
        let windows = self.ivars().windows.borrow().clone();
        for c in windows {
            c.tick();
        }
        let quit = self
            .ivars()
            .quit_after
            .is_some_and(|d| self.ivars().startup.elapsed() > d)
            || self.ivars().quit_when_idle.get();
        if quit && self.ivars().runtime.jobs.lock().unwrap().is_empty() {
            unsafe {
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            }
        }
    }
    fn route(&self, event: Event) {
        let windows = self.ivars().windows.borrow().clone();
        match event {
            Event::Volumes(paths) => {
                *self.ivars().runtime.mounted_locations.lock().unwrap() = paths.clone();
                for c in windows {
                    c.rebuild_sidebar(paths.clone());
                }
            }
            Event::PreferencesChanged => {
                self.sync_system_f2(true);
                let prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
                let explain_access = self.active_controller().filter(|c| {
                    prefs.system_f2_rename
                        && !c.system_f2_enabled()
                        && self.ivars().system_f2.borrow().status()
                            == system_f2::Status::NeedsPermission
                });
                for c in windows {
                    c.apply_preferences(prefs.clone(), true);
                }
                if let Some(c) = explain_access {
                    c.explain_system_f2_access();
                }
            }
            Event::JobStarted { id, label } => {
                for c in windows {
                    c.job_started(id, &label);
                }
            }
            Event::JobProgress { id, progress } => {
                for c in windows {
                    c.job_progress(id, &progress);
                }
            }
            Event::JobDone { id, journal } => {
                for c in windows {
                    c.job_done(id, journal.clone());
                }
            }
            Event::Conflict {
                owner,
                conflict,
                reply,
            } => {
                if let Some(c) = windows
                    .iter()
                    .find(|c| c.id() == owner)
                    .or_else(|| windows.first())
                {
                    c.show_conflict(conflict, reply);
                } else {
                    let _ = reply.send(crate::core::operations::ConflictDecision {
                        choice: crate::core::operations::ConflictChoice::Cancel,
                        apply_all: false,
                    });
                }
            }
            Event::Recovered(journals) => {
                if let Some(c) = windows.first() {
                    c.message("중단된 작업 기록을 복구했습니다",&format!("{}개 작업이 중단된 상태입니다. 파일을 자동으로 다시 처리하지 않았습니다. 실행 취소 또는 파일을 확인한 후 재시도할 수 있습니다.",journals.len()));
                }
            }
            Event::UndoDone { id, owner, result } => {
                for c in &windows {
                    c.undo_done(id, result.as_ref().ok());
                }
                if let Err(error) = result
                    && let Some(c) = windows.iter().find(|c| c.id() == owner).or(windows.first())
                {
                    c.message("되돌릴 수 없습니다", &error);
                }
            }
            event => {
                let id = match &event {
                    Event::Scan { window, .. }
                    | Event::AccessChanged { window, .. }
                    | Event::RenamePreview { window, .. }
                    | Event::PinFolders { window, .. }
                    | Event::NetworkUnmounted { window, .. }
                    | Event::Applications { window, .. }
                    | Event::FolderReady { window, .. }
                    | Event::Metadata { window, .. }
                    | Event::Location { window, .. }
                    | Event::Thumbnail { window, .. }
                    | Event::Filter { window, .. }
                    | Event::Watch { window, .. }
                    | Event::Sidebar { window, .. }
                    | Event::Open { window, .. }
                    | Event::Message { window, .. } => *window,
                    _ => return,
                };
                if let Some(c) = windows
                    .iter()
                    .find(|c| c.id() == id)
                    .or_else(|| if id == 0 { windows.first() } else { None })
                {
                    c.handle_event(event);
                }
            }
        }
    }
    unsafe fn install_menu(&self) {
        let mtm = self.mtm();
        let main = NSMenu::new(mtm);
        let app_item = NSMenuItem::new(mtm);
        main.addItem(&app_item);
        let app_menu = NSMenu::new(mtm);
        app_menu.addItem(&widgets::item(
            "Rust Explorer 정보",
            sel!(orderFrontStandardAboutPanel:),
            None,
            "",
            mtm,
        ));
        app_menu.addItem(&widgets::item(
            "Rust Explorer 설정…",
            sel!(settings:),
            Some(self),
            ",",
            mtm,
        ));
        app_menu.addItem(&NSMenuItem::separatorItem(mtm));
        app_menu.addItem(&widgets::item(
            "Rust Explorer 종료",
            sel!(terminate:),
            None,
            "q",
            mtm,
        ));
        app_item.setSubmenu(Some(&app_menu));
        let file_item = NSMenuItem::new(mtm);
        file_item.setTitle(ns_string!("파일"));
        let file = NSMenu::new(mtm);
        file.addItem(&widgets::item(
            "새 창",
            sel!(newWindow:),
            Some(self),
            "n",
            mtm,
        ));
        file.addItem(&widgets::item("새 탭", sel!(newTab:), None, "t", mtm));
        file.addItem(&widgets::item(
            "폴더 접근 설정…",
            sel!(showFolderAccess:),
            Some(self),
            "",
            mtm,
        ));
        file.addItem(&widgets::creation_submenu(
            None,
            NSEventModifierFlags::Command,
            mtm,
        ));
        for (title, action, key) in [
            ("ZIP으로 압축…", sel!(compressZip:), ""),
            ("7z로 압축…", sel!(compress7z:), ""),
            ("압축 풀기", sel!(extractArchive:), ""),
            ("Bluetooth 파일 교환…", sel!(sendBluetooth:), ""),
            ("AirDrop으로 보내기…", sel!(sendAirDrop:), ""),
            ("열기", sel!(openSelected:), ""),
            ("연결 프로그램 선택…", sel!(openWith:), ""),
            ("복제", sel!(duplicateFiles:), "d"),
            ("이름 변경", sel!(renameFile:), "\u{f705}"),
            ("이름 일괄 변경…", sel!(batchRename:), ""),
            ("휴지통으로 이동", sel!(trashFiles:), ""),
        ] {
            let command = widgets::item(title, action, None, key, mtm);
            if action == sel!(renameFile:) {
                command.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
            }
            file.addItem(&command);
        }
        file_item.setSubmenu(Some(&file));
        let alternate_rename = widgets::item("이름 변경", sel!(renameFile:), None, "\r", mtm);
        alternate_rename.setHidden(true);
        alternate_rename.setAllowsKeyEquivalentWhenHidden(true);
        file.addItem(&alternate_rename);
        main.addItem(&file_item);
        let edit_item = NSMenuItem::new(mtm);
        edit_item.setTitle(ns_string!("편집"));
        let edit = NSMenu::new(mtm);
        for (title, action, key) in [
            ("실행 취소", sel!(undo:), "z"),
            ("잘라내기", sel!(cut:), "x"),
            ("복사", sel!(copy:), "c"),
            ("붙여넣기", sel!(paste:), "v"),
            ("모두 선택", sel!(selectAll:), "a"),
        ] {
            edit.addItem(&widgets::item(title, action, None, key, mtm));
        }
        edit.addItem(&widgets::item(
            "선택 해제",
            sel!(clearFileSelection:),
            None,
            "",
            mtm,
        ));
        edit.addItem(&widgets::item(
            "선택 반전",
            sel!(invertFileSelection:),
            None,
            "",
            mtm,
        ));
        edit_item.setSubmenu(Some(&edit));
        main.addItem(&edit_item);
        let go_item = NSMenuItem::new(mtm);
        go_item.setTitle(ns_string!("이동"));
        let go = NSMenu::new(mtm);
        let folder = widgets::item(
            "폴더로 이동…",
            sel!(showNavigationGo:),
            Some(self),
            "g",
            mtm,
        );
        folder.setKeyEquivalentModifierMask(
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift,
        );
        go.addItem(&folder);
        go.addItem(&widgets::item(
            "서버에 연결…",
            sel!(showNavigationServer:),
            Some(self),
            "k",
            mtm,
        ));
        go.addItem(&widgets::item(
            "서버 연결 취소",
            sel!(cancelServerConnection:),
            None,
            "",
            mtm,
        ));
        go.addItem(&NSMenuItem::separatorItem(mtm));
        for name in [
            navigation_menu::FAVORITES,
            navigation_menu::LOCATIONS,
            navigation_menu::RECENT,
        ] {
            go.addItem(&self.folder_submenu(name, true));
        }
        go_item.setSubmenu(Some(&go));
        main.addItem(&go_item);
        let view_item = NSMenuItem::new(mtm);
        view_item.setTitle(ns_string!("보기"));
        let view = NSMenu::new(mtm);
        for (title, action, key) in [
            ("목록 / 아이콘", sel!(toggleView:), ""),
            ("미리보기 패널", sel!(togglePreview:), "p"),
            ("숨김 파일 표시", sel!(toggleHidden:), ""),
            ("목록 간격 줄이기", sel!(toggleCompact:), ""),
            ("새로 고침", sel!(refresh:), "r"),
            ("작업 진행 표시", sel!(toggleJobs:), ""),
            ("작업 기록…", sel!(showHistory:), ""),
        ] {
            view.addItem(&widgets::item(title, action, None, key, mtm));
        }
        view_item.setSubmenu(Some(&view));
        main.addItem(&view_item);
        let window_item = NSMenuItem::new(mtm);
        window_item.setTitle(ns_string!("윈도우"));
        let window_menu = NSMenu::new(mtm);
        window_menu.addItem(&widgets::item(
            "탐색기 창 표시",
            sel!(showExplorerWindow:),
            Some(self),
            "",
            mtm,
        ));
        window_menu.addItem(&widgets::item(
            "최소화",
            sel!(performMiniaturize:),
            None,
            "m",
            mtm,
        ));
        window_menu.addItem(&widgets::item(
            "모두 앞으로 가져오기",
            sel!(arrangeInFront:),
            None,
            "",
            mtm,
        ));
        window_item.setSubmenu(Some(&window_menu));
        main.addItem(&window_item);
        let help_item = NSMenuItem::new(mtm);
        help_item.setTitle(ns_string!("도움말"));
        let help = NSMenu::new(mtm);
        help.addItem(&widgets::item(
            "키 입력 확인 켜기/끄기",
            sel!(toggleInputDiagnostics:),
            None,
            "",
            mtm,
        ));
        help_item.setSubmenu(Some(&help));
        main.addItem(&help_item);
        let app = NSApplication::sharedApplication(mtm);
        app.setMainMenu(Some(&main));
        app.setWindowsMenu(Some(&window_menu));
    }
}
pub fn new_window(path: Option<PathBuf>) {
    APPLICATION.with(|a| {
        if let Some(a) = a.borrow().as_ref() {
            a.open_window(
                path.map(|p| {
                    vec![SavedTab {
                        path: path_bytes(&p),
                        sort: SortField::Name,
                        descending: false,
                    }]
                })
                .unwrap_or_default(),
            );
        }
    });
}
pub fn close_controller(id: u64) {
    APPLICATION.with(|a| {
        if let Some(a) = a.borrow().as_ref() {
            a.ivars().closed.borrow_mut().push(id);
        }
    });
}
pub fn window_became_key(id: u64) {
    APPLICATION.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            app.ivars().last_active_window.set(Some(id));
        }
    });
}
pub fn dispatch_open() {
    APPLICATION.with(|a| {
        if let Some(c) = a.borrow().as_ref().and_then(|app| app.active_controller()) {
            c.open_selected();
        }
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemRenameTarget {
    window: u64,
    tab: u64,
    path: PathBuf,
}
fn is_system_rename_app_frontmost(mtm: MainThreadMarker) -> bool {
    unsafe {
        NSApplication::sharedApplication(mtm).isActive()
            && NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .is_some_and(|a| a.processIdentifier() as u32 == std::process::id())
    }
}
fn system_rename_target(mtm: MainThreadMarker) -> Option<SystemRenameTarget> {
    if !is_system_rename_app_frontmost(mtm) {
        return None;
    }
    APPLICATION.with(|a| {
        a.borrow()
            .as_ref()
            .and_then(|app| app.active_controller())
            .and_then(|c| c.system_rename_target())
    })
}
fn perform_system_rename(target: SystemRenameTarget) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if system_rename_target(mtm).as_ref() != Some(&target) {
        return;
    }
    let controller =
        APPLICATION.with(|a| a.borrow().as_ref().and_then(|app| app.active_controller()));
    if let Some(c) = controller {
        c.rename_from_system_key();
    }
}
fn record_system_rename_input(event: &NSEvent) {
    let controller =
        APPLICATION.with(|a| a.borrow().as_ref().and_then(|app| app.active_controller()));
    if let Some(c) = controller {
        c.record_system_rename_input(event);
    }
}
fn system_f2_status() -> system_f2::Status {
    APPLICATION.with(|a| {
        a.borrow()
            .as_ref()
            .map(|app| app.ivars().system_f2.borrow().status())
            .unwrap_or_default()
    })
}

pub fn run() {
    let startup = Instant::now();
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let mut args = std::env::args_os().skip(1);
    let mut path = None;
    let mut state_dir = std::env::var_os("RUST_EXPLORER_STATE_DIR").map(PathBuf::from);
    let mut quit_after = None;
    let appearance = std::env::var("RUST_EXPLORER_APPEARANCE").unwrap_or_default();
    let mut dark = appearance == "dark";
    let mut light = appearance == "light";
    let mut metrics_path = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--path") => path = args.next().map(PathBuf::from),
            Some("--state-dir") => state_dir = args.next().map(PathBuf::from),
            Some("--quit-after") => {
                quit_after = args
                    .next()
                    .and_then(|s| s.to_str().and_then(|s| s.parse::<u64>().ok()))
                    .map(Duration::from_secs)
            }
            Some("--dark") => {
                dark = true;
                light = false;
            }
            Some("--light") => {
                light = true;
                dark = false;
            }
            Some("--metrics") => metrics_path = args.next().map(PathBuf::from),
            _ => {}
        }
    }
    let data =
        state_dir.unwrap_or_else(|| home_dir().join("Library/Application Support/Rust Explorer"));
    let store = match Store::open(&data.join("state.sqlite")) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("설정 데이터베이스를 열 수 없습니다: {error}");
            return;
        }
    };
    let initial = path
        .map(|p| {
            vec![SavedTab {
                path: path_bytes(&p),
                sort: SortField::Name,
                descending: false,
            }]
        })
        .unwrap_or_else(|| store.get::<Vec<SavedTab>>("tabs").unwrap_or_default());
    let runtime = Runtime::new(store);
    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        // Establish the process as a regular GUI app before launching windows.
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let delegate = AppDelegate::alloc(mtm).set_ivars(AppIvars {
            runtime,
            windows: RefCell::new(vec![]),
            last_active_window: Cell::new(None),
            closed: RefCell::new(vec![]),
            timer: OnceCell::new(),
            monitor: OnceCell::new(),
            system_f2: RefCell::new(system_f2::Service::default()),
            metrics_path,
            metrics: RefCell::new(Metrics::default()),
            startup,
            initial,
            quit_after,
            quit_when_idle: Cell::new(false),
            dark,
            light,
        });
        let delegate: Retained<AppDelegate> = msg_send![super(delegate), init];
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        APPLICATION.with(|a| *a.borrow_mut() = Some(delegate));
        app.run();
    }
}

pub fn record_listing(ms: f64, rows: usize) {
    APPLICATION.with(|app| {
        if let Some(app) = app.borrow().as_ref()
            && app.ivars().metrics_path.is_some()
        {
            app.ivars()
                .metrics
                .borrow_mut()
                .listings
                .push(serde_json::json!({"first_display_ms":ms,"rows_available":rows}));
        }
    });
}

pub fn cancel_size_work() {
    APPLICATION.with(|app| {
        if let Some(app) = app.borrow().as_ref() {
            for window in app.ivars().windows.borrow().iter() {
                window.cancel_size();
            }
        }
    });
}

/// AppKit object tests: no windows are shown and no OS input is synthesized.
#[cfg(feature = "native-ui-tests")]
pub fn validate_native_selection() {
    selection::native_checks::run();
}

#[cfg(feature = "native-ui-tests")]
pub fn validate_native_drop_callbacks() {
    controller::native_drop_checks::run();
}

// Apply settings synchronously; persistence still uses the serial worker.
fn apply_live_preferences(runtime: &Arc<Runtime>, prefs: &Preferences) {
    let windows = APPLICATION.with(|app| {
        app.borrow()
            .as_ref()
            .map(|a| a.ivars().windows.borrow().clone())
            .unwrap_or_default()
    });
    #[cfg(feature = "native-ui-tests")]
    let windows = if windows.is_empty() {
        THEME_TEST_WINDOWS.with(|w| w.borrow().clone())
    } else {
        windows
    };
    for c in windows {
        if Arc::ptr_eq(&c.ivars().runtime, runtime) {
            c.apply_preferences(prefs.clone(), true);
        }
    }
}

#[cfg(feature = "native-ui-tests")]
pub fn validate_native_themes() {
    controller::native_theme_checks::run();
}

#[cfg(feature = "native-ui-tests")]
thread_local! {static THEME_TEST_WINDOWS:RefCell<Vec<Retained<Controller>>>=const{RefCell::new(Vec::new())};}
