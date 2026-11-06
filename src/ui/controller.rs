#![allow(unsafe_op_in_unsafe_fn, unused_unsafe)]
use super::{
    design::{self, ExplorerViews},
    runtime::{Event, FilterRequest, Runtime, ScanRequest},
    widgets::*,
};
use crate::core::shortcuts::PinGroup;
use crate::core::{
    listing::{ScanFailure, ScanUpdate},
    model::*,
    operations::*,
    worker::Cancellation,
};
use crate::platform;
#[path = "batch_rename.rs"]
mod batch_rename;
#[path = "context_menu.rs"]
mod context_menu;
#[path = "creation.rs"]
mod creation;
#[path = "finder.rs"]
mod finder;
#[path = "folder_access.rs"]
mod folder_access;
#[path = "server.rs"]
mod server;
#[path = "sidebar.rs"]
mod sidebar;
use objc2::{
    AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use objc2_quick_look_thumbnailing::*;
use objc2_quick_look_ui::*;
use std::{
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub struct Tab {
    pub id: u64,
    pub path: PathBuf,
    pub back: Vec<PathBuf>,
    pub forward: Vec<PathBuf>,
    pub generation: u64,
    pub cancel: Cancellation,
    pub filter_cancel: Cancellation,
    pub entries: Arc<Vec<Entry>>,
    pub visible: Vec<usize>,
    pub metadata: HashMap<EntryId, Entry>,
    pub requested: HashSet<EntryId>,
    pub selected: HashSet<EntryId>,
    pub loading: bool,
    pub refreshing: bool,
    pub error: Option<ScanFailure>,
    pub skipped: usize,
    pub query: String,
    pub recursive: bool,
    pub filter_generation: u64,
    pub filter_due: Option<Instant>,
    pub watch_due: Option<Instant>,
    pub load_started: Instant,
    pub first_displayed: bool,
    pub sort: SortField,
    pub descending: bool,
    pub scroll: [Option<NSPoint>; 2],
    pub read_only: bool,
    pub free_bytes: Option<u64>,
}
impl Tab {
    fn can_write(&self) -> bool {
        !self.read_only && self.error.is_none()
    }

    fn volume_disconnected(&mut self) {
        self.cancel.cancel();
        self.filter_cancel.cancel();
        self.generation += 1;
        self.entries = Arc::new(Vec::new());
        self.visible.clear();
        self.selected.clear();
        self.metadata.clear();
        self.requested.clear();
        self.loading = false;
        self.refreshing = false;
        self.read_only = true;
        self.free_bytes = None;
        self.watch_due = None;
        self.filter_due = None;
        self.error =
            Some("저장 위치의 연결이 해제되었습니다. 다시 연결하면 목록을 갱신합니다.".into());
    }
    fn new(id: u64, path: PathBuf) -> Self {
        Self {
            id,
            path,
            back: vec![],
            forward: vec![],
            generation: 0,
            cancel: Cancellation::default(),
            filter_cancel: Cancellation::default(),
            entries: Arc::new(vec![]),
            visible: vec![],
            metadata: HashMap::new(),
            requested: HashSet::new(),
            selected: HashSet::new(),
            loading: false,
            refreshing: false,
            error: None,
            skipped: 0,
            query: String::new(),
            recursive: false,
            filter_generation: 0,
            filter_due: None,
            watch_due: None,
            load_started: Instant::now(),
            first_displayed: false,
            sort: SortField::Name,
            descending: false,
            scroll: [None; 2],
            read_only: false,
            free_bytes: None,
        }
    }
}
pub struct SidebarNode {
    pub object: Retained<OutlineNode>,
    pub label: String,
    pub path: Option<PathBuf>,
    pub children: Vec<u64>,
    pub loaded: bool,
    pub loading: bool,
    pub section: bool,
    pub key: String,
    pub symbol: String,
    pub action: Option<sidebar::SidebarAction>,
    pub group: u64,
}
pub struct HistoryDialog {
    pub popup: Retained<NSPopUpButton>,
    pub details: Retained<NSTextField>,
    pub buttons: Vec<Retained<NSButton>>,
    pub records: Vec<Journal>,
}
pub struct State {
    pub appearance_panel: Option<Retained<super::appearance::AppearancePanel>>,
    access_return: Option<(u64, PathBuf)>,
    pending_access: Option<folder_access::PendingAccess>,
    access_list: Option<Retained<NSTextView>>,
    pub size_cancel: Cancellation,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub prefs: Preferences,
    pub nodes: HashMap<u64, SidebarNode>,
    pub roots: Vec<u64>,
    pub images: ImageCache,
    pub last_job: Option<Journal>,
    pub active_job: Option<u64>,
    pub renaming: Option<PathBuf>,
    pub rename_editor: Option<Retained<NSTextField>>,
    pub last_clipboard: Option<isize>,
    paste_moves: HashMap<u64, isize>,
    #[cfg(feature = "native-ui-tests")]
    test_clipboard: Option<Retained<NSPasteboard>>,
    pub quick_urls: Vec<Retained<NSURL>>,
    pub preview_view: Option<Retained<QLPreviewView>>,
    pub preview_path: Option<PathBuf>,
    pub history: Option<HistoryDialog>,
    pub reloading: bool,
    pub theme_name: String,
    pub last_theme_check: Instant,
    pub thumbnail_requests: HashMap<EntryId, (Retained<QLThumbnailGenerationRequest>, Instant)>,
    pub thumbnail_attempted: HashSet<EntryId>,
    pub locations: Vec<crate::platform::Location>,
    pub rebuilding_sidebar: bool,
    pub sidebar_context: Option<u64>,
    pub mount_request: Option<(u64, platform::server::MountRequest)>,
    pub job_reveal_due: Option<Instant>,
    pub notice: Option<(String, Instant, Option<String>)>,
    pub extraction_jobs: HashMap<u64, (u64, PathBuf)>,
    pub creation_jobs: HashMap<u64, (u64, PathBuf, bool)>,
    pub pending_creation: Option<creation::PendingCreation>,
    pub sharing_service: Option<Retained<NSSharingService>>,
    pub batch_dialog: Option<batch_rename::BatchDialog>,
    pub application_dialog: Option<finder::ApplicationDialog>,
    pub folder_request: u64,
    pub dragging: bool,
    pub selecting: bool,
    pub selection_events: VecDeque<Event>,
    pub drag_refresh_due: Option<Instant>,
    #[cfg(feature = "native-ui-tests")]
    test_drag_modifiers: Option<NSEventModifierFlags>,
    pub input_diagnostics: bool,
    pub diagnostic_events: VecDeque<String>,
    pub diagnostic_count: u64,
}
pub struct ControllerIvars {
    pub id: u64,
    pub runtime: Arc<Runtime>,
    pub views: OnceCell<ExplorerViews>,
    pub state: RefCell<State>,
}

define_class!(
 #[unsafe(super=NSResponder)]
 #[name="REWindowController"]
 #[thread_kind=MainThreadOnly]
 #[ivars=ControllerIvars]
 pub struct Controller;
 unsafe impl NSObjectProtocol for Controller{}
 unsafe impl NSWindowDelegate for Controller{
  #[unsafe(method(windowDidResize:))]fn resized(&self,_:&NSNotification){unsafe{self.views().layout();self.refresh_tabs();self.update_breadcrumb();self.layout_preview();}}
  #[unsafe(method(windowWillClose:))]fn closed(&self,_:&NSNotification){if let Some(panel)=self.ivars().state.borrow().appearance_panel.as_ref(){panel.window().close();}super::selection::finish(&self.views().table,&self.views().collection,false);self.abandon_mount();self.cancel_size();self.cancel_thumbnails();self.ivars().runtime.unwatch(self.id(),None);for t in &self.ivars().state.borrow().tabs{t.cancel.cancel();t.filter_cancel.cancel();}super::close_controller(self.id());}
  #[unsafe(method(windowDidBecomeKey:))]fn became_key(&self,_:&NSNotification){super::window_became_key(self.id());self.save_tabs();self.update_status();}
  #[unsafe(method(windowDidResignKey:))]fn resigned_key(&self,_:&NSNotification){self.refresh_input_diagnostics();}
 }
 unsafe impl NSTableViewDataSource for Controller{
  #[unsafe(method(tableView:draggingSession:willBeginAtPoint:forRowIndexes:))]fn table_drag_begin(&self,table:&NSTableView,_:&NSDraggingSession,_:NSPoint,rows:&NSIndexSet){self.ivars().state.borrow_mut().dragging=true;table.selectRowIndexes_byExtendingSelection(rows,false);self.selection_changed();}
  #[unsafe(method(tableView:draggingSession:endedAtPoint:operation:))]fn table_drag_end(&self,_:&NSTableView,_:&NSDraggingSession,_:NSPoint,_:NSDragOperation){self.drag_ended();}
  #[unsafe(method(numberOfRowsInTableView:))]fn row_count(&self,_:&NSTableView)->isize{self.with_tab(|t|t.visible.len()as isize)}
  #[unsafe(method_id(tableView:pasteboardWriterForRow:))]fn table_writer(&self,_:&NSTableView,row:isize)->Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>>{self.entry_at(row).map(|e|unsafe{writer(&e.path)})}
  #[unsafe(method(tableView:validateDrop:proposedRow:proposedDropOperation:))]fn validate_drop(&self,t:&NSTableView,info:&ProtocolObject<dyn NSDraggingInfo>,row:isize,op:NSTableViewDropOperation)->NSDragOperation{unsafe{
   let folder=if op==NSTableViewDropOperation::On{self.entry_at(row).filter(|e|e.kind==Kind::Folder).map(|e|e.path)}else{None};
   if folder.is_none(){t.setDropRow_dropOperation(-1,NSTableViewDropOperation::On);}
   self.validate_drop_target(info,&folder.unwrap_or_else(||self.current_path()))
  }}
  #[unsafe(method(tableView:acceptDrop:row:dropOperation:))]fn accept_drop(&self,_:&NSTableView,info:&ProtocolObject<dyn NSDraggingInfo>,row:isize,op:NSTableViewDropOperation)->bool{unsafe{let target=if op==NSTableViewDropOperation::On{self.entry_at(row).filter(|e|e.kind==Kind::Folder).map(|e|e.path)}else{None}.unwrap_or_else(||self.current_path());self.drop_files(info,target)}}
  #[unsafe(method(tableView:sortDescriptorsDidChange:))]fn sort_changed(&self,table:&NSTableView,_:&NSArray<NSSortDescriptor>){if self.ivars().state.borrow().reloading{return;}
  if let Some(d)=table.sortDescriptors().firstObject(){let key=d.key().map(|s|s.to_string()).unwrap_or_default();let field=match key.as_str(){"date"=>SortField::Modified,"size"=>SortField::Size,"kind"=>SortField::Kind,_=>SortField::Name};self.set_sort(field,!d.ascending());}}
 }
 unsafe impl NSTableViewDelegate for Controller{
  #[unsafe(method(tableView:didAddRowView:forRow:))]
  fn file_row_added(&self,table:&NSTableView,view:&NSTableRowView,row:isize){unsafe{self.paint_file_row(table,view,row);}}
  #[unsafe(method_id(tableView:rowViewForRow:))]
  fn table_row(&self,table:&NSTableView,_:isize)->Option<Retained<NSTableRowView>>{Some(unsafe{super::theme::row(self.mtm(),table,self.views().palette.get(),false)})}
  #[unsafe(method_id(tableView:viewForTableColumn:row:))]fn table_cell(&self,table:&NSTableView,column:Option<&NSTableColumn>,row:isize)->Option<Retained<NSView>>{(||unsafe{
   let e=self.entry_at(row)?;let key=column.map(|c|c.identifier().to_string()).unwrap_or_else(||"name".into());let view=cell(self.mtm(),table,&key,key=="name");
   if let Some(label)=view.textField(){label.setEditable(false);label.setSelectable(false);label.setBezeled(false);label.setDrawsBackground(false);label.setTextColor(Some(&super::theme::color(if key=="name"{self.views().palette.get().ink}else{self.views().palette.get().muted})));let value=match key.as_str(){"date"=>e.modified.and_then(|s|chrono::DateTime::from_timestamp(s,0)).map(|d|d.with_timezone(&chrono::Local).format("%Y.%m.%d %H:%M").to_string()).unwrap_or_else(||"—".into()),"size"=>e.size.map(format_size).unwrap_or_else(||"—".into()),"kind"=>e.kind_label(),_=>e.name.clone()};label.setStringValue(&NSString::from_str(&value));label.setToolTip(Some(&NSString::from_str(&value)));label.setFont(Some(&NSFont::systemFontOfSize(if key=="name"{13.}else{12.})));label.setAlignment(if key=="size"{NSTextAlignment::Right}else{NSTextAlignment::Left});}
   if let Some(image)=view.imageView(){image.setImage(self.ivars().state.borrow_mut().images.icon(&e).as_deref());image.setToolTip(Some(&NSString::from_str(&e.name)));}
   view.setToolTip(Some(&NSString::from_str(&e.name)));Some(Retained::into_super(view))
  })()}
  #[unsafe(method_id(tableView:typeSelectStringForTableColumn:row:))]fn type_select(&self,_:&NSTableView,_:Option<&NSTableColumn>,row:isize)->Option<Retained<NSString>>{self.entry_at(row).map(|e|NSString::from_str(&e.name))}
  #[unsafe(method(tableViewSelectionDidChange:))]fn table_selection_changed(&self,_:&NSNotification){self.selection_changed();}
 }
 unsafe impl NSOutlineViewDataSource for Controller{
  #[unsafe(method(outlineView:validateDrop:proposedItem:proposedChildIndex:))]
  unsafe fn outline_drop_validate(&self, outline:&NSOutlineView, info:&ProtocolObject<dyn NSDraggingInfo>, item:Option<&AnyObject>, _:isize)->NSDragOperation {
      if node_id(item).and_then(PinGroup::from_section).is_some() {
          outline.setDropItem_dropChildIndex(item,-1);
          return self.pin_drag_operation(info);
      }
      let path=node_id(item).and_then(|id|self.ivars().state.borrow().nodes.get(&id).and_then(|n|n.path.clone()));
      path.map(|p|self.validate_drop_target(info,&p)).unwrap_or(NSDragOperation::None)
  }
  #[unsafe(method(outlineView:acceptDrop:item:childIndex:))]
  unsafe fn outline_drop_accept(&self,_:&NSOutlineView,info:&ProtocolObject<dyn NSDraggingInfo>,item:Option<&AnyObject>,_:isize)->bool {
      if let Some(group)=node_id(item).and_then(PinGroup::from_section) {
          self.accept_pin_drop(info,group)
      } else {
          node_id(item).and_then(|id|self.ivars().state.borrow().nodes.get(&id).and_then(|n|n.path.clone())).is_some_and(|path|self.drop_files(info,path))
      }
  }
  #[unsafe(method_id(outlineView:pasteboardWriterForItem:))]unsafe fn outline_writer(&self,_:&NSOutlineView,item:&AnyObject)->Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>>{node_id(Some(item)).and_then(|id|self.ivars().state.borrow().nodes.get(&id).and_then(|n|n.path.clone())).map(|p|writer(&p))}
  #[unsafe(method(outlineView:numberOfChildrenOfItem:))]unsafe fn children_count(&self,_:&NSOutlineView,item:Option<&AnyObject>)->isize{
   if let Some(id)=node_id(item){self.ensure_sidebar(id);self.ivars().state.borrow().nodes.get(&id).map_or(0,|n|n.children.len()as isize)}else{self.ivars().state.borrow().roots.len()as isize}
  }
  #[unsafe(method_id(outlineView:child:ofItem:))]unsafe fn child(&self,_:&NSOutlineView,index:isize,item:Option<&AnyObject>)->Retained<AnyObject>{let s=self.ivars().state.borrow();let ids=if let Some(id)=node_id(item){&s.nodes[&id].children}else{&s.roots};Retained::cast_unchecked(ids.get(index.max(0)as usize).and_then(|id|s.nodes.get(id)).map(|n|n.object.clone()).unwrap_or_else(||OutlineNode::new(self.mtm(),u64::MAX)))}
  #[unsafe(method(outlineView:isItemExpandable:))]unsafe fn expandable(&self,_:&NSOutlineView,item:&AnyObject)->bool{node_id(Some(item)).and_then(|id|self.ivars().state.borrow().nodes.get(&id).map(|n|!n.loaded||!n.children.is_empty())).unwrap_or(false)}
 }
 unsafe impl NSOutlineViewDelegate for Controller{
  #[unsafe(method(outlineView:didAddRowView:forRow:))]
  fn side_row_added(&self,table:&NSOutlineView,view:&NSTableRowView,row:isize){unsafe{self.paint_side_row(table,view,row);}}
  #[unsafe(method_id(outlineView:rowViewForItem:))]
  unsafe fn outline_row(&self,table:&NSOutlineView,_:&AnyObject)->Option<Retained<NSTableRowView>>{Some(super::theme::row(self.mtm(),table,self.views().palette.get(),true))}
  #[unsafe(method_id(outlineView:viewForTableColumn:item:))]unsafe fn outline_cell(&self,outline:&NSOutlineView,_:Option<&NSTableColumn>,item:&AnyObject)->Option<Retained<NSView>>{(||{
   let id=node_id(Some(item))?;let(label,section,icon,unavailable)={let s=self.ivars().state.borrow();let n=s.nodes.get(&id)?;(n.label.clone(),n.section,n.symbol.clone(),matches!(n.action,Some(sidebar::SidebarAction::ICloudUnavailable)))};
   let cell_id = match (section,id) { (true,1) => "favorites-section", (true,3) => "locations-section", (true,_) => "section", _ => "sidebar" };
   let view = cell(self.mtm(), outline, cell_id, !section);
   if section && let Some(group)=PinGroup::from_section(id) { self.configure_pin_button(&view,group); }
   if let Some(t)=view.textField(){t.setStringValue(&NSString::from_str(&label));t.setFont(Some(&*if section{NSFont::boldSystemFontOfSize(11.)}else{NSFont::systemFontOfSize(13.)}));t.setTextColor(Some(&super::theme::color(if section||unavailable{self.views().palette.get().side_muted}else{self.views().palette.get().side_ink})));}
   view.setToolTip(Some(&NSString::from_str(if section && PinGroup::from_section(id).is_some(){"폴더를 이 제목으로 끌어오거나 + 버튼으로 추가하세요."}else if unavailable{"iCloud Drive가 연결되어 있지 않습니다. 클릭하면 설정 방법을 확인할 수 있습니다."}else{&label})));
   if let Some(i)=view.imageView(){i.setImage(symbol(&icon).as_deref());i.setContentTintColor(Some(&super::theme::color(self.views().palette.get().side_ink)));}
   Some(Retained::into_super(view))
  })()}
  #[unsafe(method(outlineViewSelectionDidChange:))]fn outline_selected(&self,_:&NSNotification){self.activate_selected_sidebar(false);}
  #[unsafe(method(outlineView:shouldSelectItem:))]fn outline_selectable(&self,_:&NSOutlineView,item:&AnyObject)->bool{unsafe{node_id(Some(item)).and_then(|id|self.ivars().state.borrow().nodes.get(&id).map(|n|!n.section)).unwrap_or(false)}}
  #[unsafe(method(outlineViewItemDidExpand:))]fn sidebar_expanded(&self,_:&NSNotification){self.sidebar_expansion_changed();}
  #[unsafe(method(outlineViewItemDidCollapse:))]fn sidebar_collapsed(&self,_:&NSNotification){self.sidebar_expansion_changed();}

 }
 unsafe impl NSCollectionViewDataSource for Controller{
  #[unsafe(method(collectionView:numberOfItemsInSection:))]fn item_count(&self,_:&NSCollectionView,_:isize)->isize{self.with_tab(|t|t.visible.len()as isize)}
  #[unsafe(method_id(collectionView:itemForRepresentedObjectAtIndexPath:))]fn collection_item(&self,collection:&NSCollectionView,path:&NSIndexPath)->Retained<NSCollectionViewItem>{unsafe{let item=collection.makeItemWithIdentifier_forIndexPath(ns_string!("FileItem"),path);let _=item.view();if let Some(e)=self.entry_at(path.item()){if let Some(t)=item.textField(){t.setEditable(false);t.setSelectable(false);t.setBezeled(false);t.setDrawsBackground(false);t.setStringValue(&NSString::from_str(&e.name));t.setToolTip(Some(&NSString::from_str(&e.name)));}layout_file_item(&item);set_item_theme(&item,self.views().palette.get());
  if let Some(i)=item.imageView(){i.setImage(self.ivars().state.borrow_mut().images.icon(&e).as_deref());i.setToolTip(Some(&NSString::from_str(&e.name)));}item.view().setToolTip(Some(&NSString::from_str(&e.name)));}item}}
 }
 unsafe impl NSCollectionViewDelegate for Controller{
  #[unsafe(method(collectionView:willDisplayItem:forRepresentedObjectAtIndexPath:))]
  fn item_displayed(&self,_:&NSCollectionView,item:&NSCollectionViewItem,index:&NSIndexPath){unsafe{set_item_theme(item,self.views().palette.get());if let Some(image)=item.imageView()&&let Some(e)=self.entry_at(index.item()){image.setImage(self.ivars().state.borrow_mut().images.icon(&e).as_deref());}}}
  #[unsafe(method(collectionView:draggingSession:willBeginAtPoint:forItemsAtIndexPaths:))]fn grid_drag_begin(&self,grid:&NSCollectionView,_:&NSDraggingSession,_:NSPoint,indices:&NSSet<NSIndexPath>){self.ivars().state.borrow_mut().dragging=true;grid.setSelectionIndexPaths(indices);self.selection_changed();}
  #[unsafe(method(collectionView:draggingSession:endedAtPoint:dragOperation:))]fn grid_drag_end(&self,_:&NSCollectionView,_:&NSDraggingSession,_:NSPoint,_:NSDragOperation){self.drag_ended();}
  #[unsafe(method(collectionView:didSelectItemsAtIndexPaths:))]fn grid_selected(&self,_:&NSCollectionView,_:&NSSet<NSIndexPath>){self.selection_changed();}
  #[unsafe(method(collectionView:didDeselectItemsAtIndexPaths:))]fn grid_deselected(&self,_:&NSCollectionView,_:&NSSet<NSIndexPath>){self.selection_changed();}
  #[unsafe(method_id(collectionView:pasteboardWriterForItemAtIndexPath:))]fn grid_writer(&self,_:&NSCollectionView,path:&NSIndexPath)->Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>>{self.entry_at(path.item()).map(|e|unsafe{writer(&e.path)})}
  #[unsafe(method(collectionView:validateDrop:proposedIndexPath:dropOperation:))]unsafe fn grid_validate(&self,collection:&NSCollectionView,info:&ProtocolObject<dyn NSDraggingInfo>,mut index:std::ptr::NonNull<*mut NSIndexPath>,mut op:std::ptr::NonNull<NSCollectionViewDropOperation>)->NSDragOperation{
   let point=collection.convertPoint_fromView(info.draggingLocation(),None);
   let hit=collection.indexPathForItemAtPoint(point);
   let folder=hit.as_ref().and_then(|p|self.entry_at(p.item())).filter(|e|e.kind==Kind::Folder).map(|e|e.path);
   if let Some(path)=folder {
    *op.as_mut()=NSCollectionViewDropOperation::On;
    if let Some(hit)=hit{*index.as_mut()=Retained::autorelease_ptr(hit);}
    self.validate_drop_target(info,&path)
   }else{*op.as_mut()=NSCollectionViewDropOperation::Before;self.validate_drop_target(info,&self.current_path())}
  }
  #[unsafe(method(collectionView:acceptDrop:indexPath:dropOperation:))]fn grid_accept(&self,_:&NSCollectionView,info:&ProtocolObject<dyn NSDraggingInfo>,path:&NSIndexPath,op:NSCollectionViewDropOperation)->bool{unsafe{let target=if op==NSCollectionViewDropOperation::On{self.entry_at(path.item()).filter(|e|e.kind==Kind::Folder).map(|e|e.path)}else{None}.unwrap_or_else(||self.current_path());self.drop_files(info,target)}}

 }
 unsafe impl NSControlTextEditingDelegate for Controller{
  #[unsafe(method(controlTextDidChange:))]fn field_changed(&self,notification:&NSNotification){self.batch_text_changed(notification);}
  #[unsafe(method(controlTextDidEndEditing:))]fn rename_ended(&self,notification:&NSNotification){self.finish_rename(notification);}
 }
 unsafe impl NSTextFieldDelegate for Controller{}
 unsafe impl NSMenuDelegate for Controller {
  #[unsafe(method(menuNeedsUpdate:))]fn update_menu(&self,menu:&NSMenu){self.prepare_context_menu(menu);}
 }
 unsafe impl NSSharingServiceDelegate for Controller {
  #[unsafe(method_id(sharingService:sourceWindowForShareItems:sharingContentScope:))]
  unsafe fn sharing_window(&self,_:&NSSharingService,_:&NSArray,mut scope:std::ptr::NonNull<NSSharingContentScope>)->Option<Retained<NSWindow>>{*scope.as_mut()=NSSharingContentScope::Item;Some(self.views().window.clone())}
  #[unsafe(method(sharingService:didFailToShareItems:error:))]
  fn sharing_failed(&self,_:&NSSharingService,_:&NSArray,error:&NSError){if error.code()!=3072&&error.code()!=-128{let _=self.ivars().runtime.sender.send(Event::Message{window:self.id(),title:"AirDrop을 시작할 수 없습니다".into(),detail:error.localizedDescription().to_string()});}}
 }
 unsafe impl QLPreviewPanelDataSource for Controller{
  #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]unsafe fn preview_count(&self,_:Option<&QLPreviewPanel>)->isize{self.ivars().state.borrow().quick_urls.len()as isize}
  #[unsafe(method_id(previewPanel:previewItemAtIndex:))]unsafe fn preview_item(&self,_:Option<&QLPreviewPanel>,index:isize)->Option<Retained<ProtocolObject<dyn QLPreviewItem>>>{self.ivars().state.borrow().quick_urls.get(index as usize).cloned().map(ProtocolObject::from_retained)}
 }
 unsafe impl NSMenuItemValidation for Controller{
  #[unsafe(method(validateMenuItem:))]fn validate_menu(&self,item:&NSMenuItem)->bool{(||unsafe{let Some(a)=item.action()else{return true};let selected=self.selected_paths().len();
  if [sel!(clearFileSelection:),sel!(invertFileSelection:)].contains(&a){return !self.views().window.firstResponder().is_some_and(|r|r.isKindOfClass(NSTextView::class()))&&self.views().window.attachedSheet().is_none()&&self.with_tab(|t|!t.visible.is_empty());}
  if a==sel!(cut:)&&self.ivars().state.borrow().prefs.keyboard==KeyboardProfile::Finder{return false;}
  if [sel!(cutFiles:),sel!(copyFiles:),sel!(trashFiles:),sel!(openSelected:)].contains(&a){return selected>0;}
  if a==sel!(renameFile:){let editing=self.views().window.firstResponder().is_some_and(|r|r.isKindOfClass(NSTextView::class()));return selected==1&&self.views().window.attachedSheet().is_none()&&!(editing&&!item.keyEquivalent().to_string().is_empty());}
  if [sel!(newFolder:),sel!(newDocumentFromMenu:),sel!(newEmptyFile:)].contains(&a){return self.with_tab(|t| t.can_write())&&self.views().window.attachedSheet().is_none();}
  if [sel!(compressZip:),sel!(compress7z:)].contains(&a){return selected>0&&self.with_tab(|t| t.can_write())&&self.views().window.attachedSheet().is_none();}
  if a==sel!(sendBluetooth:){return self.can_send_bluetooth();}
  if a==sel!(sendAirDrop:){return selected>0;}
  if a==sel!(batchRename:){let editing=self.views().window.firstResponder().is_some_and(|r|r.isKindOfClass(NSTextView::class()));return selected>1&&!editing&&self.with_tab(|t|t.can_write())&&self.views().window.attachedSheet().is_none();}
  if [sel!(duplicateFiles:),sel!(openWith:)].contains(&a){let editing=self.views().window.firstResponder().is_some_and(|r|r.isKindOfClass(NSTextView::class()));return !editing&&self.views().window.attachedSheet().is_none()&&if a==sel!(openWith:){self.can_open_with()}else{selected>0&&self.with_tab(|t|t.can_write())};}
  if a==sel!(cancelServerConnection:){return self.ivars().state.borrow().mount_request.is_some();}
  if [sel!(goToFolder:),sel!(connectToServer:)].contains(&a){return self.views().window.attachedSheet().is_none();}
  if a==sel!(extractArchive:){return selected>0&&self.selected_paths().iter().all(|p|crate::core::archive::is_archive(p));}
  if a==sel!(toggleInputDiagnostics:){item.setState(if self.ivars().state.borrow().input_diagnostics{NSControlStateValueOn}else{NSControlStateValueOff});return true;}
  if a==sel!(pasteFiles:){return NSPasteboard::generalPasteboard().availableTypeFromArray(&NSArray::from_slice(&[NSPasteboardTypeFileURL])).is_some();}
  if a==sel!(undoOperation:){return self.ivars().runtime.history.lock().unwrap().iter().any(Journal::can_undo);}true})()}
 }
 impl Controller{
  #[unsafe(method(handleSelectionMouseEvent:))]fn selection_mouse_event(&self,event:&NSEvent)->bool{super::selection::handle_mouse(&self.views().table,&self.views().collection,event)}
  #[unsafe(method(fileSelectionChanged))]fn file_selection_changed(&self){self.selection_changed();}
  #[unsafe(method(fileSelectionTracking:))]fn file_selection_tracking(&self,active:bool){let mut state=self.ivars().state.borrow_mut();state.selecting=active;if !active{state.drag_refresh_due=Some(Instant::now());}}
  #[unsafe(method(manageStorage:))]fn manage_storage(&self,_:Option<&AnyObject>){self.open_storage_settings();}
  #[unsafe(method(sidebarVolumeInfo:))]fn sidebar_volume_info(&self,_:Option<&AnyObject>){self.show_volume_info();}
  #[unsafe(method(clearFileSelection:))]fn clear_file_selection(&self,_:Option<&AnyObject>){self.change_selection(false);}
  #[unsafe(method(invertFileSelection:))]fn invert_file_selection(&self,_:Option<&AnyObject>){self.change_selection(true);}
  #[unsafe(method(usesWindowsSelection))]fn windows_selection(&self)->bool{self.ivars().state.borrow().prefs.keyboard==KeyboardProfile::Windows}
  #[unsafe(method(handleWindowKeyEvent:))]fn window_key_event(&self,event:&NSEvent)->bool{self.record_input_diagnostic(event);self.handle_key(event)}
  #[unsafe(method(newTab:))]fn a_new_tab(&self,_:Option<&AnyObject>){self.new_tab(self.current_path());}
  #[unsafe(method(newWindow:))]fn a_new_window(&self,_:Option<&AnyObject>){super::new_window(Some(self.current_path()));}
  #[unsafe(method(selectTab:))]fn a_select_tab(&self,sender:&NSButton){self.select_tab(sender.tag()as u64);}
  #[unsafe(method(closeTabAt:))]fn a_close_tab(&self,sender:&NSButton){self.close_tab(Some(sender.tag()as u64));}
  #[unsafe(method(back:))]fn a_back(&self,_:Option<&AnyObject>){self.go_back(false);}
  #[unsafe(method(forward:))]fn a_forward(&self,_:Option<&AnyObject>){self.go_back(true);}
  #[unsafe(method(up:))]fn a_up(&self,_:Option<&AnyObject>){if let Some(p)=self.current_path().parent(){self.navigate(p.into(),true);}}
  #[unsafe(method(refresh:))]fn a_refresh(&self,_:Option<&AnyObject>){self.load(true);}
  #[unsafe(method(breadcrumbClicked:))]fn breadcrumb_clicked(&self,_:Option<&AnyObject>){if let Some(item)=self.views().breadcrumb.clickedPathItem()&& let Some(url)=item.URL(){self.navigate(platform::url_path(&url),true);}}
  #[unsafe(method(editAddress:))]fn edit_address(&self,_:Option<&AnyObject>){self.focus_address();}
  #[unsafe(method(copy:))]fn native_copy(&self,_:Option<&AnyObject>){self.copy(false);}
  #[unsafe(method(cut:))]fn native_cut(&self,_:Option<&AnyObject>){if self.ivars().state.borrow().prefs.keyboard==KeyboardProfile::Windows{self.copy(true);}}
  #[unsafe(method(paste:))]fn native_paste(&self,_:Option<&AnyObject>){self.paste(false);}
  #[unsafe(method(undo:))]fn native_undo(&self,_:Option<&AnyObject>){self.ivars().runtime.undo(self.id());}
  #[unsafe(method(selectAll:))]fn native_select_all(&self,_:Option<&AnyObject>){unsafe{if self.ivars().state.borrow().prefs.view==ViewMode::Details{self.views().table.selectAll(None);}else{self.views().collection.selectAll(None);}self.selection_changed();}}
  #[unsafe(method(addressEntered:))]
  fn a_address(&self,_:Option<&AnyObject>){
      let text=self.views().address.stringValue().to_string();
      if self.open_server_address(&text){self.update_navigation();return;}
      let path=if text=="~"{super::home_dir()}else if let Some(rest)=text.strip_prefix("~/"){super::home_dir().join(rest)}else{PathBuf::from(text)};
      let path=if path.is_absolute(){path}else{self.current_path().join(path)};
      self.navigate(path,true);
      self.focus_list();
  }
  #[unsafe(method(searchChanged:))]fn a_search(&self,_:Option<&AnyObject>){self.search_changed();}
  #[unsafe(method(searchScopeChanged:))]fn a_scope(&self,_:Option<&AnyObject>){self.search_changed();}
  #[unsafe(method(openSelected:))]fn a_open(&self,_:Option<&AnyObject>){self.open_selected();}
  #[unsafe(method(openSidebar:))]fn a_sidebar(&self,_:Option<&AnyObject>){self.activate_selected_sidebar(true);}
  #[unsafe(method(openInNewTab:))]fn a_open_tab(&self,_:Option<&AnyObject>){for p in self.selected_paths(){self.new_tab(p);}}
  #[unsafe(method(newFolder:))]
  fn a_new_folder(&self, _: Option<&AnyObject>) {
      self.create_item(OperationKind::CreateFolder, None);
  }
  #[unsafe(method(newDocumentFromMenu:))]
  fn a_new_document(&self, sender: &NSMenuItem) {
      if let Some(&template) = FileTemplate::PRESETS.get(sender.tag() as usize) {
          self.create_item(OperationKind::CreateFile(template), None);
      }
  }
  #[unsafe(method(newEmptyFile:))]
  fn a_new_empty_file(&self, _: Option<&AnyObject>) {
      self.choose_file_name();
  }
  #[unsafe(method(batchRename:))]fn a_batch_rename(&self,_:Option<&AnyObject>){self.show_batch_rename();}
  #[unsafe(method(batchRenameModeChanged:))]fn a_batch_mode(&self,_:Option<&AnyObject>){self.batch_mode_changed();}
  #[unsafe(method(refreshRenamePreview:))]fn a_batch_refresh(&self,_:Option<&AnyObject>){self.queue_batch_preview();}
  #[unsafe(method(duplicateFiles:))]fn a_duplicate(&self,_:Option<&AnyObject>){self.duplicate_selected();}
  #[unsafe(method(openWith:))]fn a_open_with(&self,_:Option<&AnyObject>){self.open_with_dialog();}
  #[unsafe(method(connectToServer:))]fn connect_server(&self,_:Option<&AnyObject>){self.connect_server_dialog();}
  #[unsafe(method(cancelServerConnection:))]fn cancel_connection(&self,_:Option<&AnyObject>){self.cancel_mount();}
  #[unsafe(method(disconnectSidebarServer:))]fn disconnect_server(&self,_:Option<&AnyObject>){self.disconnect_context_server();}
  #[unsafe(method(goToFolder:))]fn a_go_folder(&self,_:Option<&AnyObject>){self.go_to_folder_dialog();}
  #[unsafe(method(compressZip:))]fn a_compress_zip(&self,_:Option<&AnyObject>){self.compress_dialog(crate::core::archive::ArchiveFormat::Zip);}
  #[unsafe(method(compress7z:))]fn a_compress_7z(&self,_:Option<&AnyObject>){self.compress_dialog(crate::core::archive::ArchiveFormat::SevenZip);}
  #[unsafe(method(sendBluetooth:))]fn a_send_bluetooth(&self,_:Option<&AnyObject>){self.send_bluetooth();}
  #[unsafe(method(sendAirDrop:))]fn a_send_airdrop(&self,_:Option<&AnyObject>){self.send_airdrop();}
  #[unsafe(method(cutFiles:))]fn a_cut(&self,_:Option<&AnyObject>){self.copy(true);}
  #[unsafe(method(copyFiles:))]fn a_copy(&self,_:Option<&AnyObject>){self.copy(false);}
  #[unsafe(method(pasteFiles:))]fn a_paste(&self,_:Option<&AnyObject>){self.paste(false);}
  #[unsafe(method(renameFile:))]fn a_rename(&self,_:Option<&AnyObject>){self.begin_rename();}
  #[unsafe(method(toggleInputDiagnostics:))]fn toggle_input_diagnostics(&self,_:Option<&AnyObject>){let enabled=!self.ivars().state.borrow().input_diagnostics;{let mut s=self.ivars().state.borrow_mut();s.input_diagnostics=enabled;s.diagnostic_events.clear();s.diagnostic_count=0;}self.update_status();}
  #[unsafe(method(openAccessibilitySettings:))]fn open_accessibility_settings(&self,_:Option<&AnyObject>){unsafe{if let Some(url)=NSURL::URLWithString(ns_string!("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")){NSWorkspace::sharedWorkspace().openURL(&url);}}}
  #[unsafe(method(trashFiles:))]fn a_trash(&self,_:Option<&AnyObject>){self.trash();}
  #[unsafe(method(undoOperation:))]fn a_undo(&self,_:Option<&AnyObject>){self.ivars().runtime.undo(self.id());}
  #[unsafe(method(toggleView:))]fn a_view(&self,_:Option<&AnyObject>){self.toggle_pref("view");}
  #[unsafe(method(togglePreview:))]fn a_preview(&self,_:Option<&AnyObject>){self.toggle_pref("preview");}
  #[unsafe(method(toggleHidden:))]fn a_hidden(&self,_:Option<&AnyObject>){self.toggle_pref("hidden");}
  #[unsafe(method(toggleCompact:))]fn a_compact(&self,_:Option<&AnyObject>){self.toggle_pref("compact");}
  #[unsafe(method(sortByName:))]fn a_sort_name(&self,_:Option<&AnyObject>){self.toggle_sort(SortField::Name);}
  #[unsafe(method(sortByDate:))]fn a_sort_date(&self,_:Option<&AnyObject>){self.toggle_sort(SortField::Modified);}
  #[unsafe(method(sortBySize:))]fn a_sort_size(&self,_:Option<&AnyObject>){self.toggle_sort(SortField::Size);}
  #[unsafe(method(settingsFolderAccess:))]fn a_settings_access(&self,_:Option<&AnyObject>){if let Some(panel)=self.ivars().state.borrow().appearance_panel.as_ref(){unsafe{panel.window().orderOut(None);}}self.folder_access_settings();}
  #[unsafe(method(generalSettings:))]fn a_general_settings(&self,_:Option<&AnyObject>){if let Some(panel)=self.ivars().state.borrow().appearance_panel.as_ref(){unsafe{panel.window().orderOut(None);}}self.general_settings();}
  #[unsafe(method(settings:))]fn a_settings(&self,_:Option<&AnyObject>){self.settings();}
  #[unsafe(method(showFolderAccess:))]fn a_access_settings(&self,_:Option<&AnyObject>){self.folder_access_settings();}
  #[unsafe(method(dismissAccessIntro:))]fn a_access_dismiss(&self,_:Option<&AnyObject>){self.dismiss_access_intro();}
  #[unsafe(method(chooseCurrentAccess:))]fn a_access_current(&self,_:Option<&AnyObject>){self.choose_access_folders(Some(self.current_path()));}
  #[unsafe(method(retryFolderAccess:))]fn a_access_retry(&self,_:Option<&AnyObject>){self.load(false);}
  #[unsafe(method(folderErrorDetails:))]fn a_access_details(&self,_:Option<&AnyObject>){self.folder_error_details();}
  #[unsafe(method(openFilesPrivacy:))]fn a_files_privacy(&self,_:Option<&AnyObject>){self.open_files_privacy(false);}
  #[unsafe(method(openFullDiskPrivacy:))]fn a_full_disk_privacy(&self,_:Option<&AnyObject>){self.open_files_privacy(true);}
  #[unsafe(method(forgetFolderAccess:))]fn a_forget_access(&self,_:Option<&AnyObject>){if let Err(error)=self.ivars().runtime.forget_access(self.id()){self.notice(error,None);}}
  #[unsafe(method(revealPermissionApp:))]fn a_reveal_permission_app(&self,_:Option<&AnyObject>){self.reveal_permission_app();}
  #[unsafe(method(detailsView:))]fn details_view(&self,_:Option<&AnyObject>){self.set_view(ViewMode::Details,None);}
  #[unsafe(method(smallIcons:))]fn small_icons(&self,_:Option<&AnyObject>){self.set_view(ViewMode::Icons,Some(IconSize::Small));}
  #[unsafe(method(mediumIcons:))]fn medium_icons(&self,_:Option<&AnyObject>){self.set_view(ViewMode::Icons,Some(IconSize::Medium));}
  #[unsafe(method(largeIcons:))]fn large_icons(&self,_:Option<&AnyObject>){self.set_view(ViewMode::Icons,Some(IconSize::Large));}
  #[unsafe(method(iconSizeChanged:))]fn icon_size_changed(&self,sender:&NSPopUpButton){self.set_view(ViewMode::Icons,Some(IconSize::from_index(sender.indexOfSelectedItem())));}
  #[unsafe(method(copyAddress:))]fn copy_address(&self,_:Option<&AnyObject>){self.copy_text(&self.current_path().to_string_lossy());self.notice("폴더 경로를 복사했습니다",None);}
  #[unsafe(method(undoNotice:))]fn undo_notice(&self,_:Option<&AnyObject>){let id=self.ivars().state.borrow().notice.as_ref().and_then(|n|n.2.clone());if id.is_some(){self.ivars().runtime.undo_selected(self.id(),id);}}
  #[unsafe(method(pickFavoriteFolders:))]
  fn pick_favorite_folders(&self, _:Option<&AnyObject>){self.pick_pin_folders(PinGroup::Favorites);}
  #[unsafe(method(pickLocationFolders:))]
  fn pick_location_folders(&self, _:Option<&AnyObject>){self.pick_pin_folders(PinGroup::Locations);}
  #[unsafe(method(pinSidebarFolder:))]
  fn pin_sidebar_folder(&self, item:&NSMenuItem) {
      if let Some(group)=PinGroup::from_section(item.tag() as u64)
          && let Some(path)=self.sidebar_context_path(){self.pin_paths(group,&[path]);}
  }
  #[unsafe(method(renameSidebarShortcut:))]
  fn rename_sidebar_shortcut(&self, _:Option<&AnyObject>){self.rename_context_shortcut();}
  #[unsafe(method(retargetSidebarShortcut:))]
  fn retarget_sidebar_shortcut(&self, _:Option<&AnyObject>){self.retarget_context_shortcut();}
  #[unsafe(method(unpinSidebarShortcut:))]
  fn unpin_sidebar_shortcut(&self, _:Option<&AnyObject>){self.unpin_context_shortcut();}
  #[unsafe(method(addSelectedLocations:))]
  fn add_selected_locations(&self, _:Option<&AnyObject>){self.pin_selected(PinGroup::Locations);}
  #[unsafe(method(pinCurrentLocation:))]
  fn pin_current_location(&self, _:Option<&AnyObject>){self.pin_paths(PinGroup::Locations,&[self.current_path()]);}
  #[unsafe(method(sidebarOpenTab:))]fn sidebar_tab(&self,_:Option<&AnyObject>){if let Some(path)=self.sidebar_context_path(){self.new_tab(path);}}
  #[unsafe(method(sidebarHide:))]fn sidebar_hide(&self,_:Option<&AnyObject>){self.hide_sidebar_item();}
  #[unsafe(method(sidebarMoveUp:))]fn sidebar_up(&self,_:Option<&AnyObject>){self.move_sidebar_item(false);}
  #[unsafe(method(sidebarMoveDown:))]fn sidebar_down(&self,_:Option<&AnyObject>){self.move_sidebar_item(true);}
  #[unsafe(method(sidebarToggle:))]fn sidebar_toggle(&self,sender:&NSMenuItem){self.toggle_sidebar_item(&sender.representedObject().and_then(|o|o.downcast::<NSString>().ok()).map(|s|s.to_string()).unwrap_or_default());}
  #[unsafe(method(sidebarReset:))]fn sidebar_reset(&self,_:Option<&AnyObject>){let mut p=self.ivars().runtime.preferences.lock().unwrap().clone();p.sidebar=SidebarPreferences::default();self.commit_preferences(p);}
  #[unsafe(method(quickLook:))]fn a_quick_look(&self,_:Option<&AnyObject>){self.quick_look();}
  #[unsafe(method(showProperties:))]fn a_properties(&self,_:Option<&AnyObject>){self.show_properties();}
  #[unsafe(method(extractArchive:))]fn a_extract(&self,_:Option<&AnyObject>){for path in self.selected_paths(){self.extract_archive(path);}}
  #[unsafe(method(addSelectedFavorites:))]fn a_selected_favorites(&self,_:Option<&AnyObject>){self.pin_selected(PinGroup::Favorites);}
  #[unsafe(method(showHistory:))]fn a_history(&self,_:Option<&AnyObject>){self.show_history();}
  #[unsafe(method(historyChanged:))]fn history_changed(&self,_:Option<&AnyObject>){self.update_history();}
  #[unsafe(method(cancelAllJobs:))]fn cancel_all(&self,_:Option<&AnyObject>){for cancel in self.ivars().runtime.jobs.lock().unwrap().values(){cancel.cancel();}super::cancel_size_work();}
  #[unsafe(method(toggleJobs:))]fn a_jobs(&self,_:Option<&AnyObject>){self.ivars().state.borrow_mut().job_reveal_due=None;unsafe{self.views().jobs.setHidden(!self.views().jobs.isHidden());self.views().layout();}}
  #[unsafe(method(cancelJob:))]fn a_cancel(&self,_:Option<&AnyObject>){let active=self.ivars().state.borrow().active_job;if let Some(id)=active{self.ivars().runtime.cancel(id);}else{self.ivars().state.borrow_mut().job_reveal_due=None;self.views().jobs.setHidden(true);unsafe{self.views().layout();}}}
  #[unsafe(method(retryJob:))]fn a_retry(&self,_:Option<&AnyObject>){let req=self.ivars().state.borrow().last_job.as_ref().filter(|j|j.can_retry()).map(retry_request);if let Some(r)=req{self.start_operation(r);}}
  #[unsafe(method(addFavorite:))]fn a_favorite(&self,_:Option<&AnyObject>){self.pin_paths(PinGroup::Favorites,&[self.current_path()]);}
  #[unsafe(method(revealInFinder:))]fn a_reveal(&self,_:Option<&AnyObject>){unsafe{let urls:Vec<_>=self.selected_paths().iter().map(|p|platform::file_url(p)).collect();NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));}}
  #[unsafe(method(copyPath:))]fn a_path(&self,_:Option<&AnyObject>){let p=self.selected_paths().into_iter().map(|p|p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");self.copy_text(&p);self.notice("선택 항목 경로를 복사했습니다",None);}
  #[unsafe(method(calculateSize:))]fn a_size(&self,_:Option<&AnyObject>){self.calculate_size();}
  #[unsafe(method(acceptsPreviewPanelControl:))]fn accepts_preview(&self,_:&QLPreviewPanel)->bool{true}
  #[unsafe(method(beginPreviewPanelControl:))]fn begin_preview(&self,panel:&QLPreviewPanel){unsafe{panel.setDataSource(Some(ProtocolObject::from_ref(self)));}}
  #[unsafe(method(endPreviewPanelControl:))]fn end_preview(&self,panel:&QLPreviewPanel){unsafe{panel.setDataSource(None);}}
 }
);

unsafe fn node_id(item: Option<&AnyObject>) -> Option<u64> {
    item.and_then(|i| i.downcast_ref::<OutlineNode>())
        .map(OutlineNode::id)
}

impl Controller {
    pub fn id(&self) -> u64 {
        self.ivars().id
    }
    pub fn views(&self) -> &ExplorerViews {
        self.ivars().views.get().unwrap()
    }
    pub fn with_tab<R>(&self, f: impl FnOnce(&Tab) -> R) -> R {
        let s = self.ivars().state.borrow();
        f(&s.tabs[s.active])
    }
    pub fn with_tab_mut<R>(&self, f: impl FnOnce(&mut Tab) -> R) -> R {
        let mut s = self.ivars().state.borrow_mut();
        let active = s.active;
        f(&mut s.tabs[active])
    }
    pub fn current_path(&self) -> PathBuf {
        self.with_tab(|t| t.path.clone())
    }
    pub fn entry_at(&self, row: isize) -> Option<Entry> {
        if row < 0 {
            return None;
        }
        self.with_tab(|t| {
            let e = t.entries.get(*t.visible.get(row as usize)?)?;
            Some(t.metadata.get(&e.id).unwrap_or(e).clone())
        })
    }
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.with_tab(|t| {
            t.visible
                .iter()
                .filter_map(|&i| t.entries.get(i))
                .filter(|e| t.selected.contains(&e.id))
                .map(|e| e.path.clone())
                .collect()
        })
    }
    pub unsafe fn new(
        mtm: MainThreadMarker,
        runtime: Arc<Runtime>,
        paths: Vec<SavedTab>,
    ) -> Retained<Self> {
        Self::construct(mtm, runtime, paths, true)
    }
    #[cfg(feature = "native-ui-tests")]
    unsafe fn new_hidden(
        mtm: MainThreadMarker,
        runtime: Arc<Runtime>,
        paths: Vec<SavedTab>,
    ) -> Retained<Self> {
        Self::construct(mtm, runtime, paths, false)
    }
    unsafe fn construct(
        mtm: MainThreadMarker,
        runtime: Arc<Runtime>,
        paths: Vec<SavedTab>,
        show_window: bool,
    ) -> Retained<Self> {
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|tab| valid_path_input(&path_from_bytes(tab.path.clone())))
            .collect();
        let prefs = runtime.preferences.lock().unwrap().clone();
        let id = runtime.next_id();
        let tabs = if paths.is_empty() {
            vec![Tab::new(runtime.next_id(), super::home_dir())]
        } else {
            paths
                .into_iter()
                .map(|p| {
                    let mut t = Tab::new(runtime.next_id(), path_from_bytes(p.path));
                    t.sort = p.sort;
                    t.descending = p.descending;
                    t
                })
                .collect()
        };
        let obj = Self::alloc(mtm).set_ivars(ControllerIvars {
            id,
            runtime,
            views: OnceCell::new(),
            state: RefCell::new(State {
                appearance_panel: None,
                access_return: None,
                pending_access: None,
                access_list: None,
                size_cancel: Cancellation::default(),
                tabs,
                active: 0,
                prefs,
                nodes: HashMap::new(),
                roots: vec![],
                images: ImageCache::default(),
                last_job: None,
                active_job: None,
                renaming: None,
                rename_editor: None,
                last_clipboard: None,
                paste_moves: HashMap::new(),
                #[cfg(feature = "native-ui-tests")]
                test_clipboard: None,
                quick_urls: vec![],
                preview_view: None,
                preview_path: None,
                history: None,
                reloading: false,
                theme_name: String::new(),
                last_theme_check: Instant::now(),
                thumbnail_requests: HashMap::new(),
                thumbnail_attempted: HashSet::new(),
                locations: vec![],
                rebuilding_sidebar: false,
                sidebar_context: None,
                mount_request: None,
                job_reveal_due: None,
                notice: None,
                extraction_jobs: HashMap::new(),
                creation_jobs: HashMap::new(),
                pending_creation: None,
                sharing_service: None,
                batch_dialog: None,
                application_dialog: None,
                folder_request: 0,
                dragging: false,
                selecting: false,
                selection_events: VecDeque::new(),
                drag_refresh_due: None,
                #[cfg(feature = "native-ui-tests")]
                test_drag_modifiers: None,
                input_diagnostics: false,
                diagnostic_events: VecDeque::new(),
                diagnostic_count: 0,
            }),
        });
        let obj: Retained<Self> = msg_send![super(obj), init];
        let views = design::build(mtm, &obj);
        obj.ivars().views.set(views).ok().unwrap();
        let v = obj.views();
        v.window.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
        // Preserve the standard window -> controller -> application chain.
        obj.setNextResponder(Some(&NSApplication::sharedApplication(mtm)));
        v.window.setNextResponder(Some(&obj));
        v.table.setDataSource(Some(ProtocolObject::from_ref(&*obj)));
        v.table.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
        v.table.setAllowsMultipleSelection(true);
        for col in v.table.tableColumns().iter() {
            col.setSortDescriptorPrototype(Some(
                &NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&col.identifier()), true),
            ));
        }
        v.sidebar
            .setDataSource(Some(ProtocolObject::from_ref(&*obj)));
        v.sidebar.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
        v.collection
            .registerClass_forItemWithIdentifier(Some(FileItem::class()), ns_string!("FileItem"));
        v.collection
            .setDataSource(Some(ProtocolObject::from_ref(&*obj)));
        v.collection
            .setDelegate(Some(ProtocolObject::from_ref(&*obj)));
        let types = NSArray::from_slice(&[NSPasteboardTypeFileURL]);
        v.table.registerForDraggedTypes(&types);
        v.collection.registerForDraggedTypes(&types);
        v.sidebar.registerForDraggedTypes(&types);
        v.table.setDraggingSourceOperationMask_forLocal(
            NSDragOperation::Copy | NSDragOperation::Move,
            true,
        );
        v.table
            .setDraggingSourceOperationMask_forLocal(NSDragOperation::Copy, false);
        v.collection.setDraggingSourceOperationMask_forLocal(
            NSDragOperation::Copy | NSDragOperation::Move,
            true,
        );
        v.collection
            .setDraggingSourceOperationMask_forLocal(NSDragOperation::Copy, false);
        for (view, title) in [(&*v.table, "files"), (&*v.sidebar, "sidebar")] {
            let menu = NSMenu::new(mtm);
            menu.setTitle(&NSString::from_str(title));
            menu.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
            view.setMenu(Some(&menu));
        }
        if let Some(scroll) = v.sidebar.enclosingScrollView() {
            let background = NSMenu::new(mtm);
            background.setTitle(ns_string!("sidebar-background"));
            background.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
            scroll.contentView().setMenu(Some(&background));
            scroll.setMenu(Some(&background));
        }
        let menu = NSMenu::new(mtm);
        menu.setTitle(ns_string!("icons"));
        menu.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
        v.collection.setMenu(Some(&menu));
        let background = NSMenu::new(mtm);
        background.setTitle(ns_string!("background"));
        background.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
        for scroll in [&v.list_scroll, &v.icon_scroll] {
            scroll.contentView().setMenu(Some(&background));
            scroll.setMenu(Some(&background));
        }
        v.empty_label.setMenu(Some(&background));
        let mounts = obj
            .ivars()
            .runtime
            .mounted_locations
            .lock()
            .unwrap()
            .clone();
        obj.rebuild_sidebar(mounts);
        let initial_prefs = obj.ivars().state.borrow().prefs.clone();
        obj.apply_preferences(initial_prefs, false);
        obj.refresh_tabs();
        obj.load(false);
        v.layout();
        if show_window {
            v.window.makeKeyAndOrderFront(None);
            obj.focus_list();
        }
        obj
    }
}

impl Controller {
    pub fn save_tabs(&self) {
        let tabs = self
            .ivars()
            .state
            .borrow()
            .tabs
            .iter()
            .map(|t| SavedTab {
                path: path_bytes(&t.path),
                sort: t.sort,
                descending: t.descending,
            })
            .collect();
        self.ivars().runtime.save_tabs(tabs);
    }
    pub fn navigate(&self, path: PathBuf, history: bool) {
        if !valid_path_input(&path) {
            self.notice("경로에 사용할 수 없는 문자가 있습니다", None);
            self.update_navigation();
            return;
        }
        super::selection::finish(&self.views().table, &self.views().collection, false);
        self.ivars().state.borrow_mut().folder_request = 0;
        self.cancel_inline_rename();
        self.with_tab_mut(|t| {
            if history && path != t.path {
                t.back.push(t.path.clone());
                t.forward.clear();
            }
            t.path = path;
            t.query.clear();
            t.selected.clear();
            t.scroll = [None; 2];
        });
        self.load(false);
        self.refresh_tabs();
        self.save_tabs();
    }
    pub fn new_tab(&self, path: PathBuf) {
        if !valid_path_input(&path) {
            self.notice("경로에 사용할 수 없는 문자가 있습니다", None);
            self.update_navigation();
            return;
        }
        self.remember_scroll();
        super::selection::finish(&self.views().table, &self.views().collection, false);
        self.ivars().state.borrow_mut().folder_request = 0;
        self.cancel_inline_rename();
        let id = self.ivars().runtime.next_id();
        {
            let mut s = self.ivars().state.borrow_mut();
            s.tabs.push(Tab::new(id, path));
            s.active = s.tabs.len() - 1;
        }
        self.refresh_tabs();
        self.load(false);
        self.save_tabs();
    }
    pub fn select_tab(&self, id: u64) {
        super::selection::finish(&self.views().table, &self.views().collection, false);
        self.ivars().state.borrow_mut().folder_request = 0;
        if self.with_tab(|t| t.id == id) {
            self.focus_list();
            return;
        }
        self.cancel_inline_rename();
        self.cancel_thumbnails();
        self.remember_scroll();
        {
            let mut s = self.ivars().state.borrow_mut();
            if let Some(index) = s.tabs.iter().position(|t| t.id == id) {
                s.active = index;
            } else {
                return;
            }
        }
        self.refresh_tabs();
        self.update_navigation();
        self.reload();
        self.restore_scroll();
        if self.with_tab(|t| t.generation == 0) {
            self.load(false);
        }
        self.save_tabs();
    }
    pub fn close_tab(&self, id: Option<u64>) {
        let id = id.unwrap_or_else(|| self.with_tab(|t| t.id));
        let Some(index) = self
            .ivars()
            .state
            .borrow()
            .tabs
            .iter()
            .position(|t| t.id == id)
        else {
            return;
        };
        let closing_active = self.with_tab(|t| t.id == id);
        if closing_active {
            self.cancel_inline_rename();
        }
        let last = self.ivars().state.borrow().tabs.len() == 1;
        if last {
            unsafe {
                self.views().window.performClose(None);
            }
            return;
        }
        {
            let mut s = self.ivars().state.borrow_mut();
            s.tabs[index].cancel.cancel();
            s.tabs[index].filter_cancel.cancel();
            s.tabs.remove(index);
            if index < s.active {
                s.active -= 1;
            }
            s.active = s.active.min(s.tabs.len() - 1);
        }
        self.ivars().runtime.unwatch(self.id(), Some(id));
        self.refresh_tabs();
        if closing_active {
            if self.with_tab(|t| t.generation == 0) {
                self.load(false);
            } else {
                self.update_navigation();
                self.reload();
            }
            self.restore_scroll();
            self.focus_list();
        }
        self.save_tabs();
    }
    fn go_back(&self, forward: bool) {
        self.ivars().state.borrow_mut().folder_request = 0;
        let path = self.with_tab_mut(|t| {
            if forward {
                let p = t.forward.pop()?;
                t.back.push(t.path.clone());
                Some(p)
            } else {
                let p = t.back.pop()?;
                t.forward.push(t.path.clone());
                Some(p)
            }
        });
        if let Some(p) = path {
            self.navigate(p, false);
        }
    }
    pub fn load(&self, refresh: bool) {
        super::selection::finish(&self.views().table, &self.views().collection, false);
        self.cancel_thumbnails();
        let hidden = self.ivars().state.borrow().prefs.hidden;
        let (id, generation, path, sort, desc, query, cancel) = self.with_tab_mut(|t| {
            t.cancel.cancel();
            t.filter_cancel.cancel();
            t.cancel = Cancellation::default();
            t.generation += 1;
            t.load_started = Instant::now();
            t.first_displayed = false;
            t.loading = true;
            t.refreshing = refresh && !t.entries.is_empty();
            t.error = None;
            t.metadata.clear();
            t.requested.clear();
            if !t.refreshing {
                t.entries = Arc::new(vec![]);
                t.visible.clear();
            }
            t.filter_due = None;
            t.watch_due = None;
            (
                t.id,
                t.generation,
                t.path.clone(),
                t.sort,
                t.descending,
                if t.recursive && !t.query.is_empty() {
                    Some(t.query.clone())
                } else {
                    None
                },
                t.cancel.clone(),
            )
        });
        self.update_navigation();
        if !refresh {
            self.reload();
        } else {
            self.update_status();
        }
        self.ivars().runtime.scan(ScanRequest {
            window: self.id(),
            tab: id,
            generation,
            path,
            hidden,
            sort,
            descending: desc,
            query,
            cancel,
        });
    }
    unsafe fn update_breadcrumb(&self) {
        let control = &self.views().breadcrumb;
        control.setURL(Some(&platform::file_url(&self.current_path())));
        let items = control.pathItems();
        let maximum = if control.frame().size.width < 350. {
            3
        } else {
            5
        };
        if items.len() > maximum {
            let start = items.len() - (maximum - 1);
            let prefix = items.objectAtIndex(start - 1);
            prefix.setTitle(ns_string!("…"));
            prefix.setImage(None);
            let mut shown = vec![prefix];
            shown.extend(items.iter().skip(start));
            control.setPathItems(&NSArray::from_retained_slice(&shown));
        }
    }
    fn update_navigation(&self) {
        unsafe {
            let (path, query, recursive) =
                self.with_tab(|t| (t.path.clone(), t.query.clone(), t.recursive));
            self.views()
                .address
                .setStringValue(&NSString::from_str(&path.to_string_lossy()));
            self.update_breadcrumb();
            self.views().breadcrumb.setHidden(false);
            self.views().address.setHidden(true);
            self.views()
                .address
                .setToolTip(Some(ns_string!("경로를 직접 입력할 수 있습니다 · ⌘L")));
            self.views()
                .search
                .setStringValue(&NSString::from_str(&query));
            self.views()
                .search
                .setPlaceholderString(Some(&NSString::from_str(&format!(
                    "{} 검색",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ))));
            self.views()
                .search_scope
                .selectItemAtIndex(if recursive { 1 } else { 0 });
            self.views().window.setTitle(&NSString::from_str(&format!(
                "{} — Rust Explorer",
                path.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Mac".into())
            )));
        }
    }
    fn refresh_tabs(&self) {
        unsafe {
            let v = self.views();
            let tabs: Vec<_> = {
                let s = self.ivars().state.borrow();
                s.tabs
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        (
                            t.id,
                            t.path
                                .file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| "Mac".into()),
                            i == s.active,
                        )
                    })
                    .collect()
            };
            let identifiers: HashSet<_> =
                tabs.iter().map(|(id, _, _)| format!("tab:{id}")).collect();
            let mut existing = HashMap::new();
            for child in v.tabs.arrangedSubviews().iter() {
                let key = child
                    .identifier()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if identifiers.contains(&key) {
                    existing.insert(key, child);
                } else {
                    v.tabs.removeArrangedSubview(&child);
                    child.removeFromSuperview();
                }
            }
            let width = ((v.window.frame().size.width - 150.) / tabs.len() as f64).clamp(96., 190.);
            let content_width =
                (tabs.len() as f64 * (width + 4.) - 4.).max(v.tab_scroll.bounds().size.width);
            v.tabs.setFrame(rect(0., 0., content_width, 32.));
            let mut selected_group = None;
            for (id, name, active) in tabs {
                let key = format!("tab:{id}");
                let group = if let Some(group) = existing.remove(&key) {
                    group
                } else {
                    let group =
                        NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0., 0., width, 30.));
                    group.setIdentifier(Some(&NSString::from_str(&key)));
                    group.setTranslatesAutoresizingMaskIntoConstraints(false);
                    let constraint = group.widthAnchor().constraintEqualToConstant(width);
                    constraint.setIdentifier(Some(ns_string!("tab-width")));
                    constraint.setActive(true);
                    group
                        .heightAnchor()
                        .constraintEqualToConstant(30.)
                        .setActive(true);
                    group.setWantsLayer(true);
                    let select = tab_button(self.mtm(), &name, self, sel!(selectTab:));
                    select.setTag(id as isize);
                    let close = tab_button(self.mtm(), "", self, sel!(closeTabAt:));
                    close.setTag(id as isize);
                    close.setImage(symbol("xmark").as_deref());
                    group.addSubview(&select);
                    group.addSubview(&close);
                    v.tabs.addArrangedSubview(&group);
                    group
                };
                for constraint in group.constraints().iter() {
                    if constraint
                        .identifier()
                        .is_some_and(|s| s.to_string() == "tab-width")
                    {
                        constraint.setConstant(width);
                    }
                }
                if let Some(layer) = group.layer() {
                    layer.setCornerRadius(v.palette.get().radius());
                    layer.setBackgroundColor(Some(
                        &if active {
                            super::theme::color(v.palette.get().canvas)
                        } else {
                            NSColor::clearColor()
                        }
                        .CGColor(),
                    ));
                }
                let children = group.subviews();
                let select = children
                    .objectAtIndex(0)
                    .downcast::<NSButton>()
                    .expect("tab select button");
                select.setTitle(&NSString::from_str(&name));
                let tab_color = if !active
                    && v.palette.get().spec.layout == crate::core::theme::Layout::Header
                {
                    v.palette.get().on_accent
                } else {
                    v.palette.get().ink
                };
                select.setContentTintColor(Some(&super::theme::color(tab_color)));
                select.setFont(Some(&NSFont::systemFontOfSize(12.)));
                select.setFrame(rect(3., 0., width - 38., 30.));
                select.setToolTip(Some(&NSString::from_str(&name)));
                let close = children
                    .objectAtIndex(1)
                    .downcast::<NSButton>()
                    .expect("tab close button");
                close.setFrame(rect(width - 31., 1., 28., 28.));
                close.setContentTintColor(Some(&super::theme::color(tab_color)));
                let label = NSString::from_str(&format!("{name} 탭 닫기"));
                close.setToolTip(Some(&label));
                let _: () = msg_send![&*close,setAccessibilityLabel:&*label];
                if active {
                    selected_group = Some(group);
                }
            }
            v.tabs.layoutSubtreeIfNeeded();
            if let Some(group) = selected_group {
                group.scrollRectToVisible(group.bounds());
            }
        }
    }
    fn ensure_sidebar(&self, id: u64) {
        let path = {
            let mut s = self.ivars().state.borrow_mut();
            let Some(n) = s.nodes.get_mut(&id) else {
                return;
            };
            if n.loaded || n.loading {
                return;
            }
            n.loading = true;
            n.path.clone()
        };
        if let Some(path) = path {
            self.ivars().runtime.sidebar(self.id(), id, path);
        }
    }
    pub fn apply_preferences(&self, prefs: Preferences, reload: bool) {
        let _paint = super::theme::PaintTransaction::new();
        super::selection::finish(&self.views().table, &self.views().collection, false);
        let old = self.ivars().state.borrow().prefs.clone();
        let mut compare = prefs.clone();
        compare.theme = old.theme;
        let theme_only = reload && compare == old;
        if old.view != prefs.view {
            self.remember_scroll();
        }
        let hidden_changed = old.hidden != prefs.hidden;
        let theme_changed =
            old.theme != prefs.theme || self.ivars().state.borrow().theme_name.is_empty();
        let sidebar_changed = old.favorites != prefs.favorites
            || old.sidebar != prefs.sidebar
            || old.pinned_locations != prefs.pinned_locations
            || old.shortcut_labels != prefs.shortcut_labels;
        self.ivars().state.borrow_mut().prefs = prefs.clone();
        self.views()
            .set_keyboard_profile(prefs.keyboard == KeyboardProfile::Finder);
        unsafe {
            let v = self.views();
            if theme_changed {
                v.apply_theme(prefs.theme);
                let mut state = self.ivars().state.borrow_mut();
                state.images.set_theme(v.palette.get());
                state.theme_name = v.window.effectiveAppearance().name().to_string();
            }
            let height = v.palette.get().row_height(prefs.compact);
            if v.table.rowHeight() != height {
                let origin = v.list_scroll.contentView().bounds().origin;
                let row = v.table.rowAtPoint(NSPoint::new(1., origin.y));
                let offset = if row >= 0 {
                    origin.y - v.table.rectOfRow(row).origin.y
                } else {
                    0.
                };
                v.table.setRowHeight(height);
                if row >= 0 {
                    let point = NSPoint::new(
                        origin.x,
                        v.table.rectOfRow(row).origin.y + offset.min(height - 1.),
                    );
                    v.list_scroll.contentView().scrollToPoint(point);
                    v.list_scroll
                        .reflectScrolledClipView(&v.list_scroll.contentView());
                }
            }
            v.list_scroll.setHidden(prefs.view == ViewMode::Icons);
            v.icon_scroll.setHidden(prefs.view != ViewMode::Icons);
            v.preview.setHidden(!prefs.preview);
            v.access_banner.setHidden(prefs.access_intro_dismissed);
            if !reload || old.view != prefs.view || old.icon_size != prefs.icon_size {
                v.set_icon_size(prefs.view, prefs.icon_size);
            }
            v.layout();
            self.update_preview();
        }
        if theme_changed {
            self.refresh_theme_contents();
            let panel = self.ivars().state.borrow().appearance_panel.clone();
            if let Some(panel) = panel {
                panel.sync(prefs.theme);
            }
        }
        if sidebar_changed {
            self.refresh_sidebar();
        }
        if reload && hidden_changed {
            for tab in &mut self.ivars().state.borrow_mut().tabs {
                tab.watch_due = Some(Instant::now());
            }
            self.load(true);
        } else if !reload
            || old.view != prefs.view
            || old.icon_size != prefs.icon_size
            || old.compact != prefs.compact
        {
            self.reload();
        }
        if old.view != prefs.view {
            self.restore_scroll();
            self.focus_list();
        }
        if !theme_only {
            self.update_status();
        }
    }
    unsafe fn paint_file_row(&self, table: &NSTableView, row_view: &NSTableRowView, row: isize) {
        let p = self.views().palette.get();
        super::theme::restyle_row(row_view, p, false);
        for column in 0..table.numberOfColumns() {
            if let Some(view) = table
                .viewAtColumn_row_makeIfNecessary(column, row, false)
                .and_then(|v| v.downcast::<NSTableCellView>().ok())
            {
                let name = table
                    .tableColumns()
                    .objectAtIndex(column as usize)
                    .identifier()
                    .to_string()
                    == "name";
                if let Some(label) = view.textField() {
                    label.setTextColor(Some(&super::theme::color(if name {
                        p.ink
                    } else {
                        p.muted
                    })));
                }
                if let Some(image) = view.imageView()
                    && let Some(e) = self.entry_at(row)
                {
                    image.setImage(self.ivars().state.borrow_mut().images.icon(&e).as_deref());
                }
            }
        }
    }
    unsafe fn paint_side_row(&self, table: &NSOutlineView, row_view: &NSTableRowView, row: isize) {
        let p = self.views().palette.get();
        super::theme::restyle_row(row_view, p, true);
        if let Some(view) = table
            .viewAtColumn_row_makeIfNecessary(0, row, false)
            .and_then(|v| v.downcast::<NSTableCellView>().ok())
        {
            let section = view
                .identifier()
                .is_some_and(|s| s.to_string().contains("section"));
            if let Some(label) = view.textField() {
                label.setTextColor(Some(&super::theme::color(if section {
                    p.side_muted
                } else {
                    p.side_ink
                })));
            }
            if let Some(image) = view.imageView() {
                image.setContentTintColor(Some(&super::theme::color(p.side_ink)));
            }
            if let Some(button) = view
                .viewWithTag(809)
                .and_then(|v| v.downcast::<NSButton>().ok())
            {
                button.setContentTintColor(Some(&super::theme::color(p.side_ink)));
            }
        }
    }
    fn refresh_theme_contents(&self) {
        unsafe {
            // Repaint materialized (including buffered) cells; no model reload or
            // traversal of the entire folder. Display callbacks handle later reuse.
            let v = self.views();
            let p = v.palette.get();
            v.table
                .enumerateAvailableRowViewsUsingBlock(&block2::RcBlock::new(
                    |view: std::ptr::NonNull<NSTableRowView>, row| {
                        self.paint_file_row(&v.table, view.as_ref(), row);
                    },
                ));
            v.sidebar
                .enumerateAvailableRowViewsUsingBlock(&block2::RcBlock::new(
                    |view: std::ptr::NonNull<NSTableRowView>, row| {
                        self.paint_side_row(&v.sidebar, view.as_ref(), row);
                    },
                ));
            for index in v.collection.indexPathsForVisibleItems() {
                if let Some(item) = v.collection.itemAtIndexPath(&index) {
                    set_item_theme(&item, p);
                    if let Some(e) = self.entry_at(index.item())
                        && let Some(image) = item.imageView()
                    {
                        image.setImage(self.ivars().state.borrow_mut().images.icon(&e).as_deref());
                    }
                }
            }
            let image = super::theme::icon(p, super::theme::Glyph::Folder);
            let parts = v.breadcrumb.pathItems();
            for part in parts.iter() {
                if part.title().to_string() != "…" {
                    part.setImage(Some(&image));
                }
            }
            v.breadcrumb.setPathItems(&parts);
            self.refresh_tabs();
            self.update_preview();
        }
    }
    fn commit_preferences(&self, prefs: Preferences) {
        self.apply_preferences(prefs.clone(), true);
        self.ivars().runtime.save_preferences(prefs);
    }
    fn set_view(&self, mode: ViewMode, size: Option<IconSize>) {
        super::selection::finish(&self.views().table, &self.views().collection, false);
        let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
        prefs.view = mode;
        if let Some(size) = size {
            prefs.icon_size = size;
        }
        self.commit_preferences(prefs);
    }
    fn zoom_icons(&self, larger: bool) {
        let size = self.ivars().state.borrow().prefs.icon_size;
        self.set_view(
            ViewMode::Icons,
            Some(IconSize::from_index(
                (size.index() + if larger { 1 } else { -1 }).clamp(0, 2),
            )),
        );
    }
    fn copy_text(&self, text: &str) {
        unsafe {
            let board = NSPasteboard::generalPasteboard();
            board.clearContents();
            board.setString_forType(&NSString::from_str(text), NSPasteboardTypeString);
        }
    }
    fn notice(&self, text: &str, undo: Option<String>) {
        self.views().show_notice_undo(undo.is_some());
        self.ivars().state.borrow_mut().notice =
            Some((text.into(), Instant::now() + Duration::from_secs(8), undo));
        self.update_status();
    }
    fn toggle_pref(&self, which: &str) {
        let mut p = self.ivars().runtime.preferences.lock().unwrap().clone();
        match which {
            "hidden" => p.hidden = !p.hidden,
            "compact" => p.compact = !p.compact,
            "preview" => p.preview = !p.preview,
            "view" => {
                p.view = if p.view == ViewMode::Details {
                    ViewMode::Icons
                } else {
                    ViewMode::Details
                }
            }
            _ => {}
        }
        self.apply_preferences(p.clone(), true);
        self.ivars().runtime.save_preferences(p);
    }
    fn set_sort(&self, field: SortField, descending: bool) {
        self.with_tab_mut(|t| {
            t.sort = field;
            t.descending = descending;
        });
        self.load(true);
        self.save_tabs();
    }
    fn toggle_sort(&self, field: SortField) {
        let desc = self.with_tab(|t| {
            if t.sort == field {
                !t.descending
            } else {
                field == SortField::Modified
            }
        });
        self.set_sort(field, desc);
    }
    fn search_changed(&self) {
        let query = self.views().search.stringValue().to_string();
        let recursive = self.views().search_scope.indexOfSelectedItem() == 1;
        let changed = self.with_tab_mut(|t| {
            let changed = t.recursive != recursive;
            t.query = query;
            t.selected.clear();
            t.recursive = recursive;
            t.filter_due = Some(Instant::now() + Duration::from_millis(150));
            t.filter_cancel.cancel();
            changed
        });
        if changed {
            self.load(false);
        }
    }
    fn apply_filter(&self) {
        let (id, generation, filter_generation, entries, query, cancel, recursive) = self
            .with_tab_mut(|t| {
                t.filter_cancel.cancel();
                t.filter_cancel = Cancellation::default();
                t.filter_generation += 1;
                t.filter_due = None;
                (
                    t.id,
                    t.generation,
                    t.filter_generation,
                    t.entries.clone(),
                    t.query.clone(),
                    t.filter_cancel.clone(),
                    t.recursive,
                )
            });
        if recursive {
            self.load(false);
        } else {
            self.ivars().runtime.filter(FilterRequest {
                window: self.id(),
                tab: id,
                generation,
                filter_generation,
                entries,
                query,
                cancel,
            });
        }
    }
    pub fn tick(&self) {
        if self.views().window.attachedSheet().is_none() {
            let pending = self.ivars().state.borrow_mut().pending_access.take();
            match pending {
                Some(folder_access::PendingAccess::Settings) => self.folder_access_settings(),
                Some(folder_access::PendingAccess::Picker(path)) => {
                    self.choose_access_folders(path)
                }
                None => {}
            }
        }
        super::selection::tick(&self.views().table, &self.views().collection);
        if !self.ivars().state.borrow().selecting && !self.ivars().state.borrow().dragging {
            let events = std::mem::take(&mut self.ivars().state.borrow_mut().selection_events);
            for event in events {
                self.handle_event(event);
            }
        }
        self.tick_batch_preview();
        let now = Instant::now();
        let (reveal, expired) = {
            let s = self.ivars().state.borrow();
            (
                s.job_reveal_due.is_some_and(|d| d <= now),
                s.notice.as_ref().is_some_and(|n| n.1 <= now),
            )
        };
        if reveal {
            self.ivars().state.borrow_mut().job_reveal_due = None;
            if !self.ivars().runtime.jobs.lock().unwrap().is_empty() {
                unsafe {
                    self.views().jobs.setHidden(false);
                    self.views().layout();
                }
            }
        }
        if expired {
            self.ivars().state.borrow_mut().notice = None;
            self.views().show_notice_undo(false);
            self.update_status();
        }
        if self.ivars().state.borrow().renaming.is_some()
            || self.ivars().state.borrow().dragging
            || self.ivars().state.borrow().selecting
        {
            return;
        }
        if self
            .ivars()
            .state
            .borrow()
            .drag_refresh_due
            .is_some_and(|d| d <= now)
        {
            self.ivars().state.borrow_mut().drag_refresh_due = None;
            self.reload();
        }
        let now = Instant::now();
        let (filter, watch) = self.with_tab(|t| {
            (
                t.filter_due.is_some_and(|d| now >= d),
                !t.loading && t.watch_due.is_some_and(|d| now >= d),
            )
        });
        if filter {
            self.apply_filter();
        } else if watch {
            self.load(true);
        }
        self.follow_created_item();
        self.request_metadata();
        self.request_thumbnails();
        let check = {
            let mut s = self.ivars().state.borrow_mut();
            if s.last_theme_check.elapsed() > Duration::from_secs(1) {
                s.last_theme_check = now;
                true
            } else {
                false
            }
        };
        if check {
            let panel = self.ivars().state.borrow().appearance_panel.clone();
            if let Some(panel) = panel {
                panel.sync_environment();
            }
            unsafe {
                let name = self.views().window.effectiveAppearance().name().to_string();
                let changed = self.ivars().state.borrow().theme_name != name;
                if changed {
                    self.ivars().state.borrow_mut().theme_name = name;
                    self.views().apply_appearance();
                    self.ivars()
                        .state
                        .borrow_mut()
                        .images
                        .set_theme(self.views().palette.get());
                    self.refresh_theme_contents();
                }
            }
        }
    }
    fn request_metadata(&self) {
        unsafe {
            let count = self.with_tab(|t| t.visible.len());
            if count == 0 {
                return;
            }
            let (start, end) = if self.ivars().state.borrow().prefs.view == ViewMode::Details {
                let range = self
                    .views()
                    .table
                    .rowsInRect(self.views().table.visibleRect());
                (
                    range.location.min(count),
                    range
                        .location
                        .saturating_add(range.length)
                        .saturating_add(20)
                        .min(count),
                )
            } else {
                let visible = self.views().collection.indexPathsForVisibleItems();
                let min = visible
                    .iter()
                    .map(|i| i.item().max(0) as usize)
                    .min()
                    .unwrap_or(0);
                (min, (min + 100).min(count))
            };
            let request = self.with_tab_mut(|t| {
                let mut entries = vec![];
                for &i in t.visible.get(start..end).unwrap_or_default() {
                    if let Some(e) = t.entries.get(i)
                        && !e.metadata_loaded
                        && !t.metadata.contains_key(&e.id)
                        && t.requested.insert(e.id)
                    {
                        entries.push(e.clone());
                    }
                }
                (!entries.is_empty()).then_some((t.id, t.generation, entries))
            });
            if let Some((id, generation, entries)) = request {
                self.ivars()
                    .runtime
                    .metadata(self.id(), id, generation, entries);
            }
        }
    }
    pub fn reload(&self) {
        if self.ivars().state.borrow().dragging || self.ivars().state.borrow().selecting {
            self.ivars().state.borrow_mut().drag_refresh_due = Some(Instant::now());
            return;
        }
        unsafe {
            self.ivars().state.borrow_mut().reloading = true;
            let selected = self.with_tab(|t| {
                if t.selected.is_empty() {
                    return Vec::new();
                }
                t.visible
                    .iter()
                    .enumerate()
                    .filter_map(|(row, &i)| {
                        t.entries
                            .get(i)
                            .filter(|e| t.selected.contains(&e.id))
                            .map(|_| row)
                    })
                    .collect::<Vec<_>>()
            });
            if self.ivars().state.borrow().prefs.view == ViewMode::Details {
                self.views().table.reloadData();
                let set = NSMutableIndexSet::new();
                for row in &selected {
                    set.addIndex(*row);
                }
                self.views()
                    .table
                    .selectRowIndexes_byExtendingSelection(&set, false);
            } else {
                self.views().collection.reloadData();
                let indices: Vec<_> = selected
                    .into_iter()
                    .map(|i| NSIndexPath::indexPathForItem_inSection(i as isize, 0))
                    .collect();
                self.views()
                    .collection
                    .setSelectionIndexPaths(&NSSet::from_retained_slice(&indices));
            }
            self.ivars().state.borrow_mut().reloading = false;
            let (_, view_index) = self.scroll_surface();
            if self.with_tab(|t| t.scroll[view_index].is_none() && !t.visible.is_empty()) {
                self.restore_scroll();
            }
            self.update_status();
            self.request_metadata();
            self.update_preview();
            if self.with_tab(|t| !t.first_displayed && !t.entries.is_empty() && !t.refreshing) {
                self.views().window.displayIfNeeded();
                let (ms, rows) = self.with_tab_mut(|t| {
                    t.first_displayed = true;
                    (
                        t.load_started.elapsed().as_secs_f64() * 1000.,
                        t.entries.len(),
                    )
                });
                super::record_listing(ms, rows);
            }
        }
    }
    fn update_status(&self) {
        unsafe {
            let (text, empty) = self.with_tab(|t| {
                let count = t.visible.len();
                let selected = t.selected.len();
                let mut size = 0u64;
                let mut known = 0usize;
                let mut unknown = 0usize;
                let mut folders = 0usize;
                if selected > 0 {
                    for entry in t.entries.iter().filter(|e| t.selected.contains(&e.id)) {
                        let e = t.metadata.get(&entry.id).unwrap_or(entry);
                        if e.kind == Kind::Folder {
                            folders += 1;
                        } else if let Some(bytes) = e.size {
                            size = size.saturating_add(bytes);
                            known += 1;
                        } else {
                            unknown += 1;
                        }
                    }
                }
                let mut extra = if selected > 0 {
                    format!("  ·  {selected}개 선택")
                } else {
                    String::new()
                };
                if known > 0 {
                    extra.push_str(&format!(
                        "  ·  {}{}",
                        if unknown > 0 {
                            "확인된 파일 "
                        } else {
                            "파일 "
                        },
                        format_size(size)
                    ));
                }
                if folders > 0 {
                    extra.push_str(&format!("  ·  폴더 {folders}개"));
                }
                if unknown > 0 {
                    extra.push_str(&format!("  ·  크기 미확인 {unknown}개"));
                }
                let loading = if t.loading { "  ·  읽는 중…" } else { "" };
                let errors = if t.skipped > 0 {
                    format!("  ·  접근 불가 {}개", t.skipped)
                } else {
                    String::new()
                };
                let empty = if let Some(error) = &t.error {
                    Some(if error.access_denied {
                        "이 폴더에 접근할 수 없습니다\n폴더를 선택해 접근을 허용하거나 파일의 읽기 권한을 확인하세요.".into()
                    } else { error.to_string() })
                } else if count == 0 {
                    Some(
                        if t.loading {
                            "폴더를 읽는 중…"
                        } else if !t.query.is_empty() {
                            "검색 결과가 없습니다"
                        } else {
                            "이 폴더는 비어 있습니다"
                        }
                        .into(),
                    )
                } else {
                    None
                };
                (
                    if let Some(error) = &t.error {
                        if error.access_denied { "폴더 접근 권한을 확인하세요".into() } else { "폴더를 열지 못했습니다".into() }
                    } else { format!(
                        "{count}개 항목{extra}{loading}{errors}{}",
                        if t.read_only {
                            "  ·  읽기 전용"
                        } else {
                            ""
                        }
                    ) },
                    empty,
                )
            });
            let text = self
                .ivars()
                .state
                .borrow()
                .notice
                .as_ref()
                .filter(|_| self.with_tab(|t| t.error.is_none()))
                .map(|n| format!("{}  ·  {text}", n.0))
                .unwrap_or(text);
            let (text, tooltip) = {
                let state = self.ivars().state.borrow();
                if state.input_diagnostics {
                    let text = state.diagnostic_events.back().cloned().unwrap_or_else(|| {
                        "입력 확인: 대기 중 · 파일 하나를 선택하고 F2를 눌러 주세요".into()
                    });
                    let events = state
                        .diagnostic_events
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    let active = NSApplication::sharedApplication(self.mtm()).isActive();
                    let key = self.views().window.isKeyWindow();
                    let tooltip = format!("앱 활성: {active} · 키 창: {key}\n{events}");
                    let target = if active && key {
                        "이 창"
                    } else {
                        "다른 창"
                    };
                    (format!("키보드 대상: {target} · {text}"), tooltip)
                } else {
                    (text.clone(), text)
                }
            };
            self.views()
                .status
                .setStringValue(&NSString::from_str(&text));
            self.views()
                .status
                .setToolTip(Some(&NSString::from_str(&tooltip)));
            let (can_write, selected, back, forward, up) = self.with_tab(|t| {
                (
                    t.can_write(),
                    t.selected.len(),
                    !t.back.is_empty(),
                    !t.forward.is_empty(),
                    t.path.parent().is_some(),
                )
            });
            let can_paste = NSPasteboard::generalPasteboard()
                .availableTypeFromArray(&NSArray::from_slice(&[NSPasteboardTypeFileURL]))
                .is_some();
            self.views()
                .set_actions_enabled(can_write, selected, can_paste, back, forward, up);
            self.views().empty_label.setHidden(empty.is_none());
            let (has_error, denied, details) = self.with_tab(|t| {
                (
                    t.error.is_some(),
                    t.error.as_ref().is_some_and(|e| e.access_denied),
                    t.error.as_ref().map(ToString::to_string),
                )
            });
            self.views()
                .empty_label
                .setToolTip(details.as_deref().map(NSString::from_str).as_deref());
            self.views().set_error_actions(has_error, denied);
            if let Some(e) = empty {
                self.views()
                    .empty_label
                    .setStringValue(&NSString::from_str(&e));
            }
        }
    }
    fn selection_changed(&self) {
        if self.ivars().state.borrow().reloading {
            return;
        }
        unsafe {
            let rows: Vec<_> = if self.ivars().state.borrow().prefs.view == ViewMode::Details {
                let set = self.views().table.selectedRowIndexes();
                let mut rows = vec![];
                let mut i = set.firstIndex();
                while i != NSNotFound as usize {
                    rows.push(i);
                    i = set.indexGreaterThanIndex(i);
                }
                rows
            } else {
                self.views()
                    .collection
                    .selectionIndexPaths()
                    .iter()
                    .map(|p| p.item() as usize)
                    .collect()
            };
            self.with_tab_mut(|t| {
                t.selected = rows
                    .into_iter()
                    .filter_map(|r| {
                        t.visible
                            .get(r)
                            .and_then(|i| t.entries.get(*i))
                            .map(|e| e.id)
                    })
                    .collect()
            });
            self.update_status();
            if !self.ivars().state.borrow().selecting && !self.ivars().state.borrow().dragging {
                self.update_preview();
            }
        }
    }
    pub fn handle_event(&self, event: Event) {
        // Keep native row indices bound to the same IDs until the gesture ends.
        // Scan completion and background sorting may otherwise reorder them.
        if (self.ivars().state.borrow().selecting || self.ivars().state.borrow().dragging)
            && matches!(
                event,
                Event::Scan { .. } | Event::Filter { .. } | Event::AccessChanged { .. }
            )
        {
            self.ivars()
                .state
                .borrow_mut()
                .selection_events
                .push_back(event);
            return;
        }
        match event {
            Event::AccessChanged {
                origin,
                paths,
                detail,
                add_favorites,
                ..
            } => {
                self.folder_access_changed(origin, paths, detail, add_favorites);
            }
            Event::PinFolders { group, result, .. } => match result {
                Ok(paths) => self.pin_paths(group, &paths),
                Err(error) => self.notice(&error, None),
            },
            Event::NetworkUnmounted { result, .. } => match result {
                Ok(()) => {
                    self.ivars().runtime.volumes();
                    self.notice("서버 연결을 해제했습니다", None);
                }
                Err(error) => self.message("서버 연결 해제 실패", &error),
            },
            Event::Applications {
                request, result, ..
            } => self.show_applications(request, result),
            Event::FolderReady {
                request,
                tab,
                origin,
                result,
                ..
            } => self.folder_ready(request, tab, origin, result),
            Event::RenamePreview {
                dialog,
                generation,
                result,
                ..
            } => self.show_rename_preview(dialog, generation, result),
            Event::Scan {
                tab,
                generation,
                update,
                ..
            } => {
                let mut reload = false;
                let mut filter = false;
                let mut recent = None;
                {
                    let mut s = self.ivars().state.borrow_mut();
                    let active = s.active;
                    let Some(index) = s
                        .tabs
                        .iter()
                        .position(|t| t.id == tab && t.generation == generation)
                    else {
                        return;
                    };
                    let t = &mut s.tabs[index];
                    match update {
                        ScanUpdate::Batch(batch) => {
                            if !t.refreshing {
                                let start = t.entries.len();
                                let key = search_key(&t.query);
                                t.visible.extend(
                                    batch
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, e)| {
                                            t.recursive || key.is_empty() || e.key.contains(&key)
                                        })
                                        .map(|(i, _)| start + i),
                                );
                                Arc::make_mut(&mut t.entries).extend(batch);
                                reload = index == active;
                            }
                        }
                        ScanUpdate::Complete { entries, skipped } => {
                            if index == active && !t.refreshing {
                                recent = Some(t.path.clone());
                            }
                            t.loading = false;
                            t.refreshing = false;
                            t.skipped = skipped;
                            t.entries = Arc::new(entries);
                            if !t.selected.is_empty() {
                                let ids: HashSet<_> = t.entries.iter().map(|e| e.id).collect();
                                t.selected.retain(|id| ids.contains(id));
                            }
                            t.visible = (0..t.entries.len()).collect();
                            reload = index == active;
                            filter = reload && !t.recursive && !t.query.is_empty();
                        }
                        ScanUpdate::Failed(e) => {
                            t.entries = Arc::new(vec![]);
                            t.visible.clear();
                            t.selected.clear();
                            t.refreshing = false;
                            t.loading = false;
                            t.error = Some(e);
                            reload = index == active;
                        }
                    }
                }
                if let Some(path) = recent {
                    self.ivars().runtime.remember_folder(path);
                }
                if reload {
                    self.reload();
                }
                if filter {
                    self.apply_filter();
                }
            }
            Event::Metadata {
                tab,
                generation,
                entries,
                ..
            } => {
                let active = {
                    let mut s = self.ivars().state.borrow_mut();
                    let active = s.active;
                    let Some(index) = s
                        .tabs
                        .iter()
                        .position(|t| t.id == tab && t.generation == generation)
                    else {
                        return;
                    };
                    for e in entries {
                        s.tabs[index].requested.remove(&e.id);
                        s.tabs[index].metadata.insert(e.id, e);
                    }
                    index == active
                };
                if active
                    && self.ivars().state.borrow().renaming.is_none()
                    && !self.ivars().state.borrow().dragging
                    && !self.ivars().state.borrow().selecting
                {
                    unsafe {
                        let range = self
                            .views()
                            .table
                            .rowsInRect(self.views().table.visibleRect());
                        let count = self.with_tab(|t| t.visible.len());
                        if range.location < count {
                            self.views().table.reloadDataForRowIndexes_columnIndexes(
                                &NSIndexSet::indexSetWithIndexesInRange(NSRange::new(
                                    range.location,
                                    range.length.min(count - range.location),
                                )),
                                &NSIndexSet::indexSetWithIndexesInRange(NSRange::new(0, 4)),
                            );
                        }
                    }
                    self.update_status();
                    self.update_preview();
                }
            }
            Event::Location {
                tab,
                generation,
                read_only,
                free_bytes,
                ..
            } => {
                let active = {
                    let mut s = self.ivars().state.borrow_mut();
                    let active = s.active;
                    let Some(index) = s
                        .tabs
                        .iter()
                        .position(|t| t.id == tab && t.generation == generation)
                    else {
                        return;
                    };
                    s.tabs[index].read_only = read_only;
                    s.tabs[index].free_bytes = free_bytes;
                    index == active
                };
                if active {
                    self.update_status();
                }
            }
            Event::Thumbnail {
                tab,
                generation,
                id,
                png,
                cost,
                ..
            } => {
                if self.with_tab(|t| t.id == tab && t.generation == generation) {
                    self.ivars()
                        .state
                        .borrow_mut()
                        .thumbnail_requests
                        .remove(&id);
                    if let Some(png) = png
                        && let Some(image) =
                            NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png))
                    {
                        self.ivars()
                            .state
                            .borrow_mut()
                            .images
                            .insert_thumbnail(id, image, cost);
                        unsafe {
                            let paths = self.views().collection.indexPathsForVisibleItems();
                            if self.ivars().state.borrow().renaming.is_none()
                                && !self.ivars().state.borrow().selecting
                                && !self.ivars().state.borrow().dragging
                            {
                                self.views().collection.reloadItemsAtIndexPaths(&paths);
                            }
                        }
                    }
                }
            }
            Event::Filter {
                tab,
                generation,
                filter_generation,
                indices,
                ..
            } => {
                let active = {
                    let mut s = self.ivars().state.borrow_mut();
                    let active = s.active;
                    let Some(index) = s.tabs.iter().position(|t| {
                        t.id == tab
                            && t.generation == generation
                            && t.filter_generation == filter_generation
                    }) else {
                        return;
                    };
                    s.tabs[index].visible = indices;
                    let visible_ids: HashSet<_> = s.tabs[index]
                        .visible
                        .iter()
                        .filter_map(|i| s.tabs[index].entries.get(*i))
                        .map(|e| e.id)
                        .collect();
                    s.tabs[index].selected.retain(|id| visible_ids.contains(id));
                    index == active
                };
                if active {
                    self.reload();
                }
            }
            Event::Watch { tab, .. } => {
                let mut s = self.ivars().state.borrow_mut();
                if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab) {
                    t.watch_due = Some(Instant::now() + Duration::from_millis(250));
                }
            }
            Event::Sidebar {
                parent, entries, ..
            } => {
                let parent_obj = {
                    let mut s = self.ivars().state.borrow_mut();
                    let mut children = vec![];
                    for e in entries {
                        let id = path_id(Path::new(&format!("tree:{parent}:{}", e.id)));
                        children.push(id);
                        s.nodes.entry(id).or_insert_with(|| SidebarNode {
                            object: OutlineNode::new(self.mtm(), id),
                            label: e.name,
                            path: Some(e.path),
                            children: vec![],
                            loaded: false,
                            loading: false,
                            section: false,
                            key: String::new(),
                            symbol: "folder".into(),
                            action: None,
                            group: 0,
                        });
                    }
                    let Some(n) = s.nodes.get_mut(&parent) else {
                        return;
                    };
                    n.children = children;
                    n.loaded = true;
                    n.loading = false;
                    n.object.clone()
                };
                unsafe {
                    self.views()
                        .sidebar
                        .reloadItem_reloadChildren(Some(&parent_obj), true);
                    self.views().sidebar.expandItem(Some(&parent_obj));
                }
            }
            Event::Open {
                path,
                directory,
                error,
                ..
            } => {
                if let Some(error) = error {
                    self.message("파일을 열 수 없습니다", &error);
                } else if directory {
                    self.navigate(path, true);
                } else if crate::core::archive::is_archive(&path) {
                    self.extract_archive(path);
                } else {
                    unsafe {
                        if !NSWorkspace::sharedWorkspace().openURL(&platform::file_url(&path)) {
                            self.message("파일을 열 수 없습니다", "연결된 기본 앱을 확인하세요.");
                        }
                    }
                }
            }
            Event::Message { title, detail, .. } => self.message(&title, &detail),
            _ => {}
        }
    }
    pub fn message(&self, title: &str, detail: &str) {
        unsafe {
            show_message(&self.views().window, title, detail);
        }
    }
}

impl Controller {
    fn retained(&self) -> Retained<Self> {
        unsafe { Retained::retain(self as *const Self as *mut Self).unwrap() }
    }
    pub fn open_selected(&self) {
        for path in self.selected_paths() {
            self.ivars().runtime.open(self.id(), path);
        }
    }
    fn copy(&self, cut: bool) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        unsafe {
            let count = write_clipboard(&paths, cut);
            self.update_status();
            self.ivars().state.borrow_mut().last_clipboard = if cut { Some(count) } else { None };
            self.views()
                .status
                .setStringValue(&NSString::from_str(&format!(
                    "{}개 항목 {} · 대상 폴더에서 붙여넣으세요",
                    paths.len(),
                    if cut { "잘라내기" } else { "복사" }
                )));
        }
    }
    fn paste(&self, force_move: bool) {
        unsafe {
            let board = self.clipboard_board();
            let count = board.changeCount();
            let paths = clipboard_paths(&board);
            if paths.is_empty() {
                return;
            }
            let moving = force_move || clipboard_cut(&board);
            let job = self.start_operation(Operation::transfer(
                if moving {
                    OperationKind::Move
                } else {
                    OperationKind::Copy
                },
                paths,
                self.current_path(),
            ));
            if moving && let Some(id) = job {
                self.ivars()
                    .state
                    .borrow_mut()
                    .paste_moves
                    .insert(id, count);
            }
        }
    }
    fn clipboard_board(&self) -> Retained<NSPasteboard> {
        #[cfg(feature = "native-ui-tests")]
        if let Some(board) = &self.ivars().state.borrow().test_clipboard {
            return board.clone();
        }
        unsafe { NSPasteboard::generalPasteboard() }
    }
    fn trash(&self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.start_operation(Operation {
            completed_sources: vec![],
            renames: vec![],
            kind: OperationKind::Trash,
            sources: paths.iter().map(|p| StoredPath::new(p)).collect(),
            destination: None,
            name: None,
        });
    }
    pub fn start_operation(&self, request: Operation) -> Option<u64> {
        match self.ivars().runtime.execute(self.id(), request) {
            Ok(id) => {
                let idle = self.ivars().state.borrow().active_job.is_none();
                if idle {
                    self.ivars().state.borrow_mut().active_job = Some(id);
                }
                unsafe {
                    self.ivars().state.borrow_mut().job_reveal_due =
                        Some(Instant::now() + Duration::from_millis(450));
                    if idle {
                        self.views()
                            .job_title
                            .setStringValue(ns_string!("파일 작업 대기 중"));
                    }
                    self.views().layout();
                }
                Some(id)
            }
            Err(e) => {
                self.message("작업을 시작할 수 없습니다", e);
                None
            }
        }
    }
    fn extract_archive(&self, path: PathBuf) {
        if !crate::core::archive::is_archive(&path) {
            return;
        }
        let destination = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        let origin = self.with_tab(|t| (t.id, t.path.clone()));
        let request = Operation {
            completed_sources: vec![],
            renames: vec![],
            kind: OperationKind::Extract,
            sources: vec![StoredPath::new(&path)],
            destination: Some(StoredPath::new(&destination)),
            name: None,
        };
        if let Some(id) = self.start_operation(request) {
            self.ivars()
                .state
                .borrow_mut()
                .extraction_jobs
                .insert(id, origin);
        }
    }
    fn drag_ended(&self) {
        let mut state = self.ivars().state.borrow_mut();
        state.dragging = false;
        state.drag_refresh_due = Some(Instant::now() + Duration::from_millis(100));
    }
    fn drag_modifiers(&self) -> NSEventModifierFlags {
        #[cfg(feature = "native-ui-tests")]
        if let Some(flags) = self.ivars().state.borrow().test_drag_modifiers {
            return flags;
        }
        NSEvent::modifierFlags_class()
    }
    fn explicit_drop_copy(&self, flags: NSEventModifierFlags) -> bool {
        flags.contains(NSEventModifierFlags::Option)
            || (self.ivars().state.borrow().prefs.keyboard == KeyboardProfile::Windows
                && flags.contains(NSEventModifierFlags::Control))
    }
    unsafe fn drop_request(
        &self,
        info: &ProtocolObject<dyn NSDraggingInfo>,
        target: &Path,
    ) -> Option<Operation> {
        let flags = self.drag_modifiers();
        let explicit_copy = self.explicit_drop_copy(flags);
        let allowed = info.draggingSourceOperationMask();
        if !allowed.intersects(NSDragOperation::Copy | NSDragOperation::Move)
            || (explicit_copy && !allowed.contains(NSDragOperation::Copy))
        {
            return None;
        }
        let paths = crate::core::drag_drop::effective_sources(
            clipboard_paths(&info.draggingPasteboard()),
            target,
            explicit_copy,
        );
        if paths.is_empty() {
            return None;
        }
        let kind = if explicit_copy || !allowed.contains(NSDragOperation::Move) {
            OperationKind::Copy
        } else if !allowed.contains(NSDragOperation::Copy)
            || flags.contains(NSEventModifierFlags::Command)
            || (self.ivars().state.borrow().prefs.keyboard == KeyboardProfile::Windows
                && flags.contains(NSEventModifierFlags::Shift))
        {
            OperationKind::Move
        } else {
            OperationKind::Drop
        };
        Some(Operation::transfer(kind, paths, target.into()))
    }
    unsafe fn validate_drop_target(
        &self,
        info: &ProtocolObject<dyn NSDraggingInfo>,
        target: &Path,
    ) -> NSDragOperation {
        self.drop_request(info, target)
            .map(|r| {
                if r.kind == OperationKind::Copy {
                    NSDragOperation::Copy
                } else {
                    NSDragOperation::Move
                }
            })
            .unwrap_or(NSDragOperation::None)
    }
    unsafe fn drop_files(
        &self,
        info: &ProtocolObject<dyn NSDraggingInfo>,
        target: PathBuf,
    ) -> bool {
        // Reject unsupported data and no-op targets even if a caller skips
        // AppKit's validation. Do not tell another app a transfer succeeded.
        if let Some(request) = self.drop_request(info, &target) {
            self.start_operation(request).is_some()
        } else {
            false
        }
    }
    fn begin_rename(&self) {
        if self.ivars().state.borrow().renaming.is_some() {
            let editing = unsafe {
                self.ivars()
                    .state
                    .borrow()
                    .rename_editor
                    .as_ref()
                    .is_some_and(|f| f.currentEditor().is_some())
            };
            if editing {
                return;
            }
            self.cancel_inline_rename();
        }
        let paths = self.selected_paths();
        if paths.len() != 1 {
            return;
        }
        let path = paths[0].clone();
        unsafe {
            let field = if self.ivars().state.borrow().prefs.view == ViewMode::Icons {
                self.views()
                    .collection
                    .selectionIndexPaths()
                    .iter()
                    .next()
                    .and_then(|index| self.views().collection.itemAtIndexPath(&index))
                    .and_then(|item| item.textField())
            } else {
                let row = self.views().table.selectedRow();
                let column = self.views().table.columnWithIdentifier(ns_string!("name"));
                if row < 0 || column < 0 {
                    return;
                }
                self.views()
                    .table
                    .viewAtColumn_row_makeIfNecessary(column, row, true)
                    .and_then(|view| view.downcast::<NSTableCellView>().ok())
                    .and_then(|cell| cell.textField())
            };
            let Some(field) = field else {
                self.rename_dialog(path);
                return;
            };
            self.with_tab_mut(|t| {
                if t.loading {
                    t.cancel.cancel();
                    t.generation += 1;
                    t.loading = false;
                }
            });
            // Keep the editor outside reusable table/collection cells. AppKit
            // can reconfigure a cell while changing focus or loading metadata.
            let frame = self
                .views()
                .root
                .convertRect_fromView(field.bounds(), Some(&field));
            let editor = NSTextField::initWithFrame(
                NSTextField::alloc(self.mtm()),
                rect(
                    frame.origin.x - 2.,
                    frame.origin.y - 3.,
                    frame.size.width.max(120.) + 4.,
                    26.,
                ),
            );
            single_line_editor(&editor);
            editor.setStringValue(&NSString::from_str(
                &path.file_name().unwrap_or_default().to_string_lossy(),
            ));
            editor.setFont(Some(&NSFont::systemFontOfSize(13.)));
            editor.setDelegate(Some(ProtocolObject::from_ref(self)));
            self.views().root.addSubview(&editor);
            {
                let mut state = self.ivars().state.borrow_mut();
                state.renaming = Some(path.clone());
                state.rename_editor = Some(editor.clone());
            }
            editor.selectText(None);
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let whole_name = self.with_tab(|t| {
                t.entries
                    .iter()
                    .find(|e| e.path == path)
                    .is_some_and(|e| matches!(e.kind, Kind::Folder | Kind::Symlink))
            });
            let length = if whole_name {
                name.encode_utf16().count()
            } else {
                path.file_stem()
                    .map(|s| s.to_string_lossy().encode_utf16().count())
                    .unwrap_or_else(|| name.encode_utf16().count())
            };
            if let Some(text) = editor.currentEditor() {
                text.setSelectedRange(NSRange::new(0, length));
            }
        }
    }
    fn rename_dialog(&self, path: PathBuf) {
        unsafe {
            let field =
                NSTextField::initWithFrame(NSTextField::alloc(self.mtm()), rect(0., 0., 360., 28.));
            single_line_editor(&field);
            field.setStringValue(&NSString::from_str(
                &path.file_name().unwrap_or_default().to_string_lossy(),
            ));
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("이름 변경"));
            alert.addButtonWithTitle(ns_string!("변경"));
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.setAccessoryView(Some(&field));
            let controller = self.retained();
            let callback = block2::RcBlock::new(move |response: isize| {
                if response == 1000 {
                    controller.rename_to(path.clone(), field.stringValue().to_string());
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
        }
    }
    fn cancel_inline_rename(&self) {
        let editor = {
            let mut s = self.ivars().state.borrow_mut();
            s.renaming = None;
            s.rename_editor.take()
        };
        if let Some(editor) = editor {
            editor.removeFromSuperview();
            self.focus_list();
        }
    }
    fn rename_to(&self, path: PathBuf, name: String) {
        if path
            .file_name()
            .is_some_and(|s| s == std::ffi::OsStr::new(&name))
        {
            self.load(true);
            return;
        }
        self.start_operation(Operation {
            completed_sources: vec![],
            renames: vec![],
            kind: OperationKind::Rename,
            sources: vec![StoredPath::new(&path)],
            destination: None,
            name: Some(name.into_bytes()),
        });
    }
    fn finish_rename(&self, notification: &NSNotification) {
        let path = self.ivars().state.borrow_mut().renaming.take();
        let Some(path) = path else { return };
        if let Some(object) = notification.object()
            && let Ok(field) = object.downcast::<NSTextField>()
        {
            let name = field.stringValue().to_string();
            let editor = self.ivars().state.borrow_mut().rename_editor.take();
            if let Some(editor) = editor {
                editor.removeFromSuperview();
            }
            self.focus_list();
            self.rename_to(path, name);
        }
    }
    pub fn settings(&self) {
        let existing = self.ivars().state.borrow().appearance_panel.clone();
        let panel = existing.unwrap_or_else(|| unsafe {
            super::appearance::AppearancePanel::new(self.mtm(), self.ivars().runtime.clone(), self)
        });
        self.ivars().state.borrow_mut().appearance_panel = Some(panel.clone());
        panel.show();
    }
    fn general_settings(&self) {
        unsafe {
            let prefs = self.ivars().state.borrow().prefs.clone();
            let form = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0., 0., 440., 330.));
            let title = text(self.mtm(), "키보드 프로필", 13.);
            title.setFrame(rect(0., 297., 120., 22.));
            form.addSubview(&title);
            let profile = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(self.mtm()),
                rect(130., 295., 300., 28.),
                false,
            );
            profile.addItemWithTitle(ns_string!("Windows식 · Ctrl + Cmd 지원"));
            profile.addItemWithTitle(ns_string!("macOS Finder식"));
            profile.selectItemAtIndex(if prefs.keyboard == KeyboardProfile::Windows {
                0
            } else {
                1
            });
            form.addSubview(&profile);
            let help = text(
                self.mtm(),
                "Windows: Enter 열기 · F2 또는 Ctrl/⌘Enter 이름 변경\nFinder: ⌘↓ 열기 · Return/F2 또는 ⌘Enter 이름 변경",
                12.,
            );
            help.setFrame(rect(0., 240., 440., 45.));
            help.setMaximumNumberOfLines(2);
            help.setTextColor(Some(&NSColor::secondaryLabelColor()));
            form.addSubview(&help);
            let system_f2 = NSButton::checkboxWithTitle_target_action(
                ns_string!("F2 호환 모드 (이 앱의 파일 목록에서만)"),
                None,
                None,
                self.mtm(),
            );
            system_f2.setFrame(rect(0., 205., 440., 24.));
            system_f2.setState(if prefs.system_f2_rename {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            form.addSubview(&system_f2);
            let compatibility_help = text(
                self.mtm(),
                &format!(
                    "밝기 키로 전달되는 F2를 항목 하나의 이름 변경에 사용합니다.\n선택 사항 · 손쉬운 사용 허용 필요 · 시스템 키 설정은 유지\n현재: {}",
                    super::system_f2_status().label()
                ),
                11.,
            );
            compatibility_help.setFrame(rect(0., 150., 440., 50.));
            compatibility_help.setMaximumNumberOfLines(3);
            compatibility_help.setTextColor(Some(&NSColor::secondaryLabelColor()));
            form.addSubview(&compatibility_help);
            let access = NSButton::buttonWithTitle_target_action(
                ns_string!("손쉬운 사용 설정 열기…"),
                Some(self),
                Some(sel!(openAccessibilitySettings:)),
                self.mtm(),
            );
            access.setFrame(rect(0., 115., 205., 28.));
            form.addSubview(&access);
            let hidden = NSButton::checkboxWithTitle_target_action(
                ns_string!("숨김 파일 표시"),
                None,
                None,
                self.mtm(),
            );
            hidden.setFrame(rect(0., 80., 250., 24.));
            hidden.setState(if prefs.hidden {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            form.addSubview(&hidden);
            let compact = NSButton::checkboxWithTitle_target_action(
                ns_string!("목록 간격 줄이기 (24pt)"),
                None,
                None,
                self.mtm(),
            );
            compact.setFrame(rect(0., 50., 300., 24.));
            compact.setState(if prefs.compact {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            form.addSubview(&compact);
            let preview = NSButton::checkboxWithTitle_target_action(
                ns_string!("미리보기 패널 표시"),
                None,
                None,
                self.mtm(),
            );
            preview.setFrame(rect(0., 20., 300., 24.));
            preview.setState(if prefs.preview {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            form.addSubview(&preview);
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("Rust Explorer 설정"));
            alert.setInformativeText(ns_string!(
                "설정은 모든 창에 적용됩니다. 파일 이름과 확장자는 원문 그대로 표시합니다."
            ));
            alert.addButtonWithTitle(ns_string!("적용"));
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.addButtonWithTitle(ns_string!("폴더 접근 설정…"));
            alert.setAccessoryView(Some(&form));
            let runtime = self.ivars().runtime.clone();
            let controller = self.retained();
            let callback = block2::RcBlock::new(move |response: isize| {
                if response == 1002 {
                    controller.ivars().state.borrow_mut().pending_access =
                        Some(folder_access::PendingAccess::Settings);
                }
                if response == 1000 {
                    let mut prefs = runtime.preferences.lock().unwrap().clone();
                    prefs.keyboard = if profile.indexOfSelectedItem() == 0 {
                        KeyboardProfile::Windows
                    } else {
                        KeyboardProfile::Finder
                    };
                    prefs.hidden = hidden.state() == NSControlStateValueOn;
                    prefs.compact = compact.state() == NSControlStateValueOn;
                    prefs.preview = preview.state() == NSControlStateValueOn;
                    prefs.system_f2_rename = system_f2.state() == NSControlStateValueOn;
                    runtime.save_preferences(prefs);
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
        }
    }
    pub fn cancel_size(&self) {
        self.ivars().state.borrow().size_cancel.cancel();
    }
    fn calculate_size(&self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.cancel_size();
        let cancel = Cancellation::default();
        self.ivars().state.borrow_mut().size_cancel = cancel.clone();
        self.views()
            .status
            .setStringValue(ns_string!("선택 항목 크기를 계산하는 중… · Esc로 취소"));
        let tx = self.ivars().runtime.sender.clone();
        let window = self.id();
        if let Err(e) = self.ivars().runtime.sizing.submit(move || {
            let result = crate::core::listing::selection_size(paths, &cancel);
            if cancel.cancelled() {
                return;
            }
            let detail = match result {
                Ok((bytes, files)) => {
                    format!("{files}개 파일 · 총 논리 크기 {}", format_size(bytes))
                }
                Err(e) => e.to_string(),
            };
            let _ = tx.send(Event::Message {
                window,
                title: "선택 항목 크기".into(),
                detail,
            });
        }) {
            self.message("계산을 시작할 수 없습니다", e);
        }
    }
    unsafe fn layout_preview(&self) {
        if let Some(view) = &self.ivars().state.borrow().preview_view {
            view.setFrame(self.views().preview_image.frame());
        }
    }
    fn update_preview(&self) {
        unsafe {
            if self.views().preview.isHidden() {
                let old = self.ivars().state.borrow_mut().preview_view.take();
                if let Some(v) = old {
                    v.removeFromSuperview();
                }
                self.ivars().state.borrow_mut().preview_path = None;
                return;
            }
            let e = self.with_tab(|t| {
                t.entries
                    .iter()
                    .find(|e| t.selected.contains(&e.id))
                    .map(|e| t.metadata.get(&e.id).unwrap_or(e).clone())
            });
            let Some(e) = e else {
                self.views()
                    .preview_title
                    .setStringValue(ns_string!("선택 항목 없음"));
                self.views().preview_details.setStringValue(ns_string!(
                    "파일을 선택하면 미리보기와 세부 정보를 확인할 수 있습니다."
                ));
                let old = self.ivars().state.borrow_mut().preview_view.take();
                if let Some(v) = old {
                    v.removeFromSuperview();
                }
                self.ivars().state.borrow_mut().preview_path = None;
                self.views().preview_image.setImage(None);
                return;
            };
            self.views()
                .preview_title
                .setStringValue(&NSString::from_str(&e.name));
            self.views()
                .preview_details
                .setStringValue(&NSString::from_str(&format!(
                    "{}\n{}\n\n{}{}",
                    e.kind_label(),
                    e.size
                        .map(format_size)
                        .unwrap_or_else(|| "폴더 크기: 우클릭 → 선택 항목 크기 계산".into()),
                    e.path.display(),
                    if e.readonly { "\n\n읽기 전용" } else { "" }
                )));
            self.views()
                .preview_image
                .setImage(self.ivars().state.borrow_mut().images.icon(&e).as_deref());
            if self.ivars().state.borrow().preview_path.as_ref() == Some(&e.path) {
                return;
            }
            let old = self.ivars().state.borrow_mut().preview_view.take();
            if let Some(v) = old {
                v.removeFromSuperview();
            }
            self.views()
                .preview_image
                .setImage(self.ivars().state.borrow_mut().images.icon(&e).as_deref());
            if e.kind != Kind::Folder
                && let Some(view) = QLPreviewView::initWithFrame_style(
                    QLPreviewView::alloc(self.mtm()),
                    self.views().preview_image.frame(),
                    QLPreviewViewStyle::Compact,
                )
            {
                view.setPreviewItem(Some(platform::preview_item(&platform::file_url(&e.path))));
                self.views().preview.addSubview(&view);
                self.ivars().state.borrow_mut().preview_view = Some(view);
            }
            self.ivars().state.borrow_mut().preview_path = Some(e.path);
        }
    }
    fn quick_look(&self) {
        unsafe {
            let paths = self.selected_paths();
            if paths.is_empty() {
                return;
            }
            let Some(panel) = QLPreviewPanel::sharedPreviewPanel(self.mtm()) else {
                return;
            };
            if panel.isVisible() {
                panel.orderOut(None);
                return;
            }
            self.ivars().state.borrow_mut().quick_urls =
                paths.iter().map(|p| platform::file_url(p)).collect();
            panel.setDataSource(Some(ProtocolObject::from_ref(self)));
            panel.reloadData();
            panel.makeKeyAndOrderFront(None);
        }
    }
    pub fn job_started(&self, id: u64, label: &str) {
        self.ivars().state.borrow_mut().active_job = Some(id);
        self.ivars().state.borrow_mut().job_reveal_due =
            Some(Instant::now() + Duration::from_millis(450));
        self.ivars().state.borrow_mut().notice = None;
        self.views().show_notice_undo(false);
        self.views().set_job_controls(true, false);
        unsafe {
            self.views()
                .job_title
                .setStringValue(&NSString::from_str(&format!("{label} 진행 중")));
            self.views().job_progress.setIndeterminate(true);
            self.views().job_progress.startAnimation(None);
            self.views().layout();
        }
    }
    pub fn job_progress(&self, id: u64, p: &Progress) {
        let active = self.ivars().runtime.jobs.lock().unwrap().len();
        if active == 0 {
            return;
        }
        self.ivars().state.borrow_mut().active_job = Some(id);
        self.views()
            .job_title
            .setStringValue(&NSString::from_str(&format!(
                "파일 작업 · {active}개 진행 중"
            )));
        unsafe {
            self.views().job_progress.setIndeterminate(p.total == 0);
            if p.total > 0 {
                self.views().job_progress.stopAnimation(None);
                self.views()
                    .job_progress
                    .setDoubleValue((p.current as f64 / p.total as f64 * 100.).min(100.));
            }
            self.views()
                .job_detail
                .setStringValue(&NSString::from_str(&format!(
                    "현재 파일: {}  ·  {}개 완료  ·  {}",
                    p.name,
                    p.files,
                    format_size(p.bytes)
                )));
        }
    }
    pub fn job_done(&self, id: u64, journal: Journal) {
        if matches!(
            journal.request.kind,
            OperationKind::Rename | OperationKind::BatchRename
        ) {
            for step in &journal.undo {
                if let UndoStep::Renamed { from, to, .. } = step {
                    self.with_tab_mut(|t| {
                        if t.selected.remove(&path_id(&to.path())) {
                            t.selected.insert(path_id(&from.path()));
                        }
                    });
                }
            }
        }
        {
            let mut s = self.ivars().state.borrow_mut();
            if s.active_job == Some(id) {
                s.active_job = None;
            }
            s.last_job = Some(journal.clone());
        }
        unsafe {
            let state = match journal.status.as_str() {
                "complete" => "완료",
                "cancelled" => "취소됨",
                "partial" => "일부 완료",
                _ => "실패",
            };
            self.views()
                .job_title
                .setStringValue(&NSString::from_str(&format!(
                    "{} {state}",
                    journal.request.label()
                )));
            self.views().job_progress.stopAnimation(None);
            self.views().job_progress.setIndeterminate(false);
            self.views()
                .job_progress
                .setDoubleValue(if journal.status == "complete" {
                    100.
                } else {
                    0.
                });
            let detail = if journal.errors.is_empty() {
                format!(
                    "{}개 항목 완료 · {}개 건너뜀",
                    journal.completed.len(),
                    journal.skipped
                )
            } else {
                journal.errors.join("\n")
            };
            self.views()
                .job_detail
                .setStringValue(&NSString::from_str(&detail));
            self.views()
                .job_detail
                .setToolTip(Some(&NSString::from_str(&detail)));
            self.finish_paste_clipboard(id, &journal, &self.clipboard_board());
        }
        let remaining: Vec<_> = self
            .ivars()
            .runtime
            .jobs
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect();
        self.views()
            .set_job_controls(!remaining.is_empty(), journal.can_retry());
        if let Some(next) = remaining.first() {
            self.ivars().state.borrow_mut().active_job = Some(*next);
            self.views()
                .job_title
                .setStringValue(&NSString::from_str(&format!(
                    "파일 작업 · {}개 진행 중",
                    remaining.len()
                )));
        } else {
            self.ivars().state.borrow_mut().job_reveal_due = None;
            let success = journal.status == "complete";
            self.views().jobs.setHidden(success);
            unsafe {
                self.views().layout();
            }
            if success {
                self.notice(
                    &format!("{} 완료", journal.request.label()),
                    journal.can_undo().then(|| journal.id.clone()),
                );
            }
        }
        let extraction = self.ivars().state.borrow_mut().extraction_jobs.remove(&id);
        let creation = self.ivars().state.borrow_mut().creation_jobs.remove(&id);
        if self.ivars().state.borrow().renaming.is_some() {
            self.with_tab_mut(|t| t.watch_due = Some(Instant::now() + Duration::from_millis(250)));
        } else {
            if journal.status == "complete"
                && extraction
                    .is_some_and(|(tab, path)| self.with_tab(|t| t.id == tab && t.path == path))
                && let Some(UndoStep::Created { path, .. }) = journal.undo.last()
            {
                self.navigate(path.path(), true);
                self.focus_list();
            } else {
                if journal.status == "complete"
                    && let Some((tab, folder, rename)) = creation
                    && self.with_tab(|t| t.id == tab && t.path == folder)
                    && let Some(UndoStep::Created { .. }) = journal.undo.last()
                {
                    self.ivars().state.borrow_mut().pending_creation =
                        Some(creation::PendingCreation {
                            tab,
                            folder,
                            paths: journal
                                .undo
                                .iter()
                                .filter_map(|step| {
                                    if let UndoStep::Created { path, .. } = step {
                                        Some(path.path())
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                            rename,
                            expires: Instant::now() + Duration::from_secs(10),
                        });
                }
                self.load(true);
            }
        }
    }
    fn finish_paste_clipboard(&self, id: u64, journal: &Journal, board: &NSPasteboard) {
        let count = self.ivars().state.borrow_mut().paste_moves.remove(&id);
        if journal.status == "complete"
            && journal.request.kind == OperationKind::Move
            && let Some(count) = count
            && count == board.changeCount()
        {
            board.clearContents();
            let mut state = self.ivars().state.borrow_mut();
            if state.last_clipboard == Some(count) {
                state.last_clipboard = None;
            }
        }
    }
    pub fn undo_done(&self, id: u64, journal: Option<&Journal>) {
        let next = self
            .ivars()
            .runtime
            .jobs
            .lock()
            .unwrap()
            .keys()
            .next()
            .copied();
        {
            let mut state = self.ivars().state.borrow_mut();
            if state.active_job == Some(id) {
                state.active_job = next;
            }
            if let Some(journal) = journal {
                state.last_job = Some(journal.clone());
            }
        }
        self.views().set_job_controls(next.is_some(), false);
        if next.is_none() {
            self.ivars().state.borrow_mut().job_reveal_due = None;
            self.views().jobs.setHidden(journal.is_some());
            unsafe {
                self.views().layout();
            }
            if journal.is_some() {
                self.notice("되돌리기 완료", None);
            }
        }
        unsafe {
            if next.is_none() {
                self.views().job_progress.stopAnimation(None);
                self.views().job_progress.setIndeterminate(false);
                self.views()
                    .job_progress
                    .setDoubleValue(if journal.is_some() { 100. } else { 0. });
                self.views()
                    .job_title
                    .setStringValue(&NSString::from_str(if journal.is_some() {
                        "실행 취소 완료"
                    } else {
                        "실행 취소 중단"
                    }));
                self.views()
                    .job_detail
                    .setStringValue(ns_string!("작업 기록에서 결과를 확인할 수 있습니다"));
            }
        }
        self.load(true);
    }
    pub fn show_conflict(
        &self,
        conflict: Conflict,
        reply: crossbeam_channel::Sender<ConflictDecision>,
    ) {
        unsafe {
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(&NSString::from_str(&format!(
                "‘{}’ 항목이 이미 있습니다",
                conflict
                    .target
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )));
            let date = |s| {
                chrono::DateTime::from_timestamp(s, 0)
                    .map(|d| {
                        d.with_timezone(&chrono::Local)
                            .format("%Y.%m.%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_default()
            };
            alert.setInformativeText(&NSString::from_str(&format!(
                "원본  {} · {}\n{}\n\n대상  {} · {}\n{}",
                if conflict.folders {
                    "폴더".into()
                } else {
                    format_size(conflict.source_size)
                },
                date(conflict.source_modified),
                conflict.source.display(),
                if conflict.folders {
                    "폴더".into()
                } else {
                    format_size(conflict.target_size)
                },
                date(conflict.target_modified),
                conflict.target.display()
            )));
            alert.addButtonWithTitle(ns_string!("둘 다 유지"));
            alert.addButtonWithTitle(ns_string!("건너뛰기"));
            alert.addButtonWithTitle(if conflict.folders {
                ns_string!("폴더 병합")
            } else {
                ns_string!("대체")
            });
            alert.addButtonWithTitle(ns_string!("작업 취소"));
            alert.setShowsSuppressionButton(true);
            if let Some(button) = alert.suppressionButton() {
                button.setTitle(ns_string!("이 작업의 같은 종류 충돌에 모두 적용"));
            }
            let retained_alert = alert.clone();
            let callback = block2::RcBlock::new(move |response: isize| {
                let choice = match response {
                    1000 => ConflictChoice::KeepBoth,
                    1001 => ConflictChoice::Skip,
                    1002 => {
                        if conflict.folders {
                            ConflictChoice::Merge
                        } else {
                            ConflictChoice::Replace
                        }
                    }
                    _ => ConflictChoice::Cancel,
                };
                let apply_all = retained_alert
                    .suppressionButton()
                    .is_some_and(|b| b.state() == NSControlStateValueOn);
                let _ = reply.send(ConflictDecision { choice, apply_all });
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
        }
    }
    pub fn refresh_input_diagnostics(&self) {
        if self.ivars().state.borrow().input_diagnostics {
            self.update_status();
        }
    }
    pub fn system_rename_target(&self) -> Option<super::SystemRenameTarget> {
        unsafe {
            let state = self.ivars().state.borrow();
            if !state.prefs.system_f2_rename || state.renaming.is_some() || state.dragging {
                return None;
            }
            let views = self.views();
            if !views.window.isKeyWindow()
                || views.window.attachedSheet().is_some()
                || NSApplication::sharedApplication(self.mtm())
                    .modalWindow()
                    .is_some()
                || NSRunLoop::mainRunLoop()
                    .currentMode()
                    .is_some_and(|m| *m != *NSDefaultRunLoopMode)
            {
                return None;
            }
            let expected: &NSResponder = if state.prefs.view == ViewMode::Icons {
                &views.collection
            } else {
                &views.table
            };
            let responder = views.window.firstResponder()?;
            if *responder != *expected {
                return None;
            }
            if responder.respondsToSelector(sel!(hasMarkedText)) {
                let marked: bool = msg_send![&*responder, hasMarkedText];
                if marked {
                    return None;
                }
            }
            let tab = &state.tabs[state.active];
            if tab.selected.len() != 1 || tab.read_only {
                return None;
            }
            let tab_id = tab.id;
            drop(state);
            let mut paths = self.selected_paths();
            (paths.len() == 1).then(|| super::SystemRenameTarget {
                window: self.id(),
                tab: tab_id,
                path: paths.remove(0),
            })
        }
    }
    pub fn system_f2_enabled(&self) -> bool {
        self.ivars().state.borrow().prefs.system_f2_rename
    }
    pub fn explain_system_f2_access(&self) {
        unsafe {
            if self.views().window.attachedSheet().is_some() {
                self.notice(
                    "F2 호환 모드: 손쉬운 사용 권한이 필요합니다 · 환경 설정에서 확인",
                    None,
                );
                return;
            }
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("F2 호환 모드 권한"));
            alert.setInformativeText(ns_string!("이 선택 기능은 macOS의 손쉬운 사용 허용이 필요합니다. 허용 후 이 앱의 파일 목록에서 항목 하나를 선택했을 때만 밝기 키로 전달되는 F2를 이름 변경에 사용합니다.\n\n시스템 기능 키 설정은 유지됩니다. 일반 F2와 Ctrl/⌘Enter 이름 변경은 추가 권한 없이 사용할 수 있습니다."));
            alert.addButtonWithTitle(ns_string!("설정 열기"));
            alert.addButtonWithTitle(ns_string!("나중에"));
            let callback = block2::RcBlock::new(move |response: isize| {
                if response == 1000
                    && let Some(url) = NSURL::URLWithString(ns_string!(
                        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
                    ))
                {
                    NSWorkspace::sharedWorkspace().openURL(&url);
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
        }
    }
    pub fn record_system_rename_input(&self, event: &NSEvent) {
        self.record_input_diagnostic(event);
    }
    pub fn rename_from_system_key(&self) {
        self.begin_rename();
    }
    fn record_input_diagnostic(&self, event: &NSEvent) {
        if !self.ivars().state.borrow().input_diagnostics {
            return;
        }
        unsafe {
            let key_event = matches!(event.r#type(), NSEventType::KeyDown | NSEventType::KeyUp);
            // Some SystemDefined events are broadcast to background apps.
            // Diagnostics are limited to this app's focused window.
            if !self.views().window.isKeyWindow()
                || !NSApplication::sharedApplication(self.mtm()).isActive()
            {
                return;
            }
            // IOKit NX_SUBTYPE_AUX_CONTROL_BUTTONS = 8. Subtype 7 is mouse
            // activity and must not overwrite the keyboard diagnostic.
            if !key_event
                && !(event.r#type() == NSEventType::SystemDefined && event.subtype().0 == 8)
            {
                return;
            }
            if event.r#type() != NSEventType::KeyDown
                && event
                    .window(self.mtm())
                    .is_some_and(|w| *w != *self.views().window)
            {
                return;
            }
            let flags = event.modifierFlags();
            let mut parts = vec![];
            for (flag, label) in [
                (NSEventModifierFlags::Command, "Cmd"),
                (NSEventModifierFlags::Control, "Ctrl"),
                (NSEventModifierFlags::Option, "Alt"),
                (NSEventModifierFlags::Shift, "Shift"),
                (NSEventModifierFlags::Function, "Fn"),
            ] {
                if flags.contains(flag) {
                    parts.push(label);
                }
            }
            let modifier = if parts.is_empty() {
                "없음".into()
            } else {
                parts.join("+")
            };
            let key = if key_event {
                format!(
                    "키 코드 {} · {}",
                    event.keyCode(),
                    if event.r#type() == NSEventType::KeyDown {
                        "누름"
                    } else {
                        "뗌"
                    }
                )
            } else {
                format!(
                    "시스템 키 · 유형 {} · 값 {}",
                    event.subtype().0,
                    event.data1()
                )
            };
            let focus = self
                .views()
                .window
                .firstResponder()
                .map(|r| r.class().name().to_string_lossy().into_owned())
                .unwrap_or_else(|| "없음".into());
            let selected = self.with_tab(|t| t.selected.len());
            {
                let mut state = self.ivars().state.borrow_mut();
                state.diagnostic_count += 1;
                let snapshot = format!(
                    "입력 #{}: {key} · 수정 키 {modifier} · 선택 {selected}개 · {focus}",
                    state.diagnostic_count
                );
                if state.diagnostic_events.len() == 12 {
                    state.diagnostic_events.pop_front();
                }
                state.diagnostic_events.push_back(snapshot);
            }
            self.update_status();
        }
    }
    pub fn handle_input(&self, event: &NSEvent) -> bool {
        // macOS consumes Control-Return for its contextual menu before the
        // window's sendEvent:. Claim only that Windows-profile command here,
        // while the focused file list owns the event; text/IME still pass.
        if event.r#type() == NSEventType::KeyDown {
            unsafe {
                let flags = event.modifierFlags();
                // Sheets use AppKit's own NSWindow rather than our subclass.
                // Keep Windows text-editing shortcuts local to our fields.
                if self.ivars().state.borrow().prefs.keyboard == KeyboardProfile::Windows
                    && flags.contains(NSEventModifierFlags::Control)
                    && !flags
                        .intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Option)
                    && NSApplication::sharedApplication(self.mtm()).isActive()
                    && let Some(window) = event.window(self.mtm())
                    && window.isKeyWindow()
                    && (*window == *self.views().window
                        || window
                            .sheetParent()
                            .is_some_and(|parent| *parent == *self.views().window))
                    && let Some(responder) = window.firstResponder()
                    && responder.isKindOfClass(NSTextView::class())
                {
                    let marked: bool = msg_send![&*responder, hasMarkedText];
                    if !marked {
                        let key = crate::core::keyboard::shortcut_key(
                            event.keyCode(),
                            &event
                                .charactersIgnoringModifiers()
                                .map(|s| s.to_string())
                                .unwrap_or_default(),
                        );
                        let action = match key.as_str() {
                            "c" => Some(sel!(copy:)),
                            "x" => Some(sel!(cut:)),
                            "v" => Some(sel!(paste:)),
                            "a" => Some(sel!(selectAll:)),
                            "z" if flags.contains(NSEventModifierFlags::Shift) => Some(sel!(redo:)),
                            "z" => Some(sel!(undo:)),
                            "y" => Some(sel!(redo:)),
                            _ => None,
                        };
                        if let Some(action) = action
                            && responder.respondsToSelector(action)
                        {
                            return NSApplication::sharedApplication(self.mtm())
                                .sendAction_to_from(action, Some(&*responder), None);
                        }
                    }
                }
                let expected: &NSResponder =
                    if self.ivars().state.borrow().prefs.view == ViewMode::Icons {
                        &self.views().collection
                    } else {
                        &self.views().table
                    };
                if matches!(event.keyCode(), 36 | 76)
                    && flags.contains(NSEventModifierFlags::Control)
                    && !flags.intersects(
                        NSEventModifierFlags::Command
                            | NSEventModifierFlags::Option
                            | NSEventModifierFlags::Shift,
                    )
                    && self.ivars().state.borrow().prefs.keyboard == KeyboardProfile::Windows
                    && self.views().window.isKeyWindow()
                    && !self.ivars().state.borrow().dragging
                    && !self.ivars().state.borrow().selecting
                    && event
                        .window(self.mtm())
                        .is_none_or(|window| *window == *self.views().window)
                    && !NSRunLoop::mainRunLoop()
                        .currentMode()
                        .is_some_and(|mode| *mode != *NSDefaultRunLoopMode)
                    && NSApplication::sharedApplication(self.mtm()).isActive()
                    && self
                        .views()
                        .window
                        .firstResponder()
                        .is_some_and(|r| *r == *expected)
                {
                    return self.handle_key(event);
                }
            }
            return false;
        }
        self.record_input_diagnostic(event);
        if matches!(
            event.r#type(),
            NSEventType::SystemDefined | NSEventType::KeyUp
        ) {
            return false;
        }
        unsafe {
            if self.views().window.attachedSheet().is_some()
                || !event
                    .window(self.mtm())
                    .is_some_and(|w| *w == *self.views().window)
            {
                return false;
            }
            if let Some(forward) =
                crate::core::keyboard::mouse_history_direction(event.buttonNumber())
            {
                if event.r#type() == NSEventType::OtherMouseUp {
                    self.go_back(forward);
                    self.focus_list();
                }
                return true;
            }
        }
        false
    }
    pub fn handle_key(&self, event: &NSEvent) -> bool {
        unsafe {
            // Invoked by this window's sendEvent:, so it cannot act on keys
            // belonging to another explorer window or a different application.
            if self.views().window.attachedSheet().is_some() {
                return false;
            }
            let code = event.keyCode();
            if self.ivars().state.borrow().selecting {
                if code == 53 {
                    super::selection::finish(&self.views().table, &self.views().collection, true);
                }
                return true;
            }
            let modifiers = event.modifierFlags();
            let cmd = modifiers.contains(NSEventModifierFlags::Command);
            let ctrl = modifiers.contains(NSEventModifierFlags::Control);
            let alt = modifiers.contains(NSEventModifierFlags::Option);
            let shift = modifiers.contains(NSEventModifierFlags::Shift);
            let windows = self.ivars().state.borrow().prefs.keyboard == KeyboardProfile::Windows;
            let primary = cmd || (windows && ctrl);
            let key = crate::core::keyboard::shortcut_key(
                code,
                &event
                    .charactersIgnoringModifiers()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            );
            let responder = self.views().window.firstResponder();
            let text_editing = responder
                .as_ref()
                .is_some_and(|r| r.isKindOfClass(NSTextView::class()));
            if text_editing
                && responder.as_ref().is_some_and(|r| {
                    let marked: bool = msg_send![&**r, hasMarkedText];
                    marked
                })
            {
                return false;
            }
            if primary && key == "l" {
                self.focus_address();
                return true;
            }
            if primary && key == "f" {
                self.views()
                    .window
                    .makeFirstResponder(Some(&self.views().search));
                self.views().search.selectText(None);
                return true;
            }
            if primary && shift && key == "g" {
                self.go_to_folder_dialog();
                return true;
            }
            if text_editing {
                if code == 53 && !self.views().address.isHidden() {
                    self.views().address.setHidden(true);
                    self.views().breadcrumb.setHidden(false);
                    self.focus_list();
                    return true;
                }
                if windows && ctrl && !cmd && !alt {
                    let action = match key.as_str() {
                        "c" => Some(sel!(copy:)),
                        "x" => Some(sel!(cut:)),
                        "v" => Some(sel!(paste:)),
                        "a" => Some(sel!(selectAll:)),
                        "z" => Some(sel!(undo:)),
                        _ => None,
                    };
                    if let Some(action) = action {
                        NSApplication::sharedApplication(self.mtm())
                            .sendAction_to_from(action, None, None);
                        return true;
                    }
                }
                if code == 53 && self.ivars().state.borrow().renaming.is_some() {
                    self.ivars().state.borrow_mut().renaming = None;
                    let editor = self.ivars().state.borrow_mut().rename_editor.take();
                    if let Some(editor) = editor {
                        editor.removeFromSuperview();
                    }
                    if self.ivars().state.borrow().prefs.view == ViewMode::Icons {
                        self.views()
                            .window
                            .makeFirstResponder(Some(&self.views().collection));
                    } else {
                        self.views()
                            .window
                            .makeFirstResponder(Some(&self.views().table));
                    }
                    self.reload();
                    return true;
                }
                return false;
            }
            if primary {
                match key.as_str() {
                    "+" | "=" => {
                        self.zoom_icons(true);
                        return true;
                    }
                    "-" => {
                        self.zoom_icons(false);
                        return true;
                    }
                    "a" => {
                        if self.ivars().state.borrow().prefs.view == ViewMode::Details {
                            self.views().table.selectAll(None);
                        } else {
                            self.views().collection.selectAll(None);
                        }
                        self.selection_changed();
                        return true;
                    }
                    "c" => {
                        self.copy(false);
                        return true;
                    }
                    "x" if windows => {
                        self.copy(true);
                        return true;
                    }
                    "v" => {
                        self.paste(cmd && alt);
                        return true;
                    }
                    "z" => {
                        self.ivars().runtime.undo(self.id());
                        return true;
                    }
                    "t" => {
                        self.new_tab(self.current_path());
                        return true;
                    }
                    "w" => {
                        self.close_tab(None);
                        return true;
                    }
                    "n" => {
                        if shift {
                            self.create_item(OperationKind::CreateFolder, None);
                        } else {
                            super::new_window(Some(self.current_path()));
                        }
                        return true;
                    }
                    "r" => {
                        self.load(true);
                        return true;
                    }
                    "," => {
                        self.settings();
                        return true;
                    }
                    "p" => {
                        self.toggle_pref("preview");
                        return true;
                    }
                    "[" => {
                        self.go_back(false);
                        return true;
                    }
                    "]" => {
                        self.go_back(true);
                        return true;
                    }
                    "." if shift => {
                        self.toggle_pref("hidden");
                        return true;
                    }
                    _ => {}
                }
                if code == 51 {
                    self.trash();
                    return true;
                }
            }
            if ctrl && code == 48 {
                let id = {
                    let s = self.ivars().state.borrow();
                    let n = s.tabs.len();
                    s.tabs[(s.active + n + if shift { n - 1 } else { 1 }) % n].id
                };
                self.select_tab(id);
                return true;
            }
            if (cmd && !windows || alt && windows) && code == 126 {
                if let Some(parent) = self.current_path().parent() {
                    self.navigate(parent.into(), true);
                }
                return true;
            }
            if (cmd && !windows) && code == 125 {
                self.open_selected();
                return true;
            }
            if windows && alt && code == 123 {
                self.go_back(false);
                return true;
            }
            if windows && alt && code == 124 {
                self.go_back(true);
                return true;
            }
            if code == 49 && !cmd && !ctrl && !alt {
                self.quick_look();
                return true;
            }
            if code == 36 || code == 76 {
                if primary && !alt && !shift {
                    self.begin_rename();
                    return true;
                }
                if responder
                    .as_ref()
                    .is_some_and(|r| r.isKindOfClass(NSOutlineView::class()))
                {
                    let row = self.views().sidebar.selectedRow();
                    if let Some(item) = self.views().sidebar.itemAtRow(row)
                        && let Some(id) = node_id(Some(&item))
                    {
                        self.sidebar_activate(id);
                    }
                    return true;
                }
                if windows {
                    self.open_selected();
                } else {
                    self.begin_rename();
                }
                return true;
            }
            if crate::core::keyboard::is_rename_key(
                code,
                &event
                    .charactersIgnoringModifiers()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            ) && !cmd
                && !ctrl
                && !alt
            {
                self.begin_rename();
                return true;
            }
            if code == 96 && windows && !cmd && !ctrl && !alt {
                self.load(true);
                return true;
            }
            if code == 117 && windows {
                self.trash();
                return true;
            }
            if code == 51 && windows {
                self.go_back(false);
                return true;
            }
            if code == 53 {
                self.cancel_size();
                if self.with_tab(|t| t.query.is_empty()) {
                    self.change_selection(false);
                    return true;
                }
                self.with_tab_mut(|t| t.query.clear());
                self.views().search.setStringValue(ns_string!(""));
                self.apply_filter();
                return true;
            }
            false
        }
    }
}

impl Controller {
    fn focus_list(&self) {
        unsafe {
            if self.ivars().state.borrow().prefs.view == ViewMode::Icons {
                self.views()
                    .window
                    .makeFirstResponder(Some(&self.views().collection));
            } else {
                self.views()
                    .window
                    .makeFirstResponder(Some(&self.views().table));
            }
        }
    }
    fn focus_address(&self) {
        unsafe {
            self.views().breadcrumb.setHidden(true);
            self.views().address.setHidden(false);
            self.views()
                .window
                .makeFirstResponder(Some(&self.views().address));
            self.views().address.selectText(None);
        }
    }
}

impl Controller {
    fn cancel_thumbnails(&self) {
        unsafe {
            let mut s = self.ivars().state.borrow_mut();
            for (request, _) in s.thumbnail_requests.values() {
                QLThumbnailGenerator::sharedGenerator().cancelRequest(request);
            }
            s.thumbnail_requests.clear();
            s.thumbnail_attempted.clear();
            s.images.clear_thumbnails();
        }
    }
    fn request_thumbnails(&self) {
        unsafe {
            if self.ivars().state.borrow().prefs.view != ViewMode::Icons {
                return;
            }
            let visible = self.views().collection.indexPathsForVisibleItems();
            for index in visible.iter() {
                let Some(entry) = self.entry_at(index.item()) else {
                    continue;
                };
                let extension = entry
                    .path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if !matches!(
                    extension.as_str(),
                    "png"
                        | "jpg"
                        | "jpeg"
                        | "heic"
                        | "webp"
                        | "gif"
                        | "tiff"
                        | "bmp"
                        | "pdf"
                        | "mov"
                        | "mp4"
                ) {
                    continue;
                }
                {
                    let s = self.ivars().state.borrow();
                    if s.thumbnail_requests.len() >= 2 {
                        break;
                    }
                    if s.thumbnail_attempted.contains(&entry.id) || s.images.has_thumbnail(entry.id)
                    {
                        continue;
                    }
                }
                let request =
                    QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
                        QLThumbnailGenerationRequest::alloc(),
                        &platform::file_url(&entry.path),
                        NSSize::new(152., 152.),
                        2.,
                        QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
                    );
                {
                    let mut s = self.ivars().state.borrow_mut();
                    s.thumbnail_attempted.insert(entry.id);
                    s.thumbnail_requests
                        .insert(entry.id, (request.clone(), Instant::now()));
                }
                let tx = self.ivars().runtime.sender.clone();
                let window = self.id();
                let (tab, generation) = self.with_tab(|t| (t.id, t.generation));
                let id = entry.id;
                let callback = block2::RcBlock::new(
                    move |representation: *mut QLThumbnailRepresentation, _error: *mut NSError| {
                        objc2::rc::autoreleasepool(|_| {
                            let result = representation.as_ref().and_then(|r| {
                                let image = r.CGImage();
                                let rep = NSBitmapImageRep::initWithCGImage(
                                    NSBitmapImageRep::alloc(),
                                    &image,
                                );
                                let cost = (rep.pixelsWide().max(0) as usize)
                                    * (rep.pixelsHigh().max(0) as usize)
                                    * 4;
                                let data = rep.representationUsingType_properties(
                                    NSBitmapImageFileType::PNG,
                                    &NSDictionary::new(),
                                )?;
                                Some((data.to_vec(), cost))
                            });
                            let (png, cost) =
                                result.map(|(p, c)| (Some(p), c)).unwrap_or((None, 0));
                            let _ = tx.try_send(Event::Thumbnail {
                                window,
                                tab,
                                generation,
                                id,
                                png,
                                cost,
                            });
                        });
                    },
                );
                QLThumbnailGenerator::sharedGenerator()
                    .generateBestRepresentationForRequest_completionHandler(&request, &callback);
            }
            let stale: Vec<_> = self
                .ivars()
                .state
                .borrow()
                .thumbnail_requests
                .iter()
                .filter(|(_, (_, time))| time.elapsed() > Duration::from_secs(15))
                .map(|(id, _)| *id)
                .collect();
            for id in stale {
                if let Some((request, _)) = self
                    .ivars()
                    .state
                    .borrow_mut()
                    .thumbnail_requests
                    .remove(&id)
                {
                    QLThumbnailGenerator::sharedGenerator().cancelRequest(&request);
                }
            }
        }
    }
}

impl Controller {
    fn show_history(&self) {
        unsafe {
            let records = self.ivars().runtime.history.lock().unwrap().clone();
            if records.is_empty() {
                self.message("작업 기록", "아직 완료되거나 중단된 파일 작업이 없습니다.");
                return;
            }
            let form = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0., 0., 530., 300.));
            let popup = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(self.mtm()),
                rect(0., 266., 530., 28.),
                false,
            );
            for (record_index, j) in records.iter().enumerate() {
                let name = j
                    .request
                    .sources
                    .first()
                    .map(|s| {
                        s.path()
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    })
                    .unwrap_or_else(|| "새 폴더".into());
                popup.addItemWithTitle(&NSString::from_str(&format!(
                    "{} · {} · {} · {}",
                    record_index + 1,
                    j.request.label(),
                    job_state(&j.status),
                    name.chars().take(38).collect::<String>()
                )));
            }
            popup.setTarget(Some(self));
            popup.setAction(Some(sel!(historyChanged:)));
            form.addSubview(&popup);
            let details = text(self.mtm(), "", 12.);
            details.setFrame(rect(0., 0., 530., 254.));
            details.setMaximumNumberOfLines(15);
            details.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            details.setSelectable(true);
            form.addSubview(&details);
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("파일 작업 기록"));
            alert.setInformativeText(ns_string!("완료 상태와 원래 위치를 확인할 수 있습니다. 대체한 파일은 되돌리기를 위해 복구 파일로 보관됩니다."));
            let buttons = vec![
                alert.addButtonWithTitle(ns_string!("닫기")),
                alert.addButtonWithTitle(ns_string!("재시도")),
                alert.addButtonWithTitle(ns_string!("되돌리기")),
                alert.addButtonWithTitle(ns_string!("복구 파일 보기")),
            ];
            alert.setAccessoryView(Some(&form));
            self.ivars().state.borrow_mut().history = Some(HistoryDialog {
                popup,
                details,
                buttons,
                records,
            });
            self.update_history();
            let controller = self.retained();
            let block = block2::RcBlock::new(move |response: isize| {
                let dialog = controller.ivars().state.borrow_mut().history.take();
                if let Some(dialog) = dialog
                    && let Some(record) = dialog
                        .records
                        .get(dialog.popup.indexOfSelectedItem().max(0) as usize)
                {
                    match response {
                        1001 => {
                            controller.start_operation(retry_request(record));
                        }
                        1002 => controller
                            .ivars()
                            .runtime
                            .undo_selected(controller.id(), Some(record.id.clone())),
                        1003 => {
                            let urls: Vec<_> = record
                                .recovery_paths()
                                .iter()
                                .take(20)
                                .map(|p| platform::file_url(p))
                                .collect();
                            NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(
                                &NSArray::from_retained_slice(&urls),
                            );
                        }
                        _ => {}
                    }
                }
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&block));
        }
    }
    fn update_history(&self) {
        let s = self.ivars().state.borrow();
        let Some(dialog) = s.history.as_ref() else {
            return;
        };
        let Some(j) = dialog
            .records
            .get(dialog.popup.indexOfSelectedItem().max(0) as usize)
        else {
            return;
        };
        let mut detail = format!(
            "{} · {}\n완료 {}개 · 건너뜀 {}개\n",
            j.request.label(),
            job_state(&j.status),
            j.completed.len(),
            j.skipped
        );
        for source in j.request.sources.iter().take(3) {
            detail.push_str(&format!("\n원본: {}", source.path().display()));
        }
        if let Some(d) = &j.request.destination {
            detail.push_str(&format!("\n대상: {}", d.path().display()));
        }
        for error in j.errors.iter().take(3) {
            detail.push_str(&format!("\n\n{error}"));
        }
        let recoveries = j.recovery_paths();
        if !recoveries.is_empty() {
            detail.push_str(&format!("\n\n복구 파일 {}개 보관 중. ‘복구 파일 보기’로 Finder에서 확인할 수 있습니다. 복구 파일을 삭제하면 이 작업을 되돌릴 수 없습니다.",recoveries.len()));
        }
        dialog.details.setStringValue(&NSString::from_str(&detail));
        dialog.buttons[1].setEnabled(j.can_retry());
        dialog.buttons[2].setEnabled(j.can_undo());
        dialog.buttons[3].setEnabled(!recoveries.is_empty());
    }
}
fn job_state(state: &str) -> &str {
    match state {
        "complete" => "완료",
        "partial" => "일부 완료",
        "cancelled" => "취소됨",
        "interrupted" => "중단됨",
        "review" => "확인 필요",
        "undone" => "되돌림",
        "failed" => "실패",
        _ => "진행 중",
    }
}

impl Controller {
    fn change_selection(&self, invert: bool) {
        if self.views().window.attachedSheet().is_some() {
            return;
        }
        self.with_tab_mut(|t| {
            t.selected = if invert {
                t.visible
                    .iter()
                    .filter_map(|&i| t.entries.get(i))
                    .filter(|e| !t.selected.contains(&e.id))
                    .map(|e| e.id)
                    .collect()
            } else {
                HashSet::new()
            };
        });
        self.reload();
        self.focus_list();
    }
}

impl Controller {
    fn scroll_surface(&self) -> (&NSScrollView, usize) {
        if self.ivars().state.borrow().prefs.view == ViewMode::Details {
            (&self.views().list_scroll, 0)
        } else {
            (&self.views().icon_scroll, 1)
        }
    }
    fn remember_scroll(&self) {
        let (scroll, index) = self.scroll_surface();
        let origin = scroll.contentView().bounds().origin;
        self.with_tab_mut(|t| {
            if !t.visible.is_empty() {
                t.scroll[index] = Some(origin);
            }
        });
    }
    fn restore_scroll(&self) {
        let (scroll, index) = self.scroll_surface();
        scroll.tile();
        let clip = scroll.contentView();
        let mut bounds = clip.bounds();
        // A native table's top is often negative because its header overlays
        // the clip view. Zero hides the first row. Let AppKit find the minimum.
        bounds.origin = self
            .with_tab(|t| t.scroll[index])
            .unwrap_or(NSPoint::new(-1_000_000., -1_000_000.));
        bounds = clip.constrainBoundsRect(bounds);
        clip.scrollToPoint(bounds.origin);
        scroll.reflectScrolledClipView(&clip);
        self.with_tab_mut(|t| {
            if !t.visible.is_empty() {
                t.scroll[index] = Some(bounds.origin);
            }
        });
    }
}

#[cfg(feature = "native-ui-tests")]
#[path = "drop_native_checks.rs"]
pub(super) mod native_drop_checks;

#[cfg(feature = "native-ui-tests")]
#[path = "theme_native_checks.rs"]
pub mod native_theme_checks;
