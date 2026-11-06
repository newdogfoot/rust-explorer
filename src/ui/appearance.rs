//! Virtualized native preset chooser. Only the selected preview is constructed.
#![allow(unsafe_op_in_unsafe_fn)]
use super::{
    runtime::Runtime,
    theme,
    widgets::{rect, text},
};
use crate::core::theme::{Resolved, THEMES, ThemeId};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::{
    cell::{Cell, OnceCell},
    sync::Arc,
};

struct PreviewParts {
    header: Retained<NSView>,
    rail: Retained<NSView>,
    selection: Retained<NSView>,
    title: Retained<NSTextField>,
    side: Vec<Retained<NSTextField>>,
    labels: Vec<Retained<NSTextField>>,
    images: Vec<(theme::Glyph, Retained<NSImageView>)>,
}

pub struct PanelState {
    runtime: Arc<Runtime>,
    window: OnceCell<Retained<NSWindow>>,
    table: OnceCell<Retained<NSTableView>>,
    preview: OnceCell<Retained<NSView>>,
    title: OnceCell<Retained<NSTextField>>,
    detail: OnceCell<Retained<NSTextField>>,
    updating: Cell<bool>,
    selected: Cell<Option<ThemeId>>,
    neutral: Cell<Resolved>,
    preview_parts: OnceCell<PreviewParts>,
    preview_palette: Cell<Option<Resolved>>,
}
define_class!(
 #[unsafe(super=NSObject)]
 #[name="REAppearanceSettingsController"]
 #[thread_kind=MainThreadOnly]
 #[ivars=PanelState]
 pub struct AppearancePanel;
 unsafe impl NSObjectProtocol for AppearancePanel {}
 unsafe impl NSControlTextEditingDelegate for AppearancePanel {}
 unsafe impl NSTableViewDataSource for AppearancePanel {
  #[unsafe(method(numberOfRowsInTableView:))]
  fn count(&self,_:&NSTableView)->isize {31}
 }
 unsafe impl NSTableViewDelegate for AppearancePanel {
  #[unsafe(method_id(tableView:rowViewForRow:))]
  fn theme_row(&self,table:&NSTableView,_:isize)->Option<Retained<NSTableRowView>> {Some(unsafe{theme::row(self.mtm(),table,self.neutral(),false)})}
  #[unsafe(method_id(tableView:viewForTableColumn:row:))]
  fn cell(&self,table:&NSTableView,_:Option<&NSTableColumn>,row:isize)->Option<Retained<NSView>> {unsafe{self.theme_cell(table,row)}}
  #[unsafe(method(tableViewSelectionDidChange:))]
  fn selected(&self,_:&NSNotification){
    if self.ivars().updating.get(){return;}
    let row=self.table().selectedRow();if !(0..=30).contains(&row){return;}
    self.choose(ThemeId::new(row as u8));
  }
 }
 impl AppearancePanel {
  #[unsafe(method(closeSettings:))]
  fn close_panel(&self,_:Option<&NSObject>){self.window().close();}
 }
);
impl AppearancePanel {
    pub fn window(&self) -> &NSWindow {
        self.ivars().window.get().unwrap()
    }
    pub fn table(&self) -> &NSTableView {
        self.ivars().table.get().unwrap()
    }
    pub unsafe fn new(
        mtm: MainThreadMarker,
        runtime: Arc<Runtime>,
        owner: &NSObject,
    ) -> Retained<Self> {
        let obj = Self::alloc(mtm).set_ivars(PanelState {
            runtime,
            window: OnceCell::new(),
            table: OnceCell::new(),
            preview: OnceCell::new(),
            title: OnceCell::new(),
            detail: OnceCell::new(),
            updating: Cell::new(false),
            selected: Cell::new(None),
            neutral: Cell::default(),
            preview_parts: OnceCell::new(),
            preview_palette: Cell::new(None),
        });
        let obj: Retained<Self> = msg_send![super(obj), init];
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0., 0., 860., 600.),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable,
            NSBackingStoreType::Buffered,
            true,
        );
        window.setReleasedWhenClosed(false);
        window.setTitle(ns_string!("Rust Explorer 환경 설정 · 모양"));
        obj.ivars().window.set(window.clone()).ok();
        let root = NSView::initWithFrame(NSView::alloc(mtm), rect(0., 0., 860., 600.));
        let appearance = NSApplication::sharedApplication(mtm).effectiveAppearance();
        window.setAppearance(Some(&appearance));
        let neutral = ThemeId::default().resolve(appearance.name().to_string().contains("Dark"));
        obj.ivars().neutral.set(neutral);
        root.setWantsLayer(true);
        root.layer()
            .unwrap()
            .setBackgroundColor(Some(&theme::color(neutral.shell).CGColor()));
        window.setContentView(Some(&root));
        let heading = text(mtm, "모양 선택", 22.);
        heading.setFont(Some(&NSFont::boldSystemFontOfSize(22.)));
        heading.setFrame(rect(22., 548., 400., 32.));
        root.addSubview(&heading);
        let help = text(
            mtm,
            "선택하면 모든 창에 바로 적용되고, 다음 실행에도 유지됩니다.",
            13.,
        );
        help.setFrame(rect(22., 518., 810., 24.));
        root.addSubview(&help);
        let scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(22., 70., 292., 432.));
        scroll.setHasVerticalScroller(true);
        scroll.setBorderType(NSBorderType::BezelBorder);
        let table = NSTableView::initWithFrame(NSTableView::alloc(mtm), rect(0., 0., 278., 432.));
        table.setHeaderView(None);
        table.setBackgroundColor(&theme::color(neutral.canvas));
        scroll.setBackgroundColor(&theme::color(neutral.canvas));
        table.setRowHeight(64.);
        table.setIntercellSpacing(NSSize::new(0., 0.));
        table.setAllowsEmptySelection(false);
        table.setAllowsMultipleSelection(false);
        let column =
            NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("theme"));
        column.setWidth(278.);
        column.setMinWidth(260.);
        table.addTableColumn(&column);
        table.setDataSource(Some(ProtocolObject::from_ref(&*obj)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*obj)));
        scroll.setDocumentView(Some(&table));
        root.addSubview(&scroll);
        let title = text(mtm, "", 19.);
        title.setFrame(rect(338., 466., 500., 30.));
        title.setFont(Some(&NSFont::boldSystemFontOfSize(19.)));
        root.addSubview(&title);
        let detail = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
        detail.setFont(Some(&NSFont::systemFontOfSize(12.)));
        detail.setFrame(rect(338., 416., 500., 44.));
        detail.setSelectable(false);
        root.addSubview(&detail);
        let preview = NSView::initWithFrame(NSView::alloc(mtm), rect(338., 126., 500., 278.));
        root.addSubview(&preview);
        let note = text(mtm, "가로형 폴더 · 세로형 파일 · 파일 종류별 문양", 12.);
        note.setFrame(rect(338., 91., 500., 25.));
        root.addSubview(&note);
        let general = NSButton::buttonWithTitle_target_action(
            ns_string!("키보드·파일 표시 설정…"),
            Some(owner),
            Some(sel!(generalSettings:)),
            mtm,
        );
        general.setFrame(rect(22., 20., 235., 32.));
        root.addSubview(&general);
        let access = NSButton::buttonWithTitle_target_action(
            ns_string!("폴더 접근 설정…"),
            Some(owner),
            Some(sel!(settingsFolderAccess:)),
            mtm,
        );
        access.setFrame(rect(273., 20., 180., 32.));
        root.addSubview(&access);
        let close = NSButton::buttonWithTitle_target_action(
            ns_string!("닫기"),
            Some(&*obj),
            Some(sel!(closeSettings:)),
            mtm,
        );
        close.setFrame(rect(738., 20., 100., 32.));
        close.setKeyEquivalent(ns_string!("\u{1b}"));
        root.addSubview(&close);
        obj.ivars().table.set(table).ok();
        obj.ivars().preview.set(preview).ok();
        obj.ivars().title.set(title).ok();
        obj.ivars().detail.set(detail).ok();
        obj.ivars().window.set(window).ok();
        for view in obj.window().contentView().unwrap().subviews() {
            if let Some(label) = view.downcast_ref::<NSTextField>() {
                label.setTextColor(Some(&theme::color(neutral.ink)));
            }
        }
        let id = obj.ivars().runtime.preferences.lock().unwrap().theme;
        obj.sync(id);
        obj
    }
    fn neutral(&self) -> Resolved {
        unsafe {
            ThemeId::default().resolve(
                NSApplication::sharedApplication(self.mtm())
                    .effectiveAppearance()
                    .name()
                    .to_string()
                    .contains("Dark"),
            )
        }
    }
    pub fn sync_environment(&self) {
        unsafe {
            let neutral = self.neutral();
            if self.ivars().neutral.get() == neutral {
                return;
            }
            // Native reload/appearance changes may emit selection callbacks.
            // They must never overwrite the user's fixed preset.
            let id = self.ivars().selected.get().unwrap_or_default();
            self.ivars().updating.set(true);
            self.ivars().neutral.set(neutral);
            self.window().setAppearance(Some(
                &NSApplication::sharedApplication(self.mtm()).effectiveAppearance(),
            ));
            let root = self.window().contentView().unwrap();
            root.layer()
                .unwrap()
                .setBackgroundColor(Some(&theme::color(neutral.shell).CGColor()));
            for view in root.subviews() {
                if let Some(label) = view.downcast_ref::<NSTextField>() {
                    label.setTextColor(Some(&theme::color(neutral.ink)));
                }
            }
            self.table()
                .setBackgroundColor(&theme::color(neutral.canvas));
            self.table().reloadData();
            self.ivars().selected.set(None);
            self.sync(id);
            self.ivars().updating.set(false);
        }
    }
    #[cfg(feature = "native-ui-tests")]
    pub fn preview_theme(&self) -> Resolved {
        self.ivars().preview_palette.get().unwrap()
    }
    pub fn show(&self) {
        self.sync_environment();
        unsafe {
            let id = self.ivars().runtime.preferences.lock().unwrap().theme;
            self.sync(id);
            self.window().center();
            self.window().makeKeyAndOrderFront(None);
        }
    }
    pub fn choose(&self, id: ThemeId) {
        if self.ivars().runtime.preferences.lock().unwrap().theme == id {
            self.sync(id);
            return;
        }
        let mut prefs = self.ivars().runtime.preferences.lock().unwrap().clone();
        prefs.theme = id;
        self.ivars().runtime.save_preferences(prefs.clone());
        super::apply_live_preferences(&self.ivars().runtime, &prefs);
        self.sync(id);
    }
    pub fn sync(&self, id: ThemeId) {
        unsafe {
            if self.ivars().selected.get() == Some(id) {
                return;
            }
            self.ivars().selected.set(Some(id));
            self.ivars().updating.set(true);
            self.table().selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(id.index() as usize),
                false,
            );
            self.table().scrollRowToVisible(id.index() as isize);
            self.ivars().updating.set(false);
            let dark = NSApplication::sharedApplication(self.mtm())
                .effectiveAppearance()
                .name()
                .to_string()
                .contains("Dark");
            let p = id.resolve(dark);
            self.ivars()
                .title
                .get()
                .unwrap()
                .setStringValue(&NSString::from_str(&if id.index() == 0 {
                    "시스템에 맞추기".into()
                } else {
                    format!("{:02} {}", p.spec.id, p.spec.name)
                }));
            self.ivars()
                .detail
                .get()
                .unwrap()
                .setStringValue(&NSString::from_str(&if id.index() == 0 {
                    "밝은 화면은 01 익숙한 앰버, 어두운 화면은 18 미드나이트로 자동 전환합니다."
                        .into()
                } else {
                    format!(
                        "{}\n{} 테마 · 즉시 적용 · 자동 저장",
                        p.spec.description,
                        if p.spec.dark { "어두운" } else { "밝은" }
                    )
                }));
            self.preview(p);
        }
    }
    unsafe fn theme_cell(&self, table: &NSTableView, index: isize) -> Option<Retained<NSView>> {
        if !(0..=30).contains(&index) {
            return None;
        }
        let p = ThemeId::new(index as u8).resolve(false);
        let cell = table
            .makeViewWithIdentifier_owner(ns_string!("theme-choice"), None)
            .unwrap_or_else(|| {
                let v = NSView::initWithFrame(NSView::alloc(self.mtm()), rect(0., 0., 278., 64.));
                v.setIdentifier(Some(ns_string!("theme-choice")));
                v
            });
        for child in cell.subviews() {
            child.removeFromSuperview();
        }
        let image =
            NSImageView::initWithFrame(NSImageView::alloc(self.mtm()), rect(8., 14., 38., 38.));
        image.setImage(Some(&theme::icon(p, theme::Glyph::Folder)));
        image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        cell.addSubview(&image);
        let label = text(
            self.mtm(),
            &if index == 0 {
                "시스템에 맞추기".into()
            } else {
                format!("{:02} {}", index, p.spec.name)
            },
            13.,
        );
        label.setFrame(rect(54., 32., 214., 24.));
        label.setTextColor(Some(&theme::color(self.neutral().ink)));
        cell.addSubview(&label);
        let kind = text(
            self.mtm(),
            if index == 0 {
                "밝게 / 어둡게 자동"
            } else if p.spec.dark {
                "어두운 테마"
            } else {
                "밝은 테마"
            },
            11.,
        );
        kind.setFrame(rect(54., 10., 98., 20.));
        kind.setTextColor(Some(&theme::color(self.neutral().muted)));
        cell.addSubview(&kind);
        for (i, c) in [p.canvas, p.rail, p.accent, p.folder]
            .into_iter()
            .enumerate()
        {
            let swatch = NSView::initWithFrame(
                NSView::alloc(self.mtm()),
                rect(164. + i as f64 * 23., 13., 16., 16.),
            );
            swatch.setWantsLayer(true);
            if let Some(l) = swatch.layer() {
                l.setCornerRadius(8.);
                l.setBorderWidth(0.5);
                l.setBorderColor(Some(&NSColor::separatorColor().CGColor()));
                l.setBackgroundColor(Some(&theme::color(c).CGColor()));
            }
            cell.addSubview(&swatch);
        }
        cell.setToolTip(Some(&NSString::from_str(if index == 0 {
            "macOS의 밝은 화면과 어두운 화면을 따릅니다"
        } else {
            THEMES[index as usize - 1].description
        })));
        Some(cell)
    }
    unsafe fn preview(&self, p: Resolved) {
        if self.ivars().preview_palette.get() == Some(p) {
            return;
        }
        self.ivars().preview_palette.set(Some(p));
        let root = self.ivars().preview.get().unwrap();
        root.setWantsLayer(true);
        let parts = self.ivars().preview_parts.get_or_init(|| {
            let bg = |r| {
                let v = NSView::initWithFrame(NSView::alloc(self.mtm()), r);
                v.setWantsLayer(true);
                root.addSubview(&v);
                v
            };
            let header = bg(rect(1., 239., 498., 38.));
            let rail = bg(rect(1., 1., 111., 238.));
            let selection = bg(rect(117., 109., 376., 38.));
            let title = text(self.mtm(), "문서  ›  디자인 자료", 12.);
            title.setFrame(rect(14., 246., 460., 23.));
            root.addSubview(&title);
            let side = [
                "즐겨찾기",
                "문서",
                "다운로드",
                "iCloud Drive",
                "위치",
                "Macintosh HD",
            ]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let label = text(self.mtm(), name, 11.);
                label.setFrame(rect(10., 208. - i as f64 * 28., 99., 22.));
                root.addSubview(&label);
                label
            })
            .collect();
            let mut labels = vec![];
            let mut images = vec![];
            for (i, (name, g)) in [
                ("업무 자료", theme::Glyph::Folder),
                ("촬영 원본", theme::Glyph::Folder),
                ("프로젝트 제안서.pdf", theme::Glyph::Pdf),
                ("11월 예산.xlsx", theme::Glyph::Sheet),
                ("가나다라마바사아자차카타파하한.txt", theme::Glyph::Text),
            ]
            .into_iter()
            .enumerate()
            {
                let y = 197. - i as f64 * 43.;
                let image = NSImageView::initWithFrame(
                    NSImageView::alloc(self.mtm()),
                    rect(124., y, 30., 30.),
                );
                image.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                root.addSubview(&image);
                images.push((g, image));
                let label = text(self.mtm(), name, 13.);
                label.setFrame(rect(162., y + 3., 325., 24.));
                label.setToolTip(Some(&NSString::from_str(name)));
                root.addSubview(&label);
                labels.push(label);
            }
            PreviewParts {
                header,
                rail,
                selection,
                title,
                side,
                labels,
                images,
            }
        });
        if let Some(layer) = root.layer() {
            layer.setCornerRadius(p.radius());
            layer.setBorderWidth(1.);
            layer.setBorderColor(Some(&theme::color(p.line).CGColor()));
            layer.setBackgroundColor(Some(&theme::color(p.canvas).CGColor()));
        }
        let header = p.spec.layout == crate::core::theme::Layout::Header;
        for (view, color) in [
            (&parts.header, if header { p.accent } else { p.shell }),
            (&parts.rail, p.rail),
            (&parts.selection, p.selection),
        ] {
            view.layer()
                .unwrap()
                .setBackgroundColor(Some(&theme::color(color).CGColor()));
        }
        parts.title.setTextColor(Some(&theme::color(if header {
            p.on_accent
        } else {
            p.ink
        })));
        for label in &parts.side {
            label.setTextColor(Some(&theme::color(p.side_ink)));
        }
        for label in &parts.labels {
            label.setTextColor(Some(&theme::color(p.ink)));
        }
        let mut icons = std::collections::HashMap::new();
        for (g, view) in &parts.images {
            view.setImage(Some(icons.entry(*g).or_insert_with(|| theme::icon(p, *g))));
        }
    }
}
