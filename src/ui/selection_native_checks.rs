//! Tests AppKit objects in an unshown window. No event is posted to the OS or
//! application queue, and no native drag session is started.
use super::*;
use crate::ui::widgets::{FileItem, cell};
use objc2::{
    ClassType, MainThreadMarker,
    runtime::{NSObjectProtocol, ProtocolObject},
};
use std::cell::{Cell, OnceCell};

#[derive(Default)]
pub struct ObserverState {
    table: OnceCell<Retained<FileTableView>>,
    grid: OnceCell<Retained<FileCollectionView>>,
    use_grid: Cell<bool>,
    model: RefCell<BTreeSet<usize>>,
    tracking: Cell<bool>,
}

define_class!(
    #[unsafe(super=NSObject)]
    #[name="RESelectionTestObserver"]
    #[thread_kind=MainThreadOnly]
    #[ivars=ObserverState]
    struct Observer;
    unsafe impl NSObjectProtocol for Observer {}
    unsafe impl NSWindowDelegate for Observer {}
    unsafe impl NSTableViewDataSource for Observer {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self,_:&NSTableView)->isize {6}
    }
    unsafe impl NSControlTextEditingDelegate for Observer {}
    unsafe impl NSTableViewDelegate for Observer {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn row(&self,table:&NSTableView,_:Option<&NSTableColumn>,row:isize)->Option<Retained<NSView>> {
            unsafe {
                let view=cell(self.mtm(),table,"name",true);
                view.textField().unwrap().setStringValue(&NSString::from_str(&format!("{row:02} 시험.txt")));
                Some(Retained::into_super(view))
            }
        }
    }
    unsafe impl NSCollectionViewDataSource for Observer {
        #[unsafe(method(collectionView:numberOfItemsInSection:))]
        fn items(&self,_:&NSCollectionView,_:isize)->isize {6}
        #[unsafe(method_id(collectionView:itemForRepresentedObjectAtIndexPath:))]
        fn item(&self,grid:&NSCollectionView,index:&NSIndexPath)->Retained<NSCollectionViewItem> {
            let item=grid.makeItemWithIdentifier_forIndexPath(ns_string!("test-item"),index);
            let _=item.view();
            if let Some(label)=item.textField(){label.setStringValue(&NSString::from_str(&format!("{} 시험",index.item())));}
            item
        }
    }
    unsafe impl NSCollectionViewDelegate for Observer {}
    impl Observer {
        #[unsafe(method(fileSelectionChanged))]
        fn changed(&self) {
            let surface=if self.ivars().use_grid.get(){Surface::Grid(self.ivars().grid.get().unwrap())}else{Surface::Table(self.ivars().table.get().unwrap())};
            *self.ivars().model.borrow_mut()=surface.selected();
        }
        #[unsafe(method(fileSelectionTracking:))]
        fn tracking(&self,active:bool){self.ivars().tracking.set(active);}
    }
);

fn event(
    view: &NSView,
    point: NSPoint,
    kind: NSEventType,
    flags: NSEventModifierFlags,
) -> Retained<NSEvent> {
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,view.convertPoint_toView(point,None),flags,0.,view.window().unwrap().windowNumber(),None,1,1,1.,
    ).unwrap()
}

pub fn run() {
    objc2::rc::autoreleasepool(|_| unsafe {
        let mtm = MainThreadMarker::new().expect("native selection tests run on main thread");
        let app = NSApplication::sharedApplication(mtm);
        let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        assert_eq!(
            app.activationPolicy(),
            NSApplicationActivationPolicy::Prohibited
        );
        let observer = Observer::alloc(mtm).set_ivars(ObserverState::default());
        let observer: Retained<Observer> = msg_send![super(observer), init];
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0., 0., 1020., 320.),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            true,
        );
        window.setReleasedWhenClosed(false);
        window.setDelegate(Some(ProtocolObject::from_ref(&*observer)));
        let root = NSView::initWithFrame(NSView::alloc(mtm), rect(0., 0., 1020., 320.));
        window.setContentView(Some(&root));
        let table_scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0., 0., 500., 300.));
        let clip: Retained<FileClipView> =
            msg_send![FileClipView::alloc(mtm),initWithFrame:rect(0.,0.,500.,300.)];
        table_scroll.setContentView(&clip);
        root.addSubview(&table_scroll);
        let table = FileTableView::new(mtm, rect(0., 0., 500., 300.));
        table.setAllowsMultipleSelection(true);
        table.setAllowsEmptySelection(true);
        table.setRowHeight(30.);
        let column =
            NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("name"));
        column.setWidth(480.);
        table.addTableColumn(&column);
        table.setDataSource(Some(ProtocolObject::from_ref(&*observer)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*observer)));
        table_scroll.setDocumentView(Some(&table));
        observer.ivars().table.set(table.clone()).ok().unwrap();
        let grid_scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(520., 0., 500., 300.));
        root.addSubview(&grid_scroll);
        let grid = FileCollectionView::new(mtm, rect(0., 0., 500., 300.));
        grid.setSelectable(true);
        grid.setAllowsMultipleSelection(true);
        grid.setAllowsEmptySelection(true);
        let layout = NSCollectionViewFlowLayout::new(mtm);
        layout.setItemSize(NSSize::new(100., 104.));
        layout.setMinimumInteritemSpacing(12.);
        layout.setMinimumLineSpacing(12.);
        grid.setCollectionViewLayout(Some(&layout));
        grid.registerClass_forItemWithIdentifier(Some(FileItem::class()), ns_string!("test-item"));
        grid.setDataSource(Some(ProtocolObject::from_ref(&*observer)));
        grid.setDelegate(Some(ProtocolObject::from_ref(&*observer)));
        grid_scroll.setDocumentView(Some(&grid));
        observer.ivars().grid.set(grid.clone()).ok().unwrap();
        table.reloadData();
        grid.reloadData();
        root.layoutSubtreeIfNeeded();
        table_scroll.tile();
        grid_scroll.tile();
        grid.layoutSubtreeIfNeeded();
        layout.prepareLayout();
        for row in 0..6 {
            let _ = table.viewAtColumn_row_makeIfNecessary(0, row, true);
        }
        let row_point = |row, x| {
            let frame = table.rectOfRow(row);
            NSPoint::new(x, frame.origin.y + frame.size.height / 2.)
        };
        let plain = NSEventModifierFlags::empty();
        let down = |point| event(&table, point, NSEventType::LeftMouseDown, plain);
        let surface = Surface::Table(&table);
        surface.set_selected(&BTreeSet::from([0]));
        assert!(!surface.down(&down(row_point(1, 40.))));
        assert_eq!(surface.selected(), BTreeSet::from([1]));
        assert_eq!(*observer.ivars().model.borrow(), BTreeSet::from([1]));
        assert!(!handle_mouse(
            &table,
            &grid,
            &event(
                &table,
                row_point(2, 40.),
                NSEventType::LeftMouseDragged,
                plain
            )
        ));
        println!(
            "ok: file 1 -> press file 2 commits native and model selection before native drag"
        );
        surface.set_selected(&BTreeSet::from([1, 2]));
        assert!(!surface.down(&down(row_point(1, 40.))));
        assert_eq!(surface.selected(), BTreeSet::from([1, 2]));
        assert!(!surface.down(&down(row_point(2, 350.))));
        assert_eq!(surface.selected(), BTreeSet::from([1, 2]));
        println!("ok: selected file and selected row whitespace preserve multi-file drag sources");
        assert!(!surface.down(&event(
            &table,
            row_point(4, 40.),
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::Shift
        )));
        assert_eq!(surface.selected(), BTreeSet::from([1, 2]));
        assert!(!surface.down(&event(
            &table,
            row_point(4, 40.),
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::Command
        )));
        assert_eq!(surface.selected(), BTreeSet::from([1, 2]));
        println!("ok: modifier-click remains owned by native selection handling");
        assert!(surface.down(&down(row_point(3, 350.))));
        assert!(observer.ivars().tracking.get());
        assert!(surface.selected().is_empty());
        assert!(handle_mouse(
            &table,
            &grid,
            &event(
                &table,
                row_point(5, 40.),
                NSEventType::LeftMouseDragged,
                plain
            )
        ));
        assert_eq!(surface.selected(), BTreeSet::from([3, 4, 5]));
        assert!(handle_mouse(
            &table,
            &grid,
            &event(&table, row_point(5, 40.), NSEventType::LeftMouseUp, plain)
        ));
        assert!(!surface.active() && !observer.ivars().tracking.get());
        assert!(!handle_mouse(
            &table,
            &grid,
            &event(&table, row_point(5, 40.), NSEventType::LeftMouseUp, plain)
        ));
        println!("ok: background marquee routes down/drag/up once and releases ownership");
        observer.ivars().use_grid.set(true);
        let grid_surface = Surface::Grid(&grid);
        let grid_point = |i| {
            let frame = layout
                .layoutAttributesForItemAtIndexPath(&NSIndexPath::indexPathForItem_inSection(i, 0))
                .unwrap()
                .frame();
            NSPoint::new(
                frame.origin.x + frame.size.width / 2.,
                frame.origin.y + frame.size.height / 2.,
            )
        };
        grid_surface.set_selected(&BTreeSet::from([0]));
        assert!(!grid_surface.down(&event(
            &grid,
            grid_point(1),
            NSEventType::LeftMouseDown,
            plain
        )));
        assert_eq!(grid_surface.selected(), BTreeSet::from([1]));
        assert_eq!(*observer.ivars().model.borrow(), BTreeSet::from([1]));
        grid_surface.set_selected(&BTreeSet::from([1, 2]));
        assert!(!grid_surface.down(&event(
            &grid,
            grid_point(2),
            NSEventType::LeftMouseDown,
            plain
        )));
        assert_eq!(grid_surface.selected(), BTreeSet::from([1, 2]));
        println!("ok: icons also commit a new file and preserve an existing multi-selection");
        assert!(grid_surface.down(&event(
            &grid,
            NSPoint::new(490., 290.),
            NSEventType::LeftMouseDown,
            plain
        )));
        assert!(handle_mouse(
            &table,
            &grid,
            &event(&grid, grid_point(1), NSEventType::LeftMouseDragged, plain)
        ));
        assert!(grid_surface.selected().contains(&1));
        assert_eq!(grid_surface.selected(), *observer.ivars().model.borrow());
        grid_surface.finish(true);
        assert_eq!(grid_surface.selected(), BTreeSet::from([1, 2]));
        assert_eq!(*observer.ivars().model.borrow(), BTreeSet::from([1, 2]));
        assert!(!observer.ivars().tracking.get());
        println!("ok: icon marquee updates model and cancellation restores the original selection");
        // Exercise the same NSURL writer and reader used by actual drag/drop,
        // but use a private pasteboard, never the user's general clipboard.
        let path = std::env::temp_dir().join(format!(
            "RustExplorer-드래그😀-{}.txt",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"disposable native drag data").unwrap();
        let board = NSPasteboard::pasteboardWithUniqueName();
        let object = crate::ui::widgets::writer(&path);
        assert!(board.writeObjects(&NSArray::from_slice(&[&*object])));
        assert!(
            board
                .availableTypeFromArray(&NSArray::from_slice(&[NSPasteboardTypeFileURL]))
                .is_some()
        );
        let decoded = crate::ui::widgets::clipboard_paths(&board);
        assert_eq!(decoded.len(), 1);
        assert_eq!(
            std::fs::read(&decoded[0]).unwrap(),
            b"disposable native drag data"
        );
        board.clearContents();
        std::fs::remove_file(&path).unwrap();
        println!("ok: native drag writer exposes a consumable file URL on a private pasteboard");
        assert!(!window.isVisible(), "test must never show a window");
        window.setDelegate(None);
        println!(
            "7 native AppKit selection/drag-data scenarios passed; no window shown, no OS input posted"
        );
    });
}
