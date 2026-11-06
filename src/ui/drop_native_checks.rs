//! Production controller -> native drop callbacks -> runtime queue -> filesystem.
//! Uses an unshown window and private pasteboard; never posts OS input or starts
//! a native drag session. Modifiers are injected, not read from the keyboard.
use super::*;
use crate::core::store::Store;
use objc2::runtime::{AnyClass, Bool};
use std::{cell::Cell, fs, os::unix::fs::MetadataExt, ptr::NonNull};

pub struct DropInfo {
    window: Retained<NSWindow>,
    board: Retained<NSPasteboard>,
    point: NSPoint,
    allowed: NSDragOperation,
    formation: Cell<NSDraggingFormation>,
    animate: Cell<bool>,
    count: Cell<isize>,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[name = "REDropProtocolTestInfo"]
    #[thread_kind=MainThreadOnly]
    #[ivars=DropInfo]
    struct TestDrop;
    unsafe impl NSObjectProtocol for TestDrop {}
    unsafe impl NSDraggingInfo for TestDrop {
        #[unsafe(method_id(draggingDestinationWindow))]
        fn window(&self) -> Option<Retained<NSWindow>> {
            Some(self.ivars().window.clone())
        }
        #[unsafe(method(draggingSourceOperationMask))]
        fn mask(&self) -> NSDragOperation {
            self.ivars().allowed
        }
        #[unsafe(method(draggingLocation))]
        fn location(&self) -> NSPoint {
            self.ivars().point
        }
        #[unsafe(method(draggedImageLocation))]
        fn image_location(&self) -> NSPoint {
            self.ivars().point
        }
        #[unsafe(method_id(draggedImage))]
        fn image(&self) -> Option<Retained<NSImage>> {
            None
        }
        #[unsafe(method_id(draggingPasteboard))]
        fn board(&self) -> Retained<NSPasteboard> {
            self.ivars().board.clone()
        }
        #[unsafe(method_id(draggingSource))]
        fn source(&self) -> Option<Retained<AnyObject>> {
            None
        }
        #[unsafe(method(draggingSequenceNumber))]
        fn sequence(&self) -> isize {
            1
        }
        #[unsafe(method(slideDraggedImageTo:))]
        fn slide(&self, _: NSPoint) {}
        #[unsafe(method_id(namesOfPromisedFilesDroppedAtDestination:))]
        fn promised(&self, _: &NSURL) -> Option<Retained<NSArray<NSString>>> {
            None
        }
        #[unsafe(method(draggingFormation))]
        fn formation(&self) -> NSDraggingFormation {
            self.ivars().formation.get()
        }
        #[unsafe(method(setDraggingFormation:))]
        fn set_formation(&self, v: NSDraggingFormation) {
            self.ivars().formation.set(v);
        }
        #[unsafe(method(animatesToDestination))]
        fn animate(&self) -> bool {
            self.ivars().animate.get()
        }
        #[unsafe(method(setAnimatesToDestination:))]
        fn set_animate(&self, v: bool) {
            self.ivars().animate.set(v);
        }
        #[unsafe(method(numberOfValidItemsForDrop))]
        fn count(&self) -> isize {
            self.ivars().count.get()
        }
        #[unsafe(method(setNumberOfValidItemsForDrop:))]
        fn set_count(&self, v: isize) {
            self.ivars().count.set(v);
        }
        #[unsafe(method(enumerateDraggingItemsWithOptions:forView:classes:searchOptions:usingBlock:))]
        unsafe fn enumerate(
            &self,
            _: NSDraggingItemEnumerationOptions,
            _: Option<&NSView>,
            _: &NSArray<AnyClass>,
            _: &NSDictionary<NSPasteboardReadingOptionKey, AnyObject>,
            block: &block2::DynBlock<dyn Fn(NonNull<NSDraggingItem>, isize, NonNull<Bool>)>,
        ) {
            if let Some(items) = self.ivars().board.pasteboardItems() {
                for (index, item) in items.iter().enumerate() {
                    let drag = NSDraggingItem::initWithPasteboardWriter(
                        NSDraggingItem::alloc(),
                        ProtocolObject::from_ref(&*item),
                    );
                    let mut stop = Bool::NO;
                    block.call((
                        NonNull::from(&*drag),
                        index as isize,
                        NonNull::from(&mut stop),
                    ));
                    if stop.as_bool() {
                        break;
                    }
                }
            }
        }
        #[unsafe(method(springLoadingHighlight))]
        fn spring(&self) -> NSSpringLoadingHighlight {
            NSSpringLoadingHighlight::None
        }
        #[unsafe(method(resetSpringLoading))]
        fn reset_spring(&self) {}
    }
);

#[derive(Default)]
struct Results {
    done: Vec<Journal>,
    undone: usize,
    conflicts: usize,
}
fn pump(
    c: &Controller,
    r: &Arc<Runtime>,
    results: &mut Results,
    choice: ConflictChoice,
    done: impl Fn(&Controller, &Results) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while let Ok(event) = r.receiver.try_recv() {
            match event {
                Event::JobStarted { id, label } => c.job_started(id, &label),
                Event::JobProgress { id, progress } => c.job_progress(id, &progress),
                Event::JobDone { id, journal } => {
                    assert_eq!(journal.status, "complete", "{:?}", journal.errors);
                    results.done.push(journal.clone());
                    c.job_done(id, journal);
                }
                Event::UndoDone { id, result, .. } => {
                    let journal = result.expect("undo succeeds");
                    results.undone += 1;
                    c.undo_done(id, Some(&journal));
                }
                Event::Conflict { reply, .. } => {
                    results.conflicts += 1;
                    reply
                        .send(ConflictDecision {
                            choice,
                            apply_all: false,
                        })
                        .unwrap();
                }
                Event::Message { title, detail, .. } => {
                    panic!("unexpected UI alert: {title}: {detail}")
                }
                Event::Open { .. }
                | Event::Applications { .. }
                | Event::NetworkUnmounted { .. }
                | Event::FolderReady { .. } => {
                    panic!("test must not open applications or connect devices")
                }
                Event::Volumes(_) | Event::PreferencesChanged | Event::Recovered(_) => {}
                other => c.handle_event(other),
            }
        }
        assert!(
            !c.views().window.isVisible(),
            "test window must stay hidden"
        );
        if done(c, results) {
            return;
        }
        assert!(Instant::now() < deadline, "controller pipeline timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn row(c: &Controller, name: &str) -> isize {
    (0..c.with_tab(|t| t.visible.len()))
        .find(|&i| c.entry_at(i as isize).is_some_and(|e| e.name == name))
        .expect(name) as isize
}
unsafe fn selected_board(c: &Controller) -> Retained<NSPasteboard> {
    let board = NSPasteboard::pasteboardWithUniqueName();
    let mut writers: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = vec![];
    let selection = c.views().table.selectedRowIndexes();
    let mut i = selection.firstIndex();
    while i != NSNotFound as usize {
        let writer: Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
            msg_send![c,tableView:&*c.views().table,pasteboardWriterForRow:i as isize];
        writers.push(writer.expect("file URL writer"));
        i = selection.indexGreaterThanIndex(i);
    }
    assert!(board.writeObjects(&NSArray::from_retained_slice(&writers)));
    let decoded = clipboard_paths(&board);
    let selected = c.selected_paths();
    assert_eq!(decoded.len(), selected.len());
    for (a, b) in decoded.iter().zip(selected) {
        let a = fs::symlink_metadata(a).unwrap();
        let b = fs::symlink_metadata(b).unwrap();
        assert_eq!((a.dev(), a.ino()), (b.dev(), b.ino()));
    }
    board
}

unsafe fn filename_layout(
    c: &Controller,
    runtime: &Arc<Runtime>,
    results: &mut Results,
    root: &Path,
) {
    let directory = root.join("filename-layout");
    fs::create_dir(&directory).unwrap();
    let names = [
        "가나다라마바사아자차카타파하한.txt",
        "1234567890123456789.txt",
        "2026년_최종_프로젝트_자료_아주_긴_파일명_확인_문서.txt",
        "가족👨‍👩‍👧‍👦_사진_원본_백업_검수.png",
    ];
    for name in names {
        fs::write(directory.join(name), b"filename layout fixture").unwrap();
    }
    c.navigate(directory, true);
    pump(c, runtime, results, ConflictChoice::Cancel, |c, _| {
        c.with_tab(|t| !t.loading && t.visible.len() == 4)
    });
    for (appearance, tag) in [
        (NSAppearanceNameAqua, "light"),
        (NSAppearanceNameDarkAqua, "dark"),
    ] {
        c.views()
            .window
            .setAppearance(NSAppearance::appearanceNamed(appearance).as_deref());
        c.views().apply_appearance();
        for size in [IconSize::Small, IconSize::Medium, IconSize::Large] {
            let mut prefs = c.ivars().state.borrow().prefs.clone();
            prefs.view = ViewMode::Icons;
            prefs.icon_size = size;
            c.apply_preferences(prefs, false);
            c.views().collection.layoutSubtreeIfNeeded();
            c.views()
                .collection
                .collectionViewLayout()
                .unwrap()
                .prepareLayout();
            for (n, name) in names.iter().enumerate() {
                let index = NSIndexPath::indexPathForItem_inSection(row(c, name), 0);
                let item = c
                    .views()
                    .collection
                    .itemAtIndexPath(&index)
                    .expect("visible filename item");
                let label = item.textField().unwrap();
                let image = item.imageView().unwrap();
                assert_eq!(label.stringValue().to_string(), *name);
                assert_eq!(label.toolTip().unwrap().to_string(), *name);
                assert_eq!(image.toolTip().unwrap().to_string(), *name);
                assert_eq!(item.view().toolTip().unwrap().to_string(), *name);
                assert!(!label.isSelectable() && !label.isEditable());
                assert!(!label.cell().unwrap().usesSingleLineMode());
                assert_eq!(image.frame().size.width, size.dimensions().2);
                assert!(
                    label.frame().origin.y + label.frame().size.height <= image.frame().origin.y
                );
                // Measure the complete text without the line limit; a limited
                // intrinsic size alone would hide premature truncation.
                label.setMaximumNumberOfLines(0);
                let full = label.sizeThatFits(NSSize::new(label.frame().size.width, 1000.));
                label.setMaximumNumberOfLines(2);
                if n < 2 {
                    assert!(
                        full.height <= label.frame().size.height,
                        "{name}: {:?} does not fit {:?}",
                        full,
                        label.frame()
                    );
                }
            }
            render_filename_view(c, &format!("icons-{}-{tag}", size.index()));
        }
        let mut prefs = c.ivars().state.borrow().prefs.clone();
        prefs.view = ViewMode::Details;
        c.apply_preferences(prefs, false);
        c.views().window.setContentSize(NSSize::new(760., 480.));
        c.views().layout();
        c.reload();
        let name_column = c.views().table.columnWithIdentifier(ns_string!("name"));
        let column = c
            .views()
            .table
            .tableColumns()
            .objectAtIndex(name_column as usize);
        assert!(column.width() >= 260.);
        for name in names {
            let view = c
                .views()
                .table
                .viewAtColumn_row_makeIfNecessary(name_column, row(c, name), true)
                .unwrap();
            let cell = view.downcast::<NSTableCellView>().unwrap();
            assert_eq!(
                cell.textField().unwrap().toolTip().unwrap().to_string(),
                name
            );
            assert_eq!(
                cell.imageView().unwrap().toolTip().unwrap().to_string(),
                name
            );
        }
        render_filename_view(c, &format!("details-narrow-{tag}"));
        c.views().window.setContentSize(NSSize::new(1040., 680.));
        c.views().layout();
    }
    assert!(!c.views().window.isVisible());
    println!(
        "ok: 15 Hangul characters plus extension fit all icon sizes; full filename tooltips, icon scale, narrow details and light/dark offscreen renders verified"
    );
}

unsafe fn render_filename_view(c: &Controller, name: &str) {
    if let Some(directory) = std::env::var_os("RUST_EXPLORER_NATIVE_RENDER_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        let root = &c.views().root;
        root.layoutSubtreeIfNeeded();
        let bitmap = root
            .bitmapImageRepForCachingDisplayInRect(root.bounds())
            .unwrap();
        root.cacheDisplayInRect_toBitmapImageRep(root.bounds(), &bitmap);
        let data = bitmap
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
            .unwrap();
        fs::write(directory.join(format!("{name}.png")), data.to_vec()).unwrap();
    }
}

unsafe fn folder_access_pipeline(
    c: &Controller,
    runtime: &Arc<Runtime>,
    results: &mut Results,
    root: &Path,
) {
    use std::os::unix::fs::PermissionsExt;
    let blocked = root.join("접근 제한 시험 폴더");
    fs::create_dir(&blocked).unwrap();
    struct Restore(PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        }
    }
    let _restore = Restore(blocked.clone());
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    c.navigate(blocked.clone(), true);
    pump(c, runtime, results, ConflictChoice::Cancel, |c, _| {
        c.with_tab(|t| !t.loading && t.error.is_some())
    });
    assert!(c.with_tab(|t| t.error.as_ref().unwrap().access_denied));
    assert!(!c.views().empty_actions.isHidden());
    assert!(!c.views().btn_access_current.isHidden());
    assert!(!c.views().btn_access_help.isHidden());
    assert!(
        c.views()
            .empty_label
            .toolTip()
            .unwrap()
            .to_string()
            .contains("error")
    );
    for size in [NSSize::new(760., 480.), NSSize::new(1040., 680.)] {
        c.views().window.setContentSize(size);
        c.views().layout();
        let bounds = c.views().empty_actions.bounds();
        for button in [
            &c.views().btn_access_current,
            &c.views().btn_access_help,
            &c.views().btn_error_retry,
            &c.views().btn_error_details,
        ] {
            let f = button.frame();
            assert!(
                f.origin.x >= 0.
                    && f.origin.y >= 0.
                    && f.origin.x + f.size.width <= bounds.size.width + 0.1
            );
            assert!(c.respondsToSelector(button.action().unwrap()));
        }
        render_filename_view(c, &format!("access-denied-{}", size.width as u32));
    }
    let picker = c.access_picker(None);
    assert!(picker.canChooseDirectories() && picker.allowsMultipleSelection());
    assert!(!picker.canChooseFiles());
    assert!(!picker.isVisible());
    let picker = c.access_picker(Some(&blocked));
    assert!(!picker.allowsMultipleSelection());
    assert!(!picker.isVisible());
    let alert = c.access_alert();
    alert.layout();
    let alert_window = alert.window();
    assert!(!alert_window.isVisible());
    assert!(
        alert_window.frame().size.height <= 480.,
        "access sheet must fit the minimum window height"
    );
    // NSAlert uses a vibrancy layer that cannot be cached faithfully offscreen.
    // Render its real accessory controls in our hidden, non-vibrant host.
    let form = alert.accessoryView().unwrap();
    alert.setAccessoryView(None);
    form.setFrameOrigin(NSPoint::new(230., 100.));
    form.setWantsLayer(true);
    form.layer()
        .unwrap()
        .setBackgroundColor(Some(&NSColor::windowBackgroundColor().CGColor()));
    c.views().root.addSubview(&form);
    c.views().root.layoutSubtreeIfNeeded();
    if let Some(directory) = std::env::var_os("RUST_EXPLORER_NATIVE_RENDER_DIR") {
        let bitmap = form
            .bitmapImageRepForCachingDisplayInRect(form.bounds())
            .unwrap();
        form.cacheDisplayInRect_toBitmapImageRep(form.bounds(), &bitmap);
        let png = bitmap
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
            .unwrap();
        fs::write(
            PathBuf::from(directory).join("access-settings.png"),
            png.to_vec(),
        )
        .unwrap();
    }
    form.removeFromSuperview();
    c.ivars().state.borrow_mut().access_list = None;
    println!(
        "ok: real permission failure shows folder chooser/settings/retry/details, fits narrow windows, and picker configuration is non-displaying"
    );
    let origin = c.with_tab(|t| (t.id, t.path.clone()));
    c.ivars().state.borrow_mut().access_return = Some(origin.clone());
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
    c.resume_folder_access();
    pump(c, runtime, results, ConflictChoice::Cancel, |c, _| {
        c.with_tab(|t| !t.loading && t.error.is_none())
    });
    assert!(c.views().empty_actions.isHidden());
    let missing = root.join("does-not-exist");
    c.navigate(missing, true);
    pump(c, runtime, results, ConflictChoice::Cancel, |c, _| {
        c.with_tab(|t| !t.loading && t.error.is_some())
    });
    assert!(!c.with_tab(|t| t.error.as_ref().unwrap().access_denied));
    assert!(c.views().btn_access_current.isHidden());
    assert!(c.views().btn_access_help.isHidden());
    assert!(!c.views().empty_actions.isHidden());
    let generation = c.with_tab(|t| t.generation);
    c.ivars().state.borrow_mut().access_return = Some(origin.clone());
    c.resume_folder_access();
    assert_eq!(c.with_tab(|t| t.generation), generation);
    println!(
        "ok: returning from privacy settings retries the same denied location only; missing/disconnected paths are not mislabelled as TCC denial"
    );
    let current = c.current_path();
    runtime
        .remember_access(
            c.id(),
            Some(origin),
            vec![platform::file_url(&blocked)],
            false,
        )
        .unwrap();
    assert!(runtime.persistence.flush(Duration::from_secs(2)));
    pump(c, runtime, results, ConflictChoice::Cancel, |c, _| {
        c.ivars().state.borrow().prefs.access_intro_dismissed
    });
    assert_eq!(c.current_path(), current);
    assert!(c.views().access_banner.isHidden());
    let saved: Vec<platform::folder_access::SavedFolder> =
        runtime.store.get("folder_access").unwrap();
    assert_eq!(saved.len(), 1);
    let reopened = Runtime::new(runtime.store.clone());
    assert!(reopened.persistence.flush(Duration::from_secs(2)));
    assert_eq!(reopened.folder_access.lock().unwrap().paths().len(), 1);
    runtime.forget_access(c.id()).unwrap();
    assert!(runtime.persistence.flush(Duration::from_secs(2)));
    assert!(
        runtime
            .store
            .get::<Vec<platform::folder_access::SavedFolder>>("folder_access")
            .unwrap()
            .is_empty()
    );
    assert!(!c.views().window.isVisible());
    println!(
        "ok: explicit folder selections persist and restore, cancel stale navigation, dismiss onboarding once, and can be forgotten without altering system permissions"
    );
}
unsafe fn info(
    c: &Controller,
    board: Retained<NSPasteboard>,
    allowed: NSDragOperation,
    point: NSPoint,
) -> Retained<TestDrop> {
    let value = TestDrop::alloc(c.mtm()).set_ivars(DropInfo {
        window: c.views().window.clone(),
        count: Cell::new(board.pasteboardItems().map_or(0, |i| i.count() as isize)),
        board,
        point,
        allowed,
        formation: Cell::new(NSDraggingFormation::Default),
        animate: Cell::new(false),
    });
    msg_send![super(value), init]
}
unsafe fn select(c: &Controller, rows: &[isize]) {
    let indices = NSMutableIndexSet::new();
    for i in rows {
        indices.addIndex(*i as usize);
    }
    c.views()
        .table
        .selectRowIndexes_byExtendingSelection(&indices, false);
    c.selection_changed();
}
unsafe fn table_drop(
    c: &Controller,
    board: Retained<NSPasteboard>,
    folder: isize,
    allowed: NSDragOperation,
) -> NSDragOperation {
    let frame = c.views().table.rectOfRow(folder.max(0));
    let point = c
        .views()
        .table
        .convertPoint_toView(NSPoint::new(40., frame.origin.y + 15.), None);
    let drop = info(c, board, allowed, point);
    let protocol: &ProtocolObject<dyn NSDraggingInfo> = ProtocolObject::from_ref(&*drop);
    let operation: NSDragOperation = msg_send![c,tableView:&*c.views().table,validateDrop:protocol,proposedRow:folder,proposedDropOperation:NSTableViewDropOperation::On];
    if operation != NSDragOperation::None {
        let accepted: bool = msg_send![c,tableView:&*c.views().table,acceptDrop:protocol,row:folder,dropOperation:NSTableViewDropOperation::On];
        assert!(accepted);
    }
    operation
}

pub fn run() {
    objc2::rc::autoreleasepool(|_| unsafe {
        let mtm = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        assert_eq!(
            app.activationPolicy(),
            NSApplicationActivationPolicy::Prohibited
        );
        let root = std::env::temp_dir().join(format!(
            "RustExplorer-drop-pipeline-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let files = root.join("files");
        fs::create_dir(&files).unwrap();
        let target = files.join("A 받는 폴더");
        fs::create_dir(&target).unwrap();
        let nested = files.join("B 선택 폴더");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("inside"), b"nested").unwrap();
        for i in 1..=3 {
            fs::write(
                files.join(format!("0{i} 시험.txt")),
                format!("original {i}"),
            )
            .unwrap();
        }
        let store = Store::open(&root.join("state/db")).unwrap();
        let runtime = Runtime::new(store);
        let c = Controller::new_hidden(
            mtm,
            runtime.clone(),
            vec![
                SavedTab {
                    path: b"/invalid\0tab".to_vec(),
                    sort: SortField::Name,
                    descending: false,
                },
                SavedTab {
                    path: path_bytes(&files),
                    sort: SortField::Name,
                    descending: false,
                },
            ],
        );
        c.ivars().state.borrow_mut().test_drag_modifiers = Some(NSEventModifierFlags::empty());
        let mut results = Results::default();
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, _| c.with_tab(|t| !t.loading && t.visible.len() == 5),
        );
        assert_eq!(c.ivars().state.borrow().tabs.len(), 1);
        c.views()
            .address
            .setStringValue(&NSString::from_str("/invalid\0address"));
        let _: () = msg_send![&*c,addressEntered:None::<&AnyObject>];
        assert_eq!(c.current_path(), files);
        assert!(c.views().window.attachedSheet().is_none());
        println!(
            "ok: corrupt restored tabs and NUL address input cannot panic or replace the current folder"
        );
        let folder = row(&c, "A 받는 폴더");
        let first = row(&c, "01 시험.txt");
        let second = row(&c, "02 시험.txt");
        select(&c, &[first]);
        let table = c.views().table.downcast_ref::<FileTableView>().unwrap();
        let frame = table.rectOfRow(second);
        let event=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(NSEventType::LeftMouseDown,table.convertPoint_toView(NSPoint::new(40.,frame.origin.y+15.),None),NSEventModifierFlags::empty(),0.,c.views().window.windowNumber(),None,1,1,1.).unwrap();
        assert!(!super::super::selection::Surface::Table(table).down(&event));
        assert_eq!(c.selected_paths(), vec![files.join("02 시험.txt")]);
        assert_eq!(
            table_drop(
                &c,
                selected_board(&c),
                folder,
                NSDragOperation::Copy | NSDragOperation::Move
            ),
            NSDragOperation::Move
        );
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.done.len() == 1 && c.with_tab(|t| !t.loading),
        );
        assert!(!files.join("02 시험.txt").exists());
        assert_eq!(fs::read(target.join("02 시험.txt")).unwrap(), b"original 2");
        assert_eq!(fs::read(files.join("01 시험.txt")).unwrap(), b"original 1");
        assert_eq!(results.done[0].request.kind, OperationKind::Drop);
        println!(
            "ok: real controller selects file 2, writes its URL, accepts folder drop and moves only file 2"
        );
        runtime.undo(c.id());
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.undone == 1 && c.with_tab(|t| !t.loading),
        );
        assert_eq!(fs::read(files.join("02 시험.txt")).unwrap(), b"original 2");
        assert!(!target.join("02 시험.txt").exists());
        println!("ok: production undo queue restores the moved file and refreshes the listing");
        select(&c, &[row(&c, "01 시험.txt"), row(&c, "B 선택 폴더")]);
        assert_eq!(
            table_drop(
                &c,
                selected_board(&c),
                row(&c, "A 받는 폴더"),
                NSDragOperation::Copy | NSDragOperation::Move
            ),
            NSDragOperation::Move
        );
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.done.len() == 2 && c.with_tab(|t| !t.loading),
        );
        assert!(!nested.exists() && !files.join("01 시험.txt").exists());
        assert_eq!(
            fs::read(target.join("B 선택 폴더/inside")).unwrap(),
            b"nested"
        );
        runtime.undo(c.id());
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.undone == 2 && c.with_tab(|t| !t.loading),
        );
        assert_eq!(fs::read(nested.join("inside")).unwrap(), b"nested");
        println!("ok: mixed file/folder multi-drop moves and restores all selected sources");
        select(&c, &[row(&c, "02 시험.txt")]);
        let count = results.done.len();
        assert_eq!(
            table_drop(
                &c,
                selected_board(&c),
                -1,
                NSDragOperation::Copy | NSDragOperation::Move
            ),
            NSDragOperation::None
        );
        assert!(runtime.jobs.lock().unwrap().is_empty());
        assert_eq!(results.done.len(), count);
        println!("ok: dropping back into the current folder is a no-op, not a failed copy");
        for (mask, modifiers) in [
            (NSDragOperation::None, NSEventModifierFlags::empty()),
            (NSDragOperation::Link, NSEventModifierFlags::empty()),
            (NSDragOperation::Move, NSEventModifierFlags::Option),
        ] {
            c.ivars().state.borrow_mut().test_drag_modifiers = Some(modifiers);
            let drop = info(&c, selected_board(&c), mask, NSPoint::ZERO);
            let protocol: &ProtocolObject<dyn NSDraggingInfo> = ProtocolObject::from_ref(&*drop);
            let folder = row(&c, "A 받는 폴더");
            let operation: NSDragOperation = msg_send![&*c,tableView:&*c.views().table,validateDrop:protocol,proposedRow:folder,proposedDropOperation:NSTableViewDropOperation::On];
            assert_eq!(
                operation,
                NSDragOperation::None,
                "source did not allow this operation"
            );
            let accepted: bool = msg_send![&*c,tableView:&*c.views().table,acceptDrop:protocol,row:folder,dropOperation:NSTableViewDropOperation::On];
            assert!(!accepted);
            assert!(runtime.jobs.lock().unwrap().is_empty());
        }
        c.ivars().state.borrow_mut().test_drag_modifiers = Some(NSEventModifierFlags::empty());
        println!(
            "ok: unsupported source masks and a copy modifier on move-only data cannot enqueue a file operation"
        );
        let move_only = info(&c, selected_board(&c), NSDragOperation::Move, NSPoint::ZERO);
        let request = c
            .drop_request(ProtocolObject::from_ref(&*move_only), &target)
            .unwrap();
        assert_eq!(request.kind, OperationKind::Move);
        fs::write(target.join("02 시험.txt"), b"existing").unwrap();
        assert_eq!(
            table_drop(
                &c,
                selected_board(&c),
                row(&c, "A 받는 폴더"),
                NSDragOperation::Copy | NSDragOperation::Move
            ),
            NSDragOperation::Move
        );
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::KeepBoth,
            |c, r| r.done.len() == 3 && c.with_tab(|t| !t.loading),
        );
        assert_eq!(results.conflicts, 1);
        assert_eq!(fs::read(target.join("02 시험.txt")).unwrap(), b"existing");
        assert_eq!(
            fs::read(target.join("02 시험 (2).txt")).unwrap(),
            b"original 2"
        );
        runtime.undo(c.id());
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.undone == 3 && c.with_tab(|t| !t.loading),
        );
        println!(
            "ok: drop collision travels through the conflict channel; keep-both and undo preserve existing data"
        );
        let mut prefs = c.ivars().state.borrow().prefs.clone();
        prefs.view = ViewMode::Icons;
        c.apply_preferences(prefs, false);
        let source_index = row(&c, "03 시험.txt");
        let target_index = row(&c, "A 받는 폴더");
        let source_path = NSIndexPath::indexPathForItem_inSection(source_index, 0);
        let destination_path = NSIndexPath::indexPathForItem_inSection(target_index, 0);
        c.views()
            .collection
            .setSelectionIndexPaths(&NSSet::from_slice(&[&*source_path]));
        c.selection_changed();
        let writer: Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = msg_send![&*c,collectionView:&*c.views().collection,pasteboardWriterForItemAtIndexPath:&*source_path];
        let board = NSPasteboard::pasteboardWithUniqueName();
        assert!(board.writeObjects(&NSArray::from_slice(&[&*writer.unwrap()])));
        c.views().collection.layoutSubtreeIfNeeded();
        let layout = c.views().collection.collectionViewLayout().unwrap();
        layout.prepareLayout();
        let frame = layout
            .layoutAttributesForItemAtIndexPath(&destination_path)
            .unwrap()
            .frame();
        let point = c.views().collection.convertPoint_toView(
            NSPoint::new(
                frame.origin.x + frame.size.width / 2.,
                frame.origin.y + frame.size.height / 2.,
            ),
            None,
        );
        let drop = info(&c, board, NSDragOperation::Copy, point);
        let protocol: &ProtocolObject<dyn NSDraggingInfo> = ProtocolObject::from_ref(&*drop);
        let mut proposed = Retained::as_ptr(&destination_path).cast_mut();
        let mut operation = NSCollectionViewDropOperation::Before;
        let allowed: NSDragOperation = msg_send![&*c,collectionView:&*c.views().collection,validateDrop:protocol,proposedIndexPath:NonNull::from(&mut proposed),dropOperation:NonNull::from(&mut operation)];
        assert_eq!(allowed, NSDragOperation::Copy);
        assert_eq!(operation, NSCollectionViewDropOperation::On);
        let accepted: bool = msg_send![&*c,collectionView:&*c.views().collection,acceptDrop:protocol,indexPath:&*proposed,dropOperation:operation];
        assert!(accepted);
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.done.len() == 4 && c.with_tab(|t| !t.loading),
        );
        assert_eq!(fs::read(target.join("03 시험.txt")).unwrap(), b"original 3");
        assert_eq!(fs::read(files.join("03 시험.txt")).unwrap(), b"original 3");
        println!(
            "ok: icon drop hit-testing targets the folder and honors a copy-only external source"
        );
        let mut prefs = c.ivars().state.borrow().prefs.clone();
        prefs.view = ViewMode::Details;
        c.apply_preferences(prefs, false);
        select(&c, &[row(&c, "03 시험.txt")]);
        c.ivars().state.borrow_mut().test_drag_modifiers = Some(NSEventModifierFlags::Control);
        assert_eq!(
            table_drop(
                &c,
                selected_board(&c),
                -1,
                NSDragOperation::Copy | NSDragOperation::Move
            ),
            NSDragOperation::Copy
        );
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.done.len() == 5 && c.with_tab(|t| !t.loading),
        );
        assert_eq!(
            fs::read(files.join("03 시험 (2).txt")).unwrap(),
            b"original 3"
        );
        runtime.undo(c.id());
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.undone == 4 && c.with_tab(|t| !t.loading),
        );
        assert!(!files.join("03 시험 (2).txt").exists());
        assert!(files.join("03 시험.txt").exists());
        c.ivars().state.borrow_mut().test_drag_modifiers = Some(NSEventModifierFlags::empty());
        println!(
            "ok: injected Windows Ctrl-drop duplicates in place and undo preserves its source"
        );
        select(&c, &[row(&c, "B 선택 폴더")]);
        assert_eq!(
            table_drop(
                &c,
                selected_board(&c),
                row(&c, "B 선택 폴더"),
                NSDragOperation::Copy | NSDragOperation::Move
            ),
            NSDragOperation::None
        );
        assert!(runtime.jobs.lock().unwrap().is_empty());
        println!("ok: dropping a folder onto itself is rejected without a job");
        select(&c, &[row(&c, "02 시험.txt")]);
        let selected = c.selected_paths();
        let old_row = c.views().table.selectedRow();
        c.ivars().state.borrow_mut().dragging = true;
        fs::write(files.join("00 arrival.txt"), b"arrived during drag").unwrap();
        c.load(true);
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, _| !c.ivars().state.borrow().selection_events.is_empty(),
        );
        assert_eq!(c.views().table.selectedRow(), old_row);
        assert_eq!(c.selected_paths(), selected);
        c.drag_ended();
        c.tick();
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, _| c.with_tab(|t| !t.loading),
        );
        assert_eq!(c.selected_paths(), selected);
        assert_eq!(c.views().table.selectedRow(), row(&c, "02 시험.txt"));
        println!("ok: a completed background refresh cannot retarget the dragged selection");
        let malformed = NSPasteboard::pasteboardWithUniqueName();
        let item = NSPasteboardItem::new();
        item.setString_forType(
            ns_string!("file:///tmp/legitimate%00different"),
            NSPasteboardTypeFileURL,
        );
        assert!(
            malformed.writeObjects(&NSArray::from_slice(&[ProtocolObject::<
                dyn NSPasteboardWriting,
            >::from_ref(&*item)]))
        );
        assert_eq!(
            table_drop(
                &c,
                malformed,
                row(&c, "A 받는 폴더"),
                NSDragOperation::Copy | NSDragOperation::Move
            ),
            NSDragOperation::None
        );
        assert!(runtime.jobs.lock().unwrap().is_empty());
        println!(
            "ok: malformed external file URL is rejected by the real drop callback without creating a job"
        );
        assert!(
            c.views().jobs.isHidden(),
            "successful jobs need no completion dialog"
        );
        let selected = c.selected_paths();
        for size in [IconSize::Small, IconSize::Medium, IconSize::Large] {
            let mut prefs = c.ivars().state.borrow().prefs.clone();
            prefs.view = ViewMode::Icons;
            prefs.icon_size = size;
            c.apply_preferences(prefs, false);
            assert_eq!(c.selected_paths(), selected);
        }
        let mut prefs = c.ivars().state.borrow().prefs.clone();
        prefs.view = ViewMode::Details;
        c.apply_preferences(prefs, false);
        assert_eq!(c.selected_paths(), selected);
        println!(
            "ok: selection survives all icon sizes and return to details; successful jobs leave no blocking completion UI"
        );
        let base = c.with_tab(|t| t.id);
        c.new_tab(target.clone());
        let middle = c.with_tab(|t| t.id);
        c.new_tab(nested.clone());
        let (last, generation) = c.with_tab(|t| (t.id, t.generation));
        let close = NSButton::new(mtm);
        close.setTag(middle as isize);
        let _: () = msg_send![&*c, closeTabAt:&*close];
        assert_eq!(c.with_tab(|t| t.id), last);
        assert_eq!(c.ivars().state.borrow().tabs.len(), 2);
        // A stale button action cannot close whichever tab has taken its index.
        let _: () = msg_send![&*c, closeTabAt:&*close];
        assert_eq!(c.ivars().state.borrow().tabs.len(), 2);
        close.setTag(last as isize);
        let _: () = msg_send![&*c, closeTabAt:&*close];
        assert_eq!(c.with_tab(|t| t.id), base);
        assert_eq!(c.current_path(), files);
        c.handle_event(Event::Scan {
            window: c.id(),
            tab: last,
            generation,
            update: ScanUpdate::Complete {
                entries: vec![],
                skipped: 0,
            },
        });
        assert_eq!(c.with_tab(|t| t.id), base);
        assert_eq!(c.selected_paths(), selected);
        assert!(row(&c, "02 시험.txt") >= 0);
        println!(
            "ok: native tab X actions close active/background tabs by ID and ignore late results for closed tabs"
        );
        filename_layout(&c, &runtime, &mut results, &root);
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.setString_forType(ns_string!("unrelated cut payload"), NSPasteboardTypeString);
        let clipboard_count = board.changeCount();
        c.ivars().state.borrow_mut().last_clipboard = Some(clipboard_count);
        let mut unrelated = results.done[0].clone();
        unrelated.request.kind = OperationKind::Move;
        c.finish_paste_clipboard(9001, &unrelated, &board);
        assert_eq!(
            board
                .stringForType(NSPasteboardTypeString)
                .map(|s| s.to_string()),
            Some("unrelated cut payload".into()),
            "an unrelated move must not clear pending cut data"
        );
        c.ivars()
            .state
            .borrow_mut()
            .paste_moves
            .insert(9002, clipboard_count);
        board.clearContents(); // A new copy operation takes pasteboard ownership.
        board.setString_forType(ns_string!("new clipboard data"), NSPasteboardTypeString);
        c.finish_paste_clipboard(9002, &unrelated, &board);
        assert_eq!(
            board
                .stringForType(NSPasteboardTypeString)
                .unwrap()
                .to_string(),
            "new clipboard data"
        );
        c.ivars()
            .state
            .borrow_mut()
            .paste_moves
            .insert(9003, board.changeCount());
        unrelated.status = "failed".into();
        c.finish_paste_clipboard(9003, &unrelated, &board);
        assert!(board.stringForType(NSPasteboardTypeString).is_some());
        println!(
            "ok: unrelated moves, changed clipboard contents and failed paste jobs preserve pending cut data on a private pasteboard"
        );
        let cut_source = root.join("cut-source.txt");
        fs::write(&cut_source, b"move only on paste").unwrap();
        let item = NSPasteboardItem::new();
        item.setString_forType(
            &platform::file_url(&cut_source).absoluteString().unwrap(),
            NSPasteboardTypeFileURL,
        );
        item.setString_forType(ns_string!("1"), ns_string!("local.rust-explorer.cut"));
        board.clearContents();
        assert!(board.writeObjects(&NSArray::from_slice(&[ProtocolObject::<
            dyn NSPasteboardWriting,
        >::from_ref(&*item)])));
        c.ivars().state.borrow_mut().test_clipboard = Some(board.clone());
        let done = results.done.len();
        let destination = c.current_path();
        c.paste(false);
        assert_eq!(c.ivars().state.borrow().paste_moves.len(), 1);
        pump(
            &c,
            &runtime,
            &mut results,
            ConflictChoice::Cancel,
            |c, r| r.done.len() == done + 1 && c.with_tab(|t| !t.loading),
        );
        assert!(!cut_source.exists());
        assert_eq!(
            fs::read(destination.join("cut-source.txt")).unwrap(),
            b"move only on paste"
        );
        assert!(board.pasteboardItems().is_none_or(|items| items.is_empty()));
        assert!(c.ivars().state.borrow().paste_moves.is_empty());
        println!(
            "ok: a real cut-paste move clears only its own unchanged private pasteboard after successful completion"
        );
        folder_access_pipeline(&c, &runtime, &mut results, &root);
        assert!(!c.views().window.isVisible());
        runtime.unwatch(c.id(), None);
        for tab in &c.ivars().state.borrow().tabs {
            tab.cancel.cancel();
            tab.filter_cancel.cancel();
        }
        c.views().window.setDelegate(None);
        c.views().window.setNextResponder(None);
        assert!(runtime.persistence.flush(Duration::from_secs(2)));
        std::mem::drop(c);
        std::mem::drop(runtime);
        fs::remove_dir_all(&root).unwrap();
        println!(
            "20 production drop/undo/security/filename/access pipeline scenarios passed without keyboard or mouse input"
        );
    });
}
