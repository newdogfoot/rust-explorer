//! File-list gestures. AppKit still owns file dragging and keyboard navigation;
//! only a press on visual background starts a marquee.
use super::widgets::{FileCollectionView, FileTableView, rect};
use crate::core::selection::{SelectionMode, edge_scroll};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::{cell::RefCell, collections::BTreeSet};

#[derive(Default)]
pub struct SelectionIvars {
    gesture: RefCell<Option<Gesture>>,
}
struct Gesture {
    anchor: NSPoint,
    last_window: NSPoint,
    initial: BTreeSet<usize>,
    mode: SelectionMode,
    moved: bool,
    overlay: Retained<SelectionOverlay>,
}

define_class!(
    #[unsafe(super=NSView)]
    #[name="RESelectionOverlay"]
    #[thread_kind=MainThreadOnly]
    struct SelectionOverlay;
    impl SelectionOverlay {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _:NSPoint)->Option<Retained<NSView>> { None }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _:NSRect) {
            let bounds=self.bounds();
            NSColor::controlAccentColor().colorWithAlphaComponent(0.16).setFill();
            NSBezierPath::fillRect(bounds);
            NSColor::controlAccentColor().setStroke();
            let path=NSBezierPath::bezierPathWithRect(rect(0.5,0.5,bounds.size.width-1.,bounds.size.height-1.));
            path.setLineWidth(1.);
            path.stroke();
        }
    }
);

// NSTableView can be shorter than its viewport. Forward the exposed clip-view
// background too, otherwise dragging below the final row is still a dead zone.
define_class!(
    #[unsafe(super=NSClipView)]
    #[name="REFileClipView"]
    #[thread_kind=MainThreadOnly]
    pub struct FileClipView;
    impl FileClipView {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self,event:&NSEvent){
            if let Some(doc)=self.documentView() {
                if let Some(table)=doc.downcast_ref::<FileTableView>() {
                    if Surface::Table(table).down(event){return;}
                } else if let Some(grid)=doc.downcast_ref::<FileCollectionView>()
                    && Surface::Grid(grid).down(event) {return;}
            }
            // Never send an unhandled event back to the document view: its
            // default responder chain has already forwarded it to this clip.
            unsafe { let _:()=msg_send![super(self),mouseDown:event]; }
        }
    }
);

#[derive(Clone, Copy)]
pub enum Surface<'a> {
    Table(&'a FileTableView),
    Grid(&'a FileCollectionView),
}
impl<'a> Surface<'a> {
    fn view(self) -> &'a NSView {
        match self {
            Self::Table(v) => v,
            Self::Grid(v) => v,
        }
    }
    fn ivars(self) -> &'a SelectionIvars {
        match self {
            Self::Table(v) => v.ivars(),
            Self::Grid(v) => v.ivars(),
        }
    }
    pub fn active(self) -> bool {
        self.ivars().gesture.borrow().is_some()
    }
    pub fn down(self, event: &NSEvent) -> bool {
        let view = self.view();
        // Finish deferred header/layout changes before capturing document-space
        // coordinates; the first press after a tab switch can trigger tiling.
        if let Some(scroll) = view.enclosingScrollView() {
            scroll.layoutSubtreeIfNeeded();
            scroll.tile();
        }
        view.layoutSubtreeIfNeeded();
        let point = view.convertPoint_fromView(event.locationInWindow(), None);
        let hit = match self {
            Self::Table(v) => {
                let row = v.rowAtPoint(point);
                table_hit(v, point)
                    .or_else(|| (row >= 0 && v.isRowSelected(row)).then_some(row))
                    .map(|row| row as usize)
            }
            Self::Grid(v) => v
                .indexPathForItemAtPoint(point)
                .map(|index| index.item() as usize),
        };
        if let Some(index) = hit {
            self.finish(false);
            if let Some(window) = view.window() {
                window.makeFirstResponder(Some(view));
            }
            let modifiers = event.modifierFlags();
            // Commit the pressed item BEFORE AppKit starts tracking a drag.
            // Keep an already selected group together. Native modifier-click
            // handling remains responsible for extending/toggling selection.
            if !modifiers.intersects(
                NSEventModifierFlags::Command
                    | NSEventModifierFlags::Control
                    | NSEventModifierFlags::Shift,
            ) && !self.selected().contains(&index)
            {
                self.set_selected(&BTreeSet::from([index]));
            }
            self.notify_selection();
            return false;
        }
        self.finish(false);
        let Some(scroll) = view.enclosingScrollView() else {
            return false;
        };
        let Some(window) = view.window() else {
            return false;
        };
        if !window.makeFirstResponder(Some(view)) {
            return false;
        }
        let point = view.convertPoint_fromView(event.locationInWindow(), None);
        let initial = self.selected();
        let flags = event.modifierFlags();
        let mode = SelectionMode::from_modifiers(
            flags.contains(NSEventModifierFlags::Shift),
            flags.contains(NSEventModifierFlags::Command),
        );
        let overlay: Retained<SelectionOverlay> = unsafe {
            msg_send![SelectionOverlay::alloc(view.mtm()),initWithFrame:rect(0.,0.,0.,0.)]
        };
        overlay.setHidden(true);
        scroll.contentView().addSubview(&overlay);
        *self.ivars().gesture.borrow_mut() = Some(Gesture {
            anchor: point,
            last_window: event.locationInWindow(),
            initial,
            mode,
            moved: false,
            overlay,
        });
        self.notify_tracking(true);
        if mode == SelectionMode::Replace {
            self.set_selected(&BTreeSet::new());
        }
        true
    }
    pub fn dragged(self, event: &NSEvent) -> bool {
        if !self.active() {
            return false;
        }
        if let Some(g) = self.ivars().gesture.borrow_mut().as_mut() {
            g.last_window = event.locationInWindow();
        }
        self.update(true);
        true
    }
    pub fn up(self, event: &NSEvent) -> bool {
        if !self.active() {
            return false;
        }
        if let Some(g) = self.ivars().gesture.borrow_mut().as_mut() {
            g.last_window = event.locationInWindow();
        }
        self.update(false);
        self.finish(false);
        true
    }
    pub fn tick(self) {
        if !self.active() {
            return;
        }
        let active = self
            .view()
            .window()
            .is_some_and(|w| w.isKeyWindow() && w.attachedSheet().is_none());
        if !active {
            self.finish(true);
        } else if NSEvent::pressedMouseButtons() & 1 == 0 {
            self.finish(false);
        } else {
            self.update(true);
        }
    }
    pub fn finish(self, cancel: bool) {
        let gesture = self.ivars().gesture.borrow_mut().take();
        if let Some(g) = gesture {
            g.overlay.removeFromSuperview();
            if cancel {
                self.set_selected(&g.initial);
            }
            self.notify_tracking(false);
        }
    }
    fn update(self, scroll: bool) {
        let view = self.view();
        let (anchor, location, moved) = {
            let g = self.ivars().gesture.borrow();
            let Some(g) = g.as_ref() else { return };
            (g.anchor, g.last_window, g.moved)
        };
        let mut point = view.convertPoint_fromView(location, None);
        if !moved && (point.x - anchor.x).hypot(point.y - anchor.y) < 4. {
            return;
        }
        if scroll && let Some(scroll) = view.enclosingScrollView() {
            let visible = view.visibleRect();
            let dx = edge_scroll(point.x, visible.origin.x, visible.size.width);
            let dy = edge_scroll(point.y, visible.origin.y, visible.size.height);
            let clip = scroll.contentView();
            let mut bounds = clip.bounds();
            bounds.origin.x += dx;
            bounds.origin.y += dy;
            let constrained = clip.constrainBoundsRect(bounds);
            clip.scrollToPoint(constrained.origin);
            scroll.reflectScrolledClipView(&clip);
            point = view.convertPoint_fromView(location, None);
        }
        let bounds = rect(
            anchor.x.min(point.x),
            anchor.y.min(point.y),
            (anchor.x - point.x).abs().max(1.),
            (anchor.y - point.y).abs().max(1.),
        );
        let hits = self.hits(bounds);
        let selected = {
            let mut g = self.ivars().gesture.borrow_mut();
            let Some(g) = g.as_mut() else { return };
            g.moved = true;
            if let Some(clip) = view.enclosingScrollView().map(|s| s.contentView()) {
                g.overlay
                    .setFrame(view.convertRect_toView(bounds, Some(&clip)));
                g.overlay.setHidden(false);
                g.overlay.setNeedsDisplay(true);
            }
            g.mode.apply(&g.initial, hits)
        };
        self.set_selected(&selected);
    }
    fn hits(self, bounds: NSRect) -> BTreeSet<usize> {
        match self {
            Self::Table(table) => crate::core::selection::intersecting_rows(
                bounds.origin.y,
                bounds.origin.y + bounds.size.height,
                table.numberOfRows().max(0) as usize,
                |index| {
                    let frame = table.rectOfRow(index as isize);
                    (frame.origin.y, frame.origin.y + frame.size.height)
                },
            )
            .collect(),
            Self::Grid(grid) => grid
                .collectionViewLayout()
                .map(|layout| {
                    layout
                        .layoutAttributesForElementsInRect(bounds)
                        .iter()
                        .filter(|a| {
                            a.representedElementCategory() == NSCollectionElementCategory::Item
                        })
                        .filter_map(|a| a.indexPath())
                        .filter(|p| p.section() == 0)
                        .map(|p| p.item() as usize)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
    fn selected(self) -> BTreeSet<usize> {
        match self {
            Self::Table(table) => {
                let set = table.selectedRowIndexes();
                let mut rows = BTreeSet::new();
                let mut i = set.firstIndex();
                while i != NSNotFound as usize {
                    rows.insert(i);
                    i = set.indexGreaterThanIndex(i);
                }
                rows
            }
            Self::Grid(grid) => grid
                .selectionIndexPaths()
                .iter()
                .map(|p| p.item() as usize)
                .collect(),
        }
    }
    fn set_selected(self, rows: &BTreeSet<usize>) {
        if &self.selected() == rows {
            return;
        }
        match self {
            Self::Table(table) => {
                let indices = NSMutableIndexSet::new();
                for row in rows {
                    indices.addIndex(*row);
                }
                table.selectRowIndexes_byExtendingSelection(&indices, false);
            }
            Self::Grid(grid) => {
                let paths: Vec<_> = rows
                    .iter()
                    .map(|i| NSIndexPath::indexPathForItem_inSection(*i as isize, 0))
                    .collect();
                grid.setSelectionIndexPaths(&NSSet::from_retained_slice(&paths));
            }
        }
        self.notify_selection();
    }
    fn notify_selection(self) {
        if let Some(delegate) = self.view().window().and_then(|w| w.delegate()) {
            unsafe {
                let _: () = msg_send![&*delegate, fileSelectionChanged];
            }
        }
    }
    fn notify_tracking(self, tracking: bool) {
        if let Some(delegate) = self.view().window().and_then(|w| w.delegate()) {
            unsafe {
                let _: () = msg_send![&*delegate,fileSelectionTracking:tracking];
            }
        }
    }
}

/// Share hit-testing between left and right clicks. A selected row's empty
/// name-column area belongs to the selected files, so its highlight can be
/// dragged or right-clicked without discarding the multi-selection.
pub fn table_hit(table: &NSTableView, point: NSPoint) -> Option<isize> {
    let row = table.rowAtPoint(point);
    let column = table.columnAtPoint(point);
    if row < 0 || column < 0 {
        return None;
    }
    if let Some(cell) = table
        .viewAtColumn_row_makeIfNecessary(column, row, false)
        .and_then(|v| v.downcast::<NSTableCellView>().ok())
        && cell
            .identifier()
            .is_some_and(|id| id.isEqualToString(ns_string!("name")))
        && let Some(label) = unsafe { cell.textField() }
    {
        let local = label.convertPoint_fromView(point, Some(table));
        if local.x
            > label
                .intrinsicContentSize()
                .width
                .min(label.bounds().size.width)
                + 5.
        {
            return None;
        }
    }
    Some(row)
}

/// Consume only a marquee that we started. Native file dragging must retain
/// NSTableView/NSCollectionView's own mouseDragged and mouseUp implementations.
pub fn handle_mouse(table: &NSTableView, grid: &NSCollectionView, event: &NSEvent) -> bool {
    let surface = table
        .downcast_ref::<FileTableView>()
        .map(Surface::Table)
        .filter(|surface| surface.active())
        .or_else(|| {
            grid.downcast_ref::<FileCollectionView>()
                .map(Surface::Grid)
                .filter(|surface| surface.active())
        });
    match (surface, event.r#type()) {
        (Some(surface), NSEventType::LeftMouseDragged) => surface.dragged(event),
        (Some(surface), NSEventType::LeftMouseUp) => surface.up(event),
        _ => false,
    }
}

pub fn tick(table: &NSTableView, grid: &NSCollectionView) {
    if let Some(v) = table.downcast_ref::<FileTableView>() {
        Surface::Table(v).tick();
    }
    if let Some(v) = grid.downcast_ref::<FileCollectionView>() {
        Surface::Grid(v).tick();
    }
}
pub fn finish(table: &NSTableView, grid: &NSCollectionView, cancel: bool) {
    if let Some(v) = table.downcast_ref::<FileTableView>() {
        Surface::Table(v).finish(cancel);
    }
    if let Some(v) = grid.downcast_ref::<FileCollectionView>() {
        Surface::Grid(v).finish(cancel);
    }
}

/// Windows Ctrl-click is selection, not AppKit's Control-click context menu.
/// Translate only this window's file area; real right-click and text input pass.
pub fn windows_mouse_event(window: &NSWindow, event: &NSEvent) -> Option<Retained<NSEvent>> {
    if !matches!(
        event.r#type(),
        NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged | NSEventType::LeftMouseUp
    ) || !event
        .modifierFlags()
        .contains(NSEventModifierFlags::Control)
        || window.attachedSheet().is_some()
    {
        return None;
    }
    let delegate = window.delegate()?;
    if !delegate.respondsToSelector(sel!(usesWindowsSelection)) {
        return None;
    }
    let windows: bool = unsafe { msg_send![&*delegate, usesWindowsSelection] };
    if !windows {
        return None;
    }
    let root = window.contentView()?;
    let point = root.convertPoint_fromView(event.locationInWindow(), None);
    let mut hit = root.hitTest(point);
    let mut file_area = false;
    while let Some(view) = hit {
        if view.downcast_ref::<NSTextView>().is_some() {
            return None;
        }
        if view.downcast_ref::<FileTableView>().is_some()
            || view.downcast_ref::<FileCollectionView>().is_some()
            || view.downcast_ref::<FileClipView>().is_some()
        {
            file_area = true;
            break;
        }
        hit = unsafe { view.superview() };
    }
    if !file_area {
        return None;
    }
    let flags =
        (event.modifierFlags() & !NSEventModifierFlags::Control) | NSEventModifierFlags::Command;
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(event.r#type(),event.locationInWindow(),flags,event.timestamp(),event.windowNumber(),None,event.eventNumber(),event.clickCount(),event.pressure())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::ClassType;

    #[test]
    fn marquee_does_not_override_native_drag_or_bounce_events_back_to_document() {
        // Regression for the user's stack-overflow crash: the table and clip
        // previously called mouseDragged: on each other indefinitely.
        for class in [
            FileTableView::class(),
            FileCollectionView::class(),
            FileClipView::class(),
        ] {
            let own_methods = class.instance_methods();
            for method in own_methods.iter() {
                assert!(
                    ![sel!(mouseDragged:), sel!(mouseUp:)].contains(&method.name()),
                    "{} must inherit native {}",
                    class.name().to_string_lossy(),
                    method.name()
                );
            }
        }
        assert!(
            super::super::controller::Controller::class()
                .responds_to(sel!(handleSelectionMouseEvent:))
        );
    }
}

#[cfg(feature = "native-ui-tests")]
#[path = "selection_native_checks.rs"]
pub(crate) mod native_checks;
