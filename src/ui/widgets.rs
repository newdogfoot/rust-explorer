#![allow(unsafe_op_in_unsafe_fn, unused_unsafe)]
use crate::core::model::*;
use crate::core::operations::FileTemplate;
use crate::platform;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::{cell::Cell, collections::HashMap, path::PathBuf};

define_class!(
 #[unsafe(super=NSWindow)]
 #[name="REExplorerWindow"]
 #[thread_kind=MainThreadOnly]
 pub struct ExplorerWindow;
 impl ExplorerWindow {
  #[unsafe(method(sendEvent:))]
  fn send_event(&self,event:&NSEvent){unsafe{
   if matches!(event.r#type(),NSEventType::LeftMouseDragged|NSEventType::LeftMouseUp)
    && let Some(delegate)=self.delegate()
    && delegate.respondsToSelector(sel!(handleSelectionMouseEvent:)) {
     let handled:bool=msg_send![&*delegate,handleSelectionMouseEvent:event];
     if handled{return;}
   }
   if event.r#type()==NSEventType::KeyDown
    && let Some(delegate)=self.delegate()
    && delegate.respondsToSelector(sel!(handleWindowKeyEvent:)){
     let handled:bool=msg_send![&*delegate,handleWindowKeyEvent:event];
     if handled{return;}
   }
   if let Some(mapped)=super::selection::windows_mouse_event(self,event) {
       let _:()=msg_send![super(self),sendEvent:&*mapped];
   } else {let _:()=msg_send![super(self),sendEvent:event];}
  }}
 }
);

define_class!(
 #[unsafe(super=NSButton)]
 #[name="RETabButton"]
 #[thread_kind=MainThreadOnly]
 pub struct TabButton;
 impl TabButton {
  #[unsafe(method(acceptsFirstMouse:))]
  fn first_mouse(&self,_:Option<&NSEvent>)->bool{true}
  #[unsafe(method(mouseDownCanMoveWindow))]
  fn moves_window(&self)->bool{false}
 }
);
pub unsafe fn tab_button(
    mtm: MainThreadMarker,
    title: &str,
    target: &NSObject,
    action: objc2::runtime::Sel,
) -> Retained<NSButton> {
    let button: Retained<TabButton> =
        msg_send![TabButton::alloc(mtm),initWithFrame:rect(0.,0.,28.,28.)];
    button.setTitle(&NSString::from_str(title));
    button.setTarget(Some(target));
    button.setAction(Some(action));
    button.setButtonType(NSButtonType::MomentaryPushIn);
    button.setBordered(false);
    Retained::into_super(button)
}

define_class!(
 #[unsafe(super=NSPathControl)]
 #[name="REAddressPathControl"]
 #[thread_kind=MainThreadOnly]
 pub struct AddressPathControl;
 impl AddressPathControl {
  #[unsafe(method(mouseDown:))]
  fn mouse_down(&self,event:&NSEvent){unsafe{
   // Keep ancestor crumbs navigable; the current crumb and empty area edit
   // the whole path with one click, like Windows Explorer.
   let point=self.convertPoint_fromView(event.locationInWindow(),None);
   #[allow(deprecated)]
   let component=self.cell().and_then(|c|c.downcast::<NSPathCell>().ok()).and_then(|c|c.pathComponentCellAtPoint_withFrame_inView(point,self.bounds(),self));
   let current=component.and_then(|c|c.URL()).zip(self.URL()).is_none_or(|(a,b)|a==b);
   if current {
    NSApplication::sharedApplication(self.mtm()).sendAction_to_from(sel!(editAddress:),self.target().as_deref(),Some(self));
   } else {let _:()=msg_send![super(self),mouseDown:event];}
  }}
 }
);
define_class!(
    #[unsafe(super=NSTableView)]
    #[name="REFileTableView"]
    #[thread_kind=MainThreadOnly]
    #[ivars=super::selection::SelectionIvars]
    pub struct FileTableView;
    impl FileTableView {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self,event:&NSEvent){if !super::selection::Surface::Table(self).down(event){unsafe{let _:()=msg_send![super(self),mouseDown:event];}}}

        #[unsafe(method_id(menuForEvent:))]
        fn context_menu(&self, event: &NSEvent) -> Option<Retained<NSMenu>> {
            unsafe { file_table_menu(self, event) }
        }
    }
);
define_class!(
 #[unsafe(super=NSCollectionView)]
 #[name="REFileCollectionView"]
 #[thread_kind=MainThreadOnly]
 #[ivars=super::selection::SelectionIvars]
 pub struct FileCollectionView;
 impl FileCollectionView {
  #[unsafe(method(mouseDown:))]
  fn mouse_down(&self,event:&NSEvent){if !super::selection::Surface::Grid(self).down(event){unsafe{let _:()=msg_send![super(self),mouseDown:event];}}}

  #[unsafe(method_id(menuForEvent:))]
  fn context_menu(&self,event:&NSEvent)->Option<Retained<NSMenu>>{unsafe{
      let point = self.convertPoint_fromView(event.locationInWindow(),None);
      if let Some(index) = self.indexPathForItemAtPoint(point) {
          if !self.selectionIndexPaths().containsObject(&index) {
              self.setSelectionIndexPaths(&NSSet::from_slice(&[&*index]));
          }
          if let Some(window) = self.window() { window.makeFirstResponder(Some(self)); }
          self.menu()
      } else {
          background_menu(self)
      }
  }}
 }
);

impl FileTableView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let obj = Self::alloc(mtm).set_ivars(Default::default());
        unsafe { msg_send![super(obj), initWithFrame: frame] }
    }
}
impl FileCollectionView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let obj = Self::alloc(mtm).set_ivars(Default::default());
        unsafe { msg_send![super(obj), initWithFrame: frame] }
    }
}

unsafe fn background_menu(view: &NSView) -> Option<Retained<NSMenu>> {
    view.enclosingScrollView()?.contentView().menu()
}

unsafe fn file_table_menu(table: &NSTableView, event: &NSEvent) -> Option<Retained<NSMenu>> {
    let location = event.locationInWindow();
    let point = table.convertPoint_fromView(location, None);
    let row = table.rowAtPoint(point);
    let selected_row = row >= 0 && table.isRowSelected(row);
    if !selected_row && super::selection::table_hit(table, point).is_none() {
        return background_menu(table);
    }
    if !table.isRowSelected(row) {
        table.selectRowIndexes_byExtendingSelection(
            &NSIndexSet::indexSetWithIndex(row as usize),
            false,
        );
    }
    if let Some(window) = table.window() {
        window.makeFirstResponder(Some(table));
    }
    table.menu()
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(0.0), h.max(0.0)))
}
pub unsafe fn single_line_editor(field: &NSTextField) {
    if let Some(cell) = field.cell() {
        cell.setUsesSingleLineMode(true);
        cell.setScrollable(true);
    }
}
pub unsafe fn text(mtm: MainThreadMarker, value: &str, size: f64) -> Retained<NSTextField> {
    let v = NSTextField::labelWithString(&NSString::from_str(value), mtm);
    v.setFont(Some(&NSFont::systemFontOfSize(size)));
    v.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    v
}
pub unsafe fn item(
    title: &str,
    action: objc2::runtime::Sel,
    target: Option<&NSObject>,
    key: &str,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let i = NSMenuItem::initWithTitle_action_keyEquivalent(
        NSMenuItem::alloc(mtm),
        &NSString::from_str(title),
        Some(action),
        &NSString::from_str(key),
    );
    i.setTarget(target.map(|v| &**v));
    i
}

pub unsafe fn add_creation_items(
    menu: &NSMenu,
    target: Option<&NSObject>,
    shortcut: NSEventModifierFlags,
    mtm: MainThreadMarker,
) {
    let folder = item("폴더", sel!(newFolder:), target, "n", mtm);
    folder.setKeyEquivalentModifierMask(shortcut | NSEventModifierFlags::Shift);
    menu.addItem(&folder);
    for (index, template) in FileTemplate::PRESETS.into_iter().enumerate() {
        let entry = item(
            template.menu_title(),
            sel!(newDocumentFromMenu:),
            target,
            "",
            mtm,
        );
        entry.setTag(index as isize);
        menu.addItem(&entry);
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(
        FileTemplate::Empty.menu_title(),
        sel!(newEmptyFile:),
        target,
        "",
        mtm,
    ));
}

pub unsafe fn creation_submenu(
    target: Option<&NSObject>,
    shortcut: NSEventModifierFlags,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let choices = NSMenu::new(mtm);
    add_creation_items(&choices, target, shortcut, mtm);
    let entry = NSMenuItem::new(mtm);
    entry.setTitle(ns_string!("새로 만들기"));
    entry.setImage(symbol("plus.circle").as_deref());
    entry.setSubmenu(Some(&choices));
    entry
}

#[derive(Default)]
pub struct NodeIvars {
    pub id: Cell<u64>,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[name = "REOutlineNode"]
    #[thread_kind=MainThreadOnly]
    #[ivars=NodeIvars]
    pub struct OutlineNode;
    unsafe impl NSObjectProtocol for OutlineNode {}
);
impl OutlineNode {
    pub fn new(mtm: MainThreadMarker, id: u64) -> Retained<Self> {
        let o = Self::alloc(mtm).set_ivars(NodeIvars { id: Cell::new(id) });
        unsafe { msg_send![super(o), init] }
    }
    pub fn id(&self) -> u64 {
        self.ivars().id.get()
    }
}

define_class!(
 #[unsafe(super=NSView)]
 #[name="REIconView"]
 #[thread_kind=MainThreadOnly]
 #[ivars=Cell<crate::core::theme::Resolved>]
 pub struct IconView;
 unsafe impl NSObjectProtocol for IconView{}
 impl IconView{
  #[unsafe(method(mouseDown:))]
  fn mouse_down(&self,event:&NSEvent){unsafe{let _:()=msg_send![super(self),mouseDown:event];if event.clickCount()==2{super::dispatch_open();}}}
 }
);
define_class!(
 #[unsafe(super=NSCollectionViewItem)]
 #[name="REFileCollectionItem"]
 #[thread_kind=MainThreadOnly]
 pub struct FileItem;
 unsafe impl NSObjectProtocol for FileItem{}
 impl FileItem{
  #[unsafe(method(loadView))]
  fn load_view(&self){unsafe{
   let mtm=self.mtm();let(w,h,_)=IconSize::Small.dimensions();let view:Retained<IconView>=msg_send![super(IconView::alloc(mtm).set_ivars(Cell::default())),initWithFrame:rect(0.,0.,w,h)];
   view.setWantsLayer(true);if let Some(layer)=view.layer(){layer.setCornerRadius(7.);}
   let image=NSImageView::initWithFrame(NSImageView::alloc(mtm),rect(23.,36.,54.,54.));image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
   // labelWithString is single-line even with maximumNumberOfLines = 2.
   // Use a wrapping label, but keep text selection off so file dragging wins.
   let label=NSTextField::wrappingLabelWithString(ns_string!(""),mtm);
   label.setFont(Some(&NSFont::systemFontOfSize(12.)));label.setSelectable(false);label.setAlignment(NSTextAlignment::Center);label.setMaximumNumberOfLines(2);label.setLineBreakMode(NSLineBreakMode::ByCharWrapping);
   if let Some(cell)=label.cell(){cell.setUsesSingleLineMode(false);cell.setScrollable(false);cell.setWraps(true);cell.setTruncatesLastVisibleLine(true);}
   view.addSubview(&image);view.addSubview(&label);self.setView(&view);self.setImageView(Some(&image));self.setTextField(Some(&label));
   layout_file_item(self);
  }}
  #[unsafe(method(setSelected:))]
  fn selected(&self,selected:bool){unsafe{
   let _:()=msg_send![super(self),setSelected:selected];
   if let Some(view)=self.view().downcast_ref::<IconView>(){super::theme::paint_item(self,view.ivars().get());}
  }}
  #[unsafe(method(viewDidLayout))]
  fn layout(&self){unsafe{
   let _:()=msg_send![super(self),viewDidLayout];
   layout_file_item(self);
  }}
 }
);

pub unsafe fn set_item_theme(item: &NSCollectionViewItem, p: crate::core::theme::Resolved) {
    if let Some(view) = item.view().downcast_ref::<IconView>() {
        view.ivars().set(p);
    }
    super::theme::paint_item(item, p);
}

pub unsafe fn layout_file_item(item: &NSCollectionViewItem) {
    let size = item.view().bounds().size;
    // Filename width must not determine the icon scale.
    let icon = if size.height >= IconSize::Large.dimensions().1 {
        IconSize::Large.dimensions().2
    } else if size.height >= IconSize::Medium.dimensions().1 {
        IconSize::Medium.dimensions().2
    } else {
        IconSize::Small.dimensions().2
    };
    if let Some(image) = item.imageView() {
        image.setFrame(rect((size.width - icon) / 2., 44., icon, icon));
    }
    if let Some(label) = item.textField() {
        label.setFrame(rect(5., 4., size.width - 10., 36.));
    }
}

pub unsafe fn cell(
    mtm: MainThreadMarker,
    table: &NSTableView,
    id: &str,
    with_icon: bool,
) -> Retained<NSTableCellView> {
    let identifier = NSString::from_str(id);
    if let Some(view) = table.makeViewWithIdentifier_owner(&identifier, None)
        && let Ok(cell) = view.downcast::<NSTableCellView>()
    {
        return cell;
    }
    let view = NSTableCellView::initWithFrame(NSTableCellView::alloc(mtm), rect(0., 0., 300., 30.));
    view.setIdentifier(Some(&identifier));
    let x = if with_icon { 36. } else { 8. };
    let label = text(mtm, "", 13.);
    label.setFrame(rect(x, 6., 300. - x - 8., 18.));
    label.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewMinYMargin
            | NSAutoresizingMaskOptions::ViewMaxYMargin,
    );
    view.addSubview(&label);
    view.setTextField(Some(&label));
    if with_icon {
        let image = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect(5., 4., 22., 22.));
        image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        image.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewMinYMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        view.addSubview(&image);
        view.setImageView(Some(&image));
    }
    view
}
pub unsafe fn symbol(name: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
}
#[derive(Default)]
pub struct ImageCache {
    images: HashMap<super::theme::Glyph, Retained<NSImage>>,
    theme: crate::core::theme::Resolved,
    thumbs: HashMap<u64, (Retained<NSImage>, usize, u64)>,
    bytes: usize,
    clock: u64,
}
impl ImageCache {
    pub unsafe fn icon(&mut self, entry: &Entry) -> Option<Retained<NSImage>> {
        if let Some((image, _, tick)) = self.thumbs.get_mut(&entry.id) {
            self.clock += 1;
            *tick = self.clock;
            return Some(image.clone());
        }
        let key = super::theme::Glyph::for_entry(entry);
        Some(
            self.images
                .entry(key)
                .or_insert_with(|| super::theme::icon(self.theme, key))
                .clone(),
        )
    }
    pub fn set_theme(&mut self, theme: crate::core::theme::Resolved) {
        if self.theme != theme {
            self.theme = theme;
            self.images.clear();
        }
    }
    #[cfg(feature = "native-ui-tests")]
    pub fn icon_count(&self) -> usize {
        self.images.len()
    }

    pub fn has_thumbnail(&self, id: u64) -> bool {
        self.thumbs.contains_key(&id)
    }
    pub fn insert_thumbnail(&mut self, id: u64, image: Retained<NSImage>, cost: usize) {
        const BUDGET: usize = 128 * 1024 * 1024;
        if cost > BUDGET {
            return;
        }
        if let Some((_, old, _)) = self.thumbs.remove(&id) {
            self.bytes -= old;
        }
        while self.bytes + cost > BUDGET {
            if let Some(key) = self
                .thumbs
                .iter()
                .min_by_key(|(_, (_, _, tick))| *tick)
                .map(|(k, _)| *k)
            {
                let (_, cost, _) = self.thumbs.remove(&key).unwrap();
                self.bytes -= cost;
            } else {
                break;
            }
        }
        self.clock += 1;
        self.bytes += cost;
        self.thumbs.insert(id, (image, cost, self.clock));
    }
    pub fn clear_thumbnails(&mut self) {
        self.thumbs.clear();
        self.bytes = 0;
    }
}

pub unsafe fn clipboard_paths(pasteboard: &NSPasteboard) -> Vec<PathBuf> {
    pasteboard
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
                .filter(|s| s.len() <= 131_072)
                .filter_map(|s| NSURL::URLWithString(&s))
                .filter_map(|url| platform::checked_file_url_path(&url))
                .collect()
        })
        .unwrap_or_default()
}
pub unsafe fn write_clipboard(paths: &[PathBuf], cut: bool) -> isize {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths
        .iter()
        .map(|path| {
            let item = NSPasteboardItem::new();
            item.setString_forType(
                &platform::file_url(path)
                    .absoluteString()
                    .unwrap_or_else(|| NSString::from_str("")),
                NSPasteboardTypeFileURL,
            );
            if cut {
                item.setString_forType(ns_string!("1"), ns_string!("local.rust-explorer.cut"));
            }
            ProtocolObject::from_retained(item)
        })
        .collect();
    board.writeObjects(&NSArray::from_retained_slice(&items));
    board.changeCount()
}
pub unsafe fn clipboard_cut(board: &NSPasteboard) -> bool {
    board
        .stringForType(ns_string!("local.rust-explorer.cut"))
        .is_some()
}
pub unsafe fn writer(path: &std::path::Path) -> Retained<ProtocolObject<dyn NSPasteboardWriting>> {
    ProtocolObject::from_retained(platform::file_url(path))
}

pub unsafe fn show_message(window: &NSWindow, title: &str, detail: &str) {
    let alert = NSAlert::new(window.mtm());
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.addButtonWithTitle(ns_string!("확인"));
    alert.beginSheetModalForWindow_completionHandler(window, None);
}
