// UI draft: agy / gemini-3.8-flash-high (high), 2026-10-30.
// Integrated and refined for native behavior and accessibility.
#![allow(unsafe_op_in_unsafe_fn)]

use super::widgets::{AddressPathControl, FileCollectionView, FileTableView, add_creation_items};
use crate::core::model::{IconSize, ViewMode};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAppearance, NSAppearanceCustomization, NSButton, NSCollectionView,
    NSCollectionViewFlowLayout, NSColor, NSFont, NSImage, NSImageView, NSLayoutAttribute, NSMenu,
    NSMenuItem, NSOutlineView, NSPathControl, NSPopUpButton, NSProgressIndicator, NSScrollView,
    NSSearchField, NSStackView, NSTableColumn, NSTableView, NSTextField,
    NSTitlebarAccessoryViewController, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::CALayer;

pub struct ExplorerViews {
    last_columns_width: std::cell::Cell<f64>,
    pub palette: std::cell::Cell<crate::core::theme::Resolved>,
    theme_choice: std::cell::Cell<crate::core::theme::ThemeId>,
    pub window: Retained<NSWindow>,
    pub root: Retained<NSView>,
    pub tabs: Retained<NSStackView>,
    pub tab_scroll: Retained<NSScrollView>,
    pub address: Retained<NSTextField>,
    pub breadcrumb: Retained<NSPathControl>,
    pub search: Retained<NSSearchField>,
    pub search_scope: Retained<NSPopUpButton>,
    pub sidebar: Retained<NSOutlineView>,
    pub table: Retained<NSTableView>,
    pub collection: Retained<NSCollectionView>,
    pub list_scroll: Retained<NSScrollView>,
    pub icon_scroll: Retained<NSScrollView>,
    pub preview: Retained<NSView>,
    pub preview_title: Retained<NSTextField>,
    pub preview_details: Retained<NSTextField>,
    pub preview_image: Retained<NSImageView>,
    pub status: Retained<NSTextField>,
    pub empty_label: Retained<NSTextField>,
    pub access_banner: Retained<NSView>,
    access_intro_label: Retained<NSTextField>,
    btn_access_intro: Retained<NSButton>,
    btn_access_dismiss: Retained<NSButton>,
    pub empty_actions: Retained<NSView>,
    pub btn_access_current: Retained<NSButton>,
    pub btn_access_help: Retained<NSButton>,
    pub btn_error_retry: Retained<NSButton>,
    pub btn_error_details: Retained<NSButton>,
    pub jobs: Retained<NSView>,
    pub job_title: Retained<NSTextField>,
    pub job_detail: Retained<NSTextField>,
    pub job_progress: Retained<NSProgressIndicator>,
    pub icon_size: Retained<NSPopUpButton>,
    flow_layout: Retained<NSCollectionViewFlowLayout>,
    btn_copy_address: Retained<NSButton>,
    btn_settings: Retained<NSButton>,
    btn_status_undo: Retained<NSButton>,

    // Private subview components for structured layout & theming
    tab_bar: Retained<NSView>,
    new_tab_btn: Retained<NSButton>,
    nav_bar: Retained<NSView>,
    btn_back: Retained<NSButton>,
    btn_forward: Retained<NSButton>,
    btn_up: Retained<NSButton>,
    btn_refresh: Retained<NSButton>,
    command_bar: Retained<NSView>,
    btn_new: Retained<NSPopUpButton>,
    btn_cut: Retained<NSButton>,
    btn_copy: Retained<NSButton>,
    btn_paste: Retained<NSButton>,
    btn_rename: Retained<NSButton>,
    btn_trash: Retained<NSButton>,
    btn_sort: Retained<NSPopUpButton>,
    btn_view_mode: Retained<NSPopUpButton>,
    btn_toggle_preview: Retained<NSButton>,
    btn_more: Retained<NSPopUpButton>,
    sidebar_scroll: Retained<NSScrollView>,
    status_bar: Retained<NSView>,
    btn_jobs_cancel: Retained<NSButton>,
    btn_jobs_retry: Retained<NSButton>,
    sep_tabs: Retained<NSView>,
    sep_nav: Retained<NSView>,
    sep_cmd: Retained<NSView>,
    sep_status: Retained<NSView>,
    sep_sidebar: Retained<NSView>,
    sep_preview: Retained<NSView>,
    sep_jobs: Retained<NSView>,
}

unsafe fn sf_symbol(name: &str, alt: &str) -> Option<Retained<NSImage>> {
    let name_str = NSString::from_str(name);
    let desc_str = NSString::from_str(alt);
    let img: Option<Retained<NSImage>> = msg_send![
        class!(NSImage),
        imageWithSystemSymbolName: &*name_str,
        accessibilityDescription: &*desc_str
    ];
    if let Some(ref image) = img {
        let _: () = msg_send![&**image, setTemplate: true];
    }
    img
}

unsafe fn create_icon_button(
    mtm: MainThreadMarker,
    target: &NSObject,
    action: Sel,
    symbol_name: &str,
    tooltip: &str,
) -> Retained<NSButton> {
    let btn: Retained<NSButton> = msg_send![mtm.alloc::<NSButton>(), init];
    let _: () = msg_send![&*btn, setButtonType: 0usize]; // NSButtonTypeMomentaryLight
    let _: () = msg_send![&*btn, setBezelStyle: 1usize]; // NSBezelStyleRounded
    let _: () = msg_send![&*btn, setBordered: false];
    let _: () = msg_send![&*btn, setTitle: &*NSString::from_str("")];
    let _: () = msg_send![&*btn, setImagePosition: 1usize]; // NSImageOnly
    let _: () = msg_send![&*btn, setTarget: Some(target)];
    let _: () = msg_send![&*btn, setAction: action];
    let _: () = msg_send![&*btn, setToolTip: &*NSString::from_str(tooltip)];

    if let Some(img) = sf_symbol(symbol_name, tooltip) {
        let _: () = msg_send![&*btn, setImage: Some(&*img)];
    }

    let cell: *mut AnyObject = msg_send![&*btn, cell];
    if !cell.is_null() {
        let _: () = msg_send![cell, setAccessibilityLabel: &*NSString::from_str(tooltip)];
    }
    btn
}

unsafe fn create_menu_button(
    mtm: MainThreadMarker,
    target: &NSObject,
    symbol_name: &str,
    title: &str,
    tooltip: &str,
    items: &[(&str, Sel)],
) -> Retained<NSPopUpButton> {
    let popup: Retained<NSPopUpButton> = msg_send![
        mtm.alloc::<NSPopUpButton>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(76.0, 30.0)),
        pullsDown: true
    ];
    let _: () = msg_send![&*popup, setBordered: false];
    let _: () = msg_send![&*popup, setToolTip: &*NSString::from_str(tooltip)];

    let menu: Retained<NSMenu> = msg_send![mtm.alloc::<NSMenu>(), init];
    let _: () = msg_send![&*menu, setAutoenablesItems: false];

    // Header item acting as the button face
    let header: Retained<NSMenuItem> = msg_send![
        mtm.alloc::<NSMenuItem>(),
        initWithTitle: &*NSString::from_str(title),
        action: None::<Sel>,
        keyEquivalent: &*NSString::from_str("")
    ];
    if let Some(img) = sf_symbol(symbol_name, tooltip) {
        let _: () = msg_send![&*header, setImage: Some(&*img)];
    }
    let _: () = msg_send![&*menu, addItem: &*header];

    for (item_title, sel) in items {
        let item: Retained<NSMenuItem> = msg_send![
            mtm.alloc::<NSMenuItem>(),
            initWithTitle: &*NSString::from_str(item_title),
            action: Some(*sel),
            keyEquivalent: &*NSString::from_str("")
        ];
        let _: () = msg_send![&*item, setTarget: Some(target)];
        let _: () = msg_send![&*item, setEnabled: true];
        let _: () = msg_send![&*menu, addItem: &*item];
    }

    let _: () = msg_send![&*popup, setMenu: Some(&*menu)];
    popup
}

unsafe fn create_separator(mtm: MainThreadMarker) -> Retained<NSView> {
    let view: Retained<NSView> = msg_send![
        mtm.alloc::<NSView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0))
    ];
    view.setWantsLayer(true);
    view
}

unsafe fn set_layer_bg(view: &NSView, color: &NSColor) {
    view.setWantsLayer(true);
    let layer: Option<Retained<CALayer>> = view.layer();
    if let Some(l) = layer {
        l.setBackgroundColor(Some(&color.CGColor()));
    }
}

/// # Safety
/// AppKit must be initialized on the main thread. Keep `target` alive while the
/// window exists; it must implement the action selectors wired below.
pub unsafe fn build(mtm: MainThreadMarker, target: &NSObject) -> ExplorerViews {
    let size = std::env::var("RUST_EXPLORER_WINDOW_SIZE")
        .ok()
        .and_then(|s| {
            s.split_once('x')
                .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        })
        .map(|(w, h)| NSSize::new(w.clamp(760, 3840) as f64, h.clamp(480, 2160) as f64))
        .unwrap_or(NSSize::new(1040., 680.));
    let initial_rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable;

    let window: Retained<super::widgets::ExplorerWindow> = msg_send![
        mtm.alloc::<super::widgets::ExplorerWindow>(),
        initWithContentRect: initial_rect,
        styleMask: style,
        backing: 2usize, // NSBackingStoreBuffered
        defer: false
    ];
    let window: Retained<NSWindow> = Retained::into_super(window);

    let _: () = msg_send![&*window, setReleasedWhenClosed: false];
    window.setTabbingMode(objc2_app_kit::NSWindowTabbingMode::Disallowed);
    NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
    let _: () = msg_send![&*window, setTitlebarAppearsTransparent: true];
    let _: () = msg_send![&*window, setTitleVisibility: 1isize]; // NSWindowTitleHidden
    let _: () = msg_send![&*window, setMinSize: NSSize::new(760.0, 480.0)];
    let _: () = msg_send![&*window, setContentSize: size];
    let _: () = msg_send![&*window, center];

    let root: Retained<NSView> = msg_send![mtm.alloc::<NSView>(), initWithFrame: initial_rect];
    root.setWantsLayer(true);
    root.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    window.setContentView(Some(&root));

    // 1. Tab Bar
    let tab_bar: Retained<NSView> = msg_send![
        mtm.alloc::<NSView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 640.0), NSSize::new(1040.0, 40.0))
    ];
    tab_bar.setWantsLayer(true);
    // Put interactive tabs in AppKit's titlebar accessory. The main content
    // stays below the native titlebar rather than drawing behind its hit area.
    let tab_accessory = NSTitlebarAccessoryViewController::new(mtm);
    tab_accessory.setLayoutAttribute(NSLayoutAttribute::Bottom);
    tab_bar.setFrame(NSRect::new(
        NSPoint::new(0., 0.),
        NSSize::new(initial_rect.size.width, 40.),
    ));
    tab_accessory.setView(&tab_bar);
    tab_accessory.setFullScreenMinHeight(40.);
    window.addTitlebarAccessoryViewController(&tab_accessory);

    let tabs: Retained<NSStackView> = msg_send![
        mtm.alloc::<NSStackView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 32.0))
    ];
    let _: () = msg_send![&*tabs, setOrientation: 0isize]; // Horizontal
    let _: () = msg_send![&*tabs, setSpacing: 4.0f64];
    let tab_scroll = NSScrollView::initWithFrame(
        mtm.alloc(),
        NSRect::new(NSPoint::new(80., 4.), NSSize::new(900., 32.)),
    );
    tab_scroll.setDrawsBackground(false);
    tab_scroll.setHasHorizontalScroller(false);
    tab_scroll.setAutohidesScrollers(true);
    tab_scroll.setScrollerStyle(objc2_app_kit::NSScrollerStyle::Overlay);
    tab_scroll.setDocumentView(Some(&tabs));
    tab_bar.addSubview(&tab_scroll);

    let new_tab_btn = create_icon_button(mtm, target, sel!(newTab:), "plus", "새 탭 (Cmd+T)");
    tab_bar.addSubview(&new_tab_btn);

    let sep_tabs = create_separator(mtm);
    tab_bar.addSubview(&sep_tabs);

    // 2. Navigation Bar
    let nav_bar: Retained<NSView> = msg_send![
        mtm.alloc::<NSView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 594.0), NSSize::new(1040.0, 46.0))
    ];
    nav_bar.setWantsLayer(true);
    root.addSubview(&nav_bar);

    let btn_back = create_icon_button(mtm, target, sel!(back:), "arrow.left", "뒤로 (Cmd+[)");
    let btn_forward =
        create_icon_button(mtm, target, sel!(forward:), "arrow.right", "앞으로 (Cmd+])");
    let btn_up = create_icon_button(mtm, target, sel!(up:), "arrow.up", "상위 폴더 (Cmd+Up)");
    let btn_refresh = create_icon_button(
        mtm,
        target,
        sel!(refresh:),
        "arrow.clockwise",
        "새로 고침 (Cmd+R)",
    );
    nav_bar.addSubview(&btn_back);
    nav_bar.addSubview(&btn_forward);
    nav_bar.addSubview(&btn_up);
    nav_bar.addSubview(&btn_refresh);

    let address: Retained<NSTextField> = msg_send![
        mtm.alloc::<NSTextField>(),
        initWithFrame: NSRect::new(NSPoint::new(154.0, 8.0), NSSize::new(580.0, 30.0))
    ];
    let _: () =
        msg_send![&*address, setPlaceholderString: &*NSString::from_str("경로를 입력하세요")];
    let _: () = msg_send![&*address, setTarget: Some(target)];
    let _: () = msg_send![&*address, setAction: sel!(addressEntered:)];
    super::widgets::single_line_editor(&address);
    address.setWantsLayer(true);
    if let Some(l) = address.layer() {
        let _: () = msg_send![&*l, setCornerRadius: 6.0f64];
    }
    nav_bar.addSubview(&address);
    address.setHidden(true);
    let breadcrumb: Retained<AddressPathControl> =
        msg_send![mtm.alloc::<AddressPathControl>(), initWithFrame:address.frame()];
    let breadcrumb = Retained::into_super(breadcrumb);
    breadcrumb.setTarget(Some(target));
    breadcrumb.setAction(Some(sel!(breadcrumbClicked:)));
    breadcrumb.setDoubleAction(Some(sel!(editAddress:)));
    breadcrumb.setToolTip(Some(&NSString::from_str(
        "경로 탐색 · ⌘L 또는 Ctrl+L로 직접 입력",
    )));
    nav_bar.addSubview(&breadcrumb);
    let btn_copy_address = create_icon_button(
        mtm,
        target,
        sel!(copyAddress:),
        "doc.on.doc",
        "현재 폴더 경로 복사",
    );
    nav_bar.addSubview(&btn_copy_address);

    let search_scope: Retained<NSPopUpButton> = msg_send![
        mtm.alloc::<NSPopUpButton>(),
        initWithFrame: NSRect::new(NSPoint::new(742.0, 8.0), NSSize::new(80.0, 30.0)),
        pullsDown: false
    ];
    let _: () = msg_send![&*search_scope, addItemWithTitle: &*NSString::from_str("현재 목록")];
    let _: () = msg_send![&*search_scope, addItemWithTitle: &*NSString::from_str("하위 폴더")];
    let _: () = msg_send![&*search_scope, setTarget: Some(target)];
    let _: () = msg_send![&*search_scope, setAction: sel!(searchScopeChanged:)];
    nav_bar.addSubview(&search_scope);

    let search: Retained<NSSearchField> = msg_send![
        mtm.alloc::<NSSearchField>(),
        initWithFrame: NSRect::new(NSPoint::new(828.0, 8.0), NSSize::new(200.0, 30.0))
    ];
    let _: () = msg_send![&*search, setSendsSearchStringImmediately: true];
    let _: () = msg_send![&*search, setPlaceholderString: &*NSString::from_str("검색")];
    let _: () = msg_send![&*search, setTarget: Some(target)];
    let _: () = msg_send![&*search, setAction: sel!(searchChanged:)];
    search.setWantsLayer(true);
    if let Some(l) = search.layer() {
        let _: () = msg_send![&*l, setCornerRadius: 6.0f64];
    }
    nav_bar.addSubview(&search);

    let sep_nav = create_separator(mtm);
    nav_bar.addSubview(&sep_nav);

    // 3. Command Bar
    let command_bar: Retained<NSView> = msg_send![
        mtm.alloc::<NSView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 548.0), NSSize::new(1040.0, 46.0))
    ];
    command_bar.setWantsLayer(true);
    root.addSubview(&command_bar);

    let btn_new = create_menu_button(
        mtm,
        target,
        "plus.circle",
        "새로 만들기",
        "현재 폴더에 새 항목 만들기",
        &[],
    );
    let new_menu = btn_new.menu().expect("creation menu");
    add_creation_items(
        &new_menu,
        Some(target),
        objc2_app_kit::NSEventModifierFlags::Command,
        mtm,
    );
    let btn_cut = create_icon_button(mtm, target, sel!(cutFiles:), "scissors", "잘라내기 (Cmd+X)");
    let btn_copy = create_icon_button(mtm, target, sel!(copyFiles:), "doc.on.doc", "복사 (Cmd+C)");
    let btn_paste = create_icon_button(
        mtm,
        target,
        sel!(pasteFiles:),
        "doc.on.clipboard",
        "붙여넣기 (Cmd+V)",
    );
    let btn_rename =
        create_icon_button(mtm, target, sel!(renameFile:), "pencil", "이름 바꾸기 (F2)");
    let btn_trash =
        create_icon_button(mtm, target, sel!(trashFiles:), "trash", "삭제 (Cmd+Delete)");

    command_bar.addSubview(&btn_new);
    command_bar.addSubview(&btn_cut);
    command_bar.addSubview(&btn_copy);
    command_bar.addSubview(&btn_paste);
    command_bar.addSubview(&btn_rename);
    command_bar.addSubview(&btn_trash);

    let btn_sort = create_menu_button(
        mtm,
        target,
        "arrow.up.arrow.down",
        "정렬",
        "항목 정렬",
        &[
            ("이름순 정렬", sel!(sortByName:)),
            ("수정한 날짜순 정렬", sel!(sortByDate:)),
            ("크기순 정렬", sel!(sortBySize:)),
        ],
    );
    command_bar.addSubview(&btn_sort);

    let btn_view_mode = create_menu_button(
        mtm,
        target,
        "square.grid.2x2",
        "보기",
        "보기 모드 설정",
        &[
            ("자세히", sel!(detailsView:)),
            ("작은 아이콘", sel!(smallIcons:)),
            ("중간 아이콘", sel!(mediumIcons:)),
            ("큰 아이콘", sel!(largeIcons:)),
            ("목록 간격 줄이기", sel!(toggleCompact:)),
            ("숨김 파일 표시", sel!(toggleHidden:)),
        ],
    );
    command_bar.addSubview(&btn_view_mode);

    let btn_toggle_preview = create_icon_button(
        mtm,
        target,
        sel!(togglePreview:),
        "sidebar.right",
        "미리보기 창 (Cmd+P)",
    );
    command_bar.addSubview(&btn_toggle_preview);

    let btn_more = create_menu_button(
        mtm,
        target,
        "ellipsis.circle",
        "더 보기",
        "추가 작업",
        &[
            ("새 창", sel!(newWindow:)),
            ("잘라내기", sel!(cutFiles:)),
            ("복사", sel!(copyFiles:)),
            ("붙여넣기", sel!(pasteFiles:)),
            ("이름 변경", sel!(renameFile:)),
            ("휴지통으로 이동", sel!(trashFiles:)),
            ("실행 취소", sel!(undoOperation:)),
            ("즐겨찾기에 추가", sel!(addFavorite:)),
            ("작업 진행 표시", sel!(toggleJobs:)),
            ("작업 기록…", sel!(showHistory:)),
            ("모든 작업 취소", sel!(cancelAllJobs:)),
        ],
    );
    command_bar.addSubview(&btn_more);
    let btn_settings =
        create_icon_button(mtm, target, sel!(settings:), "gearshape", "환경 설정 (⌘,)");
    btn_settings.setTitle(&NSString::from_str("환경 설정"));
    btn_settings.setImagePosition(objc2_app_kit::NSCellImagePosition::ImageLeading);
    btn_settings.setFont(Some(&NSFont::systemFontOfSize(13.)));
    command_bar.addSubview(&btn_settings);

    let sep_cmd = create_separator(mtm);
    command_bar.addSubview(&sep_cmd);

    // 4. Status Bar
    let status_bar: Retained<NSView> = msg_send![
        mtm.alloc::<NSView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1040.0, 26.0))
    ];
    status_bar.setWantsLayer(true);
    root.addSubview(&status_bar);

    let status: Retained<NSTextField> = msg_send![
        mtm.alloc::<NSTextField>(),
        initWithFrame: NSRect::new(NSPoint::new(12.0, 4.0), NSSize::new(1016.0, 18.0))
    ];
    let _: () = msg_send![&*status, setStringValue: &*NSString::from_str("0개 항목")];
    let _: () = msg_send![&*status, setEditable: false];
    let _: () = msg_send![&*status, setSelectable: false];
    let _: () = msg_send![&*status, setBezeled: false];
    let _: () = msg_send![&*status, setDrawsBackground: false];
    let font_meta: Option<Retained<NSFont>> = msg_send![class!(NSFont), systemFontOfSize: 12.0f64];
    if let Some(f) = font_meta.clone() {
        let _: () = msg_send![&*status, setFont: &*f];
    }
    status_bar.addSubview(&status);
    let icon_size: Retained<NSPopUpButton> = msg_send![mtm.alloc::<NSPopUpButton>(),initWithFrame:super::widgets::rect(0.,0.,118.,24.),pullsDown:false];
    for title in ["작은 아이콘", "중간 아이콘", "큰 아이콘"] {
        icon_size.addItemWithTitle(&NSString::from_str(title));
    }
    icon_size.setTarget(Some(target));
    icon_size.setAction(Some(sel!(iconSizeChanged:)));
    icon_size.setToolTip(Some(&NSString::from_str("아이콘 크기 · Ctrl/⌘ +, −")));
    icon_size.setHidden(true);
    status_bar.addSubview(&icon_size);
    let btn_status_undo = create_icon_button(
        mtm,
        target,
        sel!(undoNotice:),
        "arrow.uturn.backward",
        "방금 작업 되돌리기",
    );
    btn_status_undo.setTitle(&NSString::from_str("되돌리기"));
    btn_status_undo.setImagePosition(objc2_app_kit::NSCellImagePosition::ImageLeading);
    btn_status_undo.setHidden(true);
    status_bar.addSubview(&btn_status_undo);

    let sep_status = create_separator(mtm);
    status_bar.addSubview(&sep_status);

    // 5. Jobs Panel (Hidden initially)
    let jobs: Retained<NSView> = msg_send![
        mtm.alloc::<NSView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 26.0), NSSize::new(1040.0, 76.0))
    ];
    jobs.setWantsLayer(true);
    jobs.setHidden(true);
    root.addSubview(&jobs);

    let job_title: Retained<NSTextField> = msg_send![
        mtm.alloc::<NSTextField>(),
        initWithFrame: NSRect::new(NSPoint::new(16.0, 48.0), NSSize::new(860.0, 20.0))
    ];
    let _: () = msg_send![&*job_title, setStringValue: &*NSString::from_str("작업 진행 상태")];
    let _: () = msg_send![&*job_title, setEditable: false];
    let _: () = msg_send![&*job_title, setSelectable: false];
    let _: () = msg_send![&*job_title, setBezeled: false];
    let _: () = msg_send![&*job_title, setDrawsBackground: false];
    let font_title: Option<Retained<NSFont>> =
        msg_send![class!(NSFont), boldSystemFontOfSize: 13.0f64];
    if let Some(f) = font_title {
        let _: () = msg_send![&*job_title, setFont: &*f];
    }
    jobs.addSubview(&job_title);

    let job_detail: Retained<NSTextField> = msg_send![
        mtm.alloc::<NSTextField>(),
        initWithFrame: NSRect::new(NSPoint::new(16.0, 28.0), NSSize::new(860.0, 16.0))
    ];
    let _: () = msg_send![&*job_detail, setStringValue: &*NSString::from_str("대기 중…")];
    let _: () = msg_send![&*job_detail, setEditable: false];
    let _: () = msg_send![&*job_detail, setSelectable: false];
    let _: () = msg_send![&*job_detail, setBezeled: false];
    let _: () = msg_send![&*job_detail, setDrawsBackground: false];
    if let Some(f) = font_meta.clone() {
        let _: () = msg_send![&*job_detail, setFont: &*f];
    }
    jobs.addSubview(&job_detail);

    let job_progress: Retained<NSProgressIndicator> = msg_send![
        mtm.alloc::<NSProgressIndicator>(),
        initWithFrame: NSRect::new(NSPoint::new(16.0, 12.0), NSSize::new(860.0, 10.0))
    ];
    let _: () = msg_send![&*job_progress, setStyle: 0usize]; // NSProgressIndicatorStyleBar
    let _: () = msg_send![&*job_progress, setIndeterminate: false];
    let _: () = msg_send![&*job_progress, setMinValue: 0.0f64];
    let _: () = msg_send![&*job_progress, setMaxValue: 100.0f64];
    let _: () = msg_send![&*job_progress, setDoubleValue: 0.0f64];
    jobs.addSubview(&job_progress);

    let btn_jobs_cancel: Retained<NSButton> = msg_send![mtm.alloc::<NSButton>(), init];
    let _: () = msg_send![&*btn_jobs_cancel, setButtonType: 0usize];
    let _: () = msg_send![&*btn_jobs_cancel, setBezelStyle: 1usize];
    let _: () = msg_send![&*btn_jobs_cancel, setTitle: &*NSString::from_str("취소")];
    let _: () = msg_send![&*btn_jobs_cancel, setTarget: Some(target)];
    let _: () = msg_send![&*btn_jobs_cancel, setAction: sel!(cancelJob:)];
    jobs.addSubview(&btn_jobs_cancel);

    let btn_jobs_retry: Retained<NSButton> = msg_send![mtm.alloc::<NSButton>(), init];
    let _: () = msg_send![&*btn_jobs_retry, setButtonType: 0usize];
    let _: () = msg_send![&*btn_jobs_retry, setBezelStyle: 1usize];
    let _: () = msg_send![&*btn_jobs_retry, setTitle: &*NSString::from_str("다시 시도")];
    let _: () = msg_send![&*btn_jobs_retry, setTarget: Some(target)];
    let _: () = msg_send![&*btn_jobs_retry, setAction: sel!(retryJob:)];
    jobs.addSubview(&btn_jobs_retry);

    let sep_jobs = create_separator(mtm);
    jobs.addSubview(&sep_jobs);

    // 6. Sidebar (OutlineView inside ScrollView)
    let sidebar_rect = NSRect::new(NSPoint::new(0.0, 26.0), NSSize::new(220.0, 522.0));
    let sidebar_scroll: Retained<NSScrollView> = msg_send![
        mtm.alloc::<NSScrollView>(),
        initWithFrame: sidebar_rect
    ];
    let _: () = msg_send![&*sidebar_scroll, setHasVerticalScroller: true];
    let _: () = msg_send![&*sidebar_scroll, setHasHorizontalScroller: false];
    let _: () = msg_send![&*sidebar_scroll, setAutohidesScrollers: true];
    let _: () = msg_send![&*sidebar_scroll, setDrawsBackground: false];

    let sidebar: Retained<NSOutlineView> = msg_send![
        mtm.alloc::<NSOutlineView>(),
        initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), sidebar_rect.size)
    ];
    let col_name_str = NSString::from_str("name");
    let col_side: Retained<NSTableColumn> = msg_send![
        mtm.alloc::<NSTableColumn>(),
        initWithIdentifier: &*col_name_str
    ];
    let _: () = msg_send![&*col_side, setResizingMask: 1usize]; // NSTableColumnAutoresizingMask
    let _: () = msg_send![&*sidebar, addTableColumn: &*col_side];
    let _: () = msg_send![&*sidebar, setOutlineTableColumn: &*col_side];
    let _: () = msg_send![&*sidebar, setHeaderView: std::ptr::null::<NSView>()];
    let _: () = msg_send![&*sidebar, setRowHeight: 32.0f64];
    let _: () = msg_send![&*sidebar, setAllowsMultipleSelection: false];
    let _: () = msg_send![&*sidebar, setTarget: Some(target)];
    let _: () = msg_send![&*sidebar, setAction: sel!(openSidebar:)];
    let _: () = msg_send![&*sidebar, setDoubleAction: sel!(openSidebar:)];
    let _: () = msg_send![&*sidebar_scroll, setDocumentView: Some(&*sidebar)];
    root.addSubview(&sidebar_scroll);

    let sep_sidebar = create_separator(mtm);
    root.addSubview(&sep_sidebar);

    // 7. Preview Panel (Hidden initially)
    let preview: Retained<NSView> = msg_send![
        mtm.alloc::<NSView>(),
        initWithFrame: NSRect::new(NSPoint::new(780.0, 26.0), NSSize::new(260.0, 522.0))
    ];
    preview.setWantsLayer(true);
    preview.setHidden(true);

    let preview_image: Retained<NSImageView> = msg_send![
        mtm.alloc::<NSImageView>(),
        initWithFrame: NSRect::new(NSPoint::new(12.0, 350.0), NSSize::new(236.0, 160.0))
    ];
    let _: () = msg_send![&*preview_image, setImageScaling: 2usize]; // ProportionallyUpOrDown
    let _: () = msg_send![&*preview_image, setEditable: false];
    preview.addSubview(&preview_image);

    let preview_title: Retained<NSTextField> = msg_send![
        mtm.alloc::<NSTextField>(),
        initWithFrame: NSRect::new(NSPoint::new(12.0, 318.0), NSSize::new(236.0, 24.0))
    ];
    let _: () = msg_send![&*preview_title, setStringValue: &*NSString::from_str("선택 항목 없음")];
    let _: () = msg_send![&*preview_title, setEditable: false];
    let _: () = msg_send![&*preview_title, setSelectable: true];
    let _: () = msg_send![&*preview_title, setBezeled: false];
    let _: () = msg_send![&*preview_title, setDrawsBackground: false];
    let font_bold14: Option<Retained<NSFont>> =
        msg_send![class!(NSFont), boldSystemFontOfSize: 14.0f64];
    if let Some(f) = font_bold14 {
        let _: () = msg_send![&*preview_title, setFont: &*f];
    }
    preview.addSubview(&preview_title);

    let preview_details: Retained<NSTextField> = msg_send![
        mtm.alloc::<NSTextField>(),
        initWithFrame: NSRect::new(NSPoint::new(12.0, 12.0), NSSize::new(236.0, 298.0))
    ];
    let _: () = msg_send![
        &*preview_details,
        setStringValue: &*NSString::from_str("파일을 선택하면 세부 정보가 여기에 표시됩니다.")
    ];
    let _: () = msg_send![&*preview_details, setEditable: false];
    let _: () = msg_send![&*preview_details, setSelectable: true];
    let _: () = msg_send![&*preview_details, setBezeled: false];
    let _: () = msg_send![&*preview_details, setDrawsBackground: false];
    let cell_details: *mut AnyObject = msg_send![&*preview_details, cell];
    if !cell_details.is_null() {
        let _: () = msg_send![cell_details, setWraps: true];
    }
    if let Some(f) = font_meta.clone() {
        let _: () = msg_send![&*preview_details, setFont: &*f];
    }
    preview.addSubview(&preview_details);
    root.addSubview(&preview);

    let sep_preview = create_separator(mtm);
    sep_preview.setHidden(true);
    root.addSubview(&sep_preview);

    // 8. Main Content: Table (List) and Collection (Icon)
    let content_rect = NSRect::new(NSPoint::new(221.0, 26.0), NSSize::new(819.0, 522.0));

    let list_scroll: Retained<NSScrollView> = msg_send![
        mtm.alloc::<NSScrollView>(),
        initWithFrame: content_rect
    ];
    let _: () = msg_send![&*list_scroll, setHasVerticalScroller: true];
    let _: () = msg_send![&*list_scroll, setHasHorizontalScroller: true];
    let _: () = msg_send![&*list_scroll, setAutohidesScrollers: true];

    let list_clip: Retained<super::selection::FileClipView> =
        msg_send![mtm.alloc::<super::selection::FileClipView>(), initWithFrame: content_rect];
    list_scroll.setContentView(&list_clip);
    let table = FileTableView::new(mtm, NSRect::new(NSPoint::new(0., 0.), content_rect.size));
    let table = Retained::into_super(table);
    table.setStyle(objc2_app_kit::NSTableViewStyle::Plain);
    table.setIntercellSpacing(NSSize::new(3., 0.));
    table.setColumnAutoresizingStyle(
        objc2_app_kit::NSTableViewColumnAutoresizingStyle::NoColumnAutoresizing,
    );
    let table_cols = [
        ("name", "이름", 320.0f64),
        ("date", "수정한 날짜", 150.0f64),
        ("kind", "종류", 110.0f64),
        ("size", "크기", 90.0f64),
    ];
    for (id, title, w) in table_cols {
        let col_id = NSString::from_str(id);
        let col: Retained<NSTableColumn> = msg_send![
            mtm.alloc::<NSTableColumn>(),
            initWithIdentifier: &*col_id
        ];
        let col_title = NSString::from_str(title);
        let _: () = msg_send![&*col, setTitle: &*col_title];
        if id == "name" {
            col.setMinWidth(260.);
        }
        let _: () = msg_send![&*col, setWidth: w];
        let _: () = msg_send![&*table, addTableColumn: &*col];
    }
    let _: () = msg_send![&*table, setRowHeight: 30.0f64];
    let _: () = msg_send![&*table, setAllowsMultipleSelection: true];
    let _: () = msg_send![&*table, setUsesAlternatingRowBackgroundColors: false];
    let _: () = msg_send![&*table, setGridStyleMask: 0usize];
    let _: () = msg_send![&*table, setTarget: Some(target)];
    let _: () = msg_send![&*table, setDoubleAction: sel!(openSelected:)];
    let _: () = msg_send![&*list_scroll, setDocumentView: Some(&*table)];
    root.addSubview(&list_scroll);

    let icon_scroll: Retained<NSScrollView> = msg_send![
        mtm.alloc::<NSScrollView>(),
        initWithFrame: content_rect
    ];
    let _: () = msg_send![&*icon_scroll, setHasVerticalScroller: true];
    let _: () = msg_send![&*icon_scroll, setAutohidesScrollers: true];
    let _: () = msg_send![&*icon_scroll, setHidden: true];

    let flow_layout: Retained<NSCollectionViewFlowLayout> =
        msg_send![mtm.alloc::<NSCollectionViewFlowLayout>(), init];
    let (icon_w, icon_h, _) = IconSize::Small.dimensions();
    let _: () = msg_send![&*flow_layout, setItemSize: NSSize::new(icon_w, icon_h)];
    let _: () = msg_send![&*flow_layout, setMinimumInteritemSpacing: 12.0f64];
    let _: () = msg_send![&*flow_layout, setMinimumLineSpacing: 12.0f64];

    let icon_clip: Retained<super::selection::FileClipView> =
        msg_send![mtm.alloc::<super::selection::FileClipView>(), initWithFrame: content_rect];
    icon_scroll.setContentView(&icon_clip);
    let collection =
        FileCollectionView::new(mtm, NSRect::new(NSPoint::new(0., 0.), content_rect.size));
    let collection = Retained::into_super(collection);
    let _: () = msg_send![&*collection, setCollectionViewLayout: Some(&*flow_layout)];
    collection.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);
    let _: () = msg_send![&*collection, setSelectable: true];
    let _: () = msg_send![&*collection, setAllowsMultipleSelection: true];
    let _: () = msg_send![&*icon_scroll, setDocumentView: Some(&*collection)];
    root.addSubview(&icon_scroll);

    let empty_label: Retained<NSTextField> = msg_send![
        mtm.alloc::<NSTextField>(),
        initWithFrame: NSRect::new(NSPoint::new(480.0, 270.0), NSSize::new(300.0, 32.0))
    ];
    let _: () = msg_send![&*empty_label, setStringValue: &*NSString::from_str("폴더를 읽는 중…")];
    let _: () = msg_send![&*empty_label, setEditable: false];
    let _: () = msg_send![&*empty_label, setSelectable: false];
    let _: () = msg_send![&*empty_label, setBezeled: false];
    let _: () = msg_send![&*empty_label, setDrawsBackground: false];
    empty_label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
    let font_body: Option<Retained<NSFont>> = msg_send![class!(NSFont), systemFontOfSize: 13.0f64];
    if let Some(f) = font_body {
        let _: () = msg_send![&*empty_label, setFont: &*f];
    }
    empty_label.setMaximumNumberOfLines(4);
    empty_label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByWordWrapping);
    root.addSubview(&empty_label);

    let access_banner = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    let access_intro_label = NSTextField::wrappingLabelWithString(
        &NSString::from_str(
            "폴더 접근을 한 번에 설정하세요\n자주 쓰는 폴더를 선택하면 다음 실행에도 위치를 기억합니다.",
        ),
        mtm,
    );
    access_intro_label.setSelectable(false);
    access_intro_label.setFont(Some(&NSFont::systemFontOfSize(12.)));
    let btn_access_intro = NSButton::buttonWithTitle_target_action(
        &NSString::from_str("폴더 접근 설정…"),
        Some(target),
        Some(sel!(showFolderAccess:)),
        mtm,
    );
    let btn_access_dismiss = NSButton::buttonWithTitle_target_action(
        &NSString::from_str("×"),
        Some(target),
        Some(sel!(dismissAccessIntro:)),
        mtm,
    );
    btn_access_dismiss.setToolTip(Some(&NSString::from_str("이 안내 닫기")));
    btn_access_dismiss.setAccessibilityLabel(Some(&NSString::from_str("폴더 접근 안내 닫기")));
    access_banner.addSubview(&access_intro_label);
    access_banner.addSubview(&btn_access_intro);
    access_banner.addSubview(&btn_access_dismiss);
    root.addSubview(&access_banner);
    let empty_actions = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    let action = |title: &str, selector| {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(selector),
            mtm,
        )
    };
    let btn_access_current = action("폴더 선택…", sel!(chooseCurrentAccess:));
    let btn_access_help = action("접근 설정…", sel!(showFolderAccess:));
    let btn_error_retry = action("다시 시도", sel!(retryFolderAccess:));
    let btn_error_details = action("오류 자세히…", sel!(folderErrorDetails:));
    for button in [
        &btn_access_current,
        &btn_access_help,
        &btn_error_retry,
        &btn_error_details,
    ] {
        empty_actions.addSubview(button);
    }
    empty_actions.setHidden(true);
    root.addSubview(&empty_actions);

    let views = ExplorerViews {
        last_columns_width: std::cell::Cell::new(0.),
        palette: std::cell::Cell::default(),
        theme_choice: std::cell::Cell::default(),
        window,
        root,
        tabs,
        address,
        breadcrumb,
        search,
        search_scope,
        sidebar,
        table,
        collection,
        list_scroll,
        icon_scroll,
        preview,
        preview_title,
        preview_details,
        preview_image,
        status,
        empty_label,
        access_banner,
        access_intro_label,
        btn_access_intro,
        btn_access_dismiss,
        empty_actions,
        btn_access_current,
        btn_access_help,
        btn_error_retry,
        btn_error_details,
        jobs,
        job_title,
        job_detail,
        job_progress,
        icon_size,
        flow_layout,
        btn_copy_address,
        btn_settings,
        btn_status_undo,
        tab_bar,
        tab_scroll,
        new_tab_btn,
        nav_bar,
        btn_back,
        btn_forward,
        btn_up,
        btn_refresh,
        command_bar,
        btn_new,
        btn_cut,
        btn_copy,
        btn_paste,
        btn_rename,
        btn_trash,
        btn_sort,
        btn_view_mode,
        btn_toggle_preview,
        btn_more,
        sidebar_scroll,
        status_bar,
        btn_jobs_cancel,
        btn_jobs_retry,
        sep_tabs,
        sep_nav,
        sep_cmd,
        sep_status,
        sep_sidebar,
        sep_preview,
        sep_jobs,
    };

    views.window.setFrame_display(
        NSRect::new(views.window.frame().origin, initial_rect.size),
        false,
    );
    views.window.center();
    views.layout();
    views.apply_appearance();
    views
}

impl ExplorerViews {
    /// # Safety
    /// Call on the AppKit main thread while the window and its views are alive.
    pub unsafe fn layout(&self) {
        let bounds = self.root.bounds();
        let w = bounds.size.width;
        let h = bounds.size.height;

        // 1. Tabs live below the native titlebar; AppKit owns their hit testing.
        let tab_h = 40.0f64;
        self.tab_bar.setFrameSize(NSSize::new(w, tab_h));
        let tab_y = h;
        let new_tab_size = 28.0f64;
        let tabs_w = (w - 12.0 - new_tab_size - 16.0).max(40.0);
        self.tab_scroll.setFrame(NSRect::new(
            NSPoint::new(12.0, 4.0),
            NSSize::new(tabs_w, 32.0),
        ));
        let content_width = (self
            .tabs
            .arrangedSubviews()
            .iter()
            .map(|v| v.frame().size.width + 4.)
            .sum::<f64>()
            - 4.)
            .max(tabs_w);
        self.tabs.setFrame(NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(content_width, 32.),
        ));
        self.new_tab_btn.setFrame(NSRect::new(
            NSPoint::new(12.0 + tabs_w + 6.0, 6.0),
            NSSize::new(new_tab_size, new_tab_size),
        ));
        self.sep_tabs
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, 1.0)));

        // 2. Navigation bar (height 46pt)
        let nav_h = 46.0f64;
        let nav_y = tab_y - nav_h;
        self.nav_bar
            .setFrame(NSRect::new(NSPoint::new(0.0, nav_y), NSSize::new(w, nav_h)));
        self.btn_back.setFrame(NSRect::new(
            NSPoint::new(12.0, 8.0),
            NSSize::new(30.0, 30.0),
        ));
        self.btn_forward.setFrame(NSRect::new(
            NSPoint::new(46.0, 8.0),
            NSSize::new(30.0, 30.0),
        ));
        self.btn_up.setFrame(NSRect::new(
            NSPoint::new(80.0, 8.0),
            NSSize::new(30.0, 30.0),
        ));
        self.btn_refresh.setFrame(NSRect::new(
            NSPoint::new(114.0, 8.0),
            NSSize::new(30.0, 30.0),
        ));

        let search_w = if w < 840.0 { 160.0f64 } else { 200.0f64 };
        let scope_w = 98.0f64;
        let search_x = w - 12.0 - search_w;
        let scope_x = search_x - 8.0 - scope_w;
        let addr_x = 152.0f64;
        let addr_w = (scope_x - 44.0 - addr_x).max(100.0);

        self.address.setFrame(NSRect::new(
            NSPoint::new(addr_x, 8.0),
            NSSize::new(addr_w, 30.0),
        ));
        self.breadcrumb.setFrame(self.address.frame());
        self.btn_copy_address
            .setFrame(super::widgets::rect(scope_x - 40., 8., 30., 30.));
        self.search_scope.setFrame(NSRect::new(
            NSPoint::new(scope_x, 8.0),
            NSSize::new(scope_w, 30.0),
        ));
        self.search.setFrame(NSRect::new(
            NSPoint::new(search_x, 8.0),
            NSSize::new(search_w, 30.0),
        ));
        self.sep_nav
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, 1.0)));

        // 3. Command bar (height 46pt)
        let cmd_h = 46.0f64;
        let cmd_y = nav_y - cmd_h;
        self.command_bar
            .setFrame(NSRect::new(NSPoint::new(0.0, cmd_y), NSSize::new(w, cmd_h)));

        self.btn_new.setFrame(NSRect::new(
            NSPoint::new(12.0, 8.0),
            NSSize::new(103.0, 30.0),
        ));

        let is_compact_width = w < 860.0;
        self.btn_cut.setHidden(is_compact_width);
        self.btn_copy.setHidden(is_compact_width);
        self.btn_paste.setHidden(is_compact_width);
        self.btn_rename.setHidden(is_compact_width);
        self.btn_trash.setHidden(is_compact_width);

        let next_x = if is_compact_width {
            116.0f64
        } else {
            self.btn_cut.setFrame(NSRect::new(
                NSPoint::new(116.0, 8.0),
                NSSize::new(32.0, 30.0),
            ));
            self.btn_copy.setFrame(NSRect::new(
                NSPoint::new(150.0, 8.0),
                NSSize::new(32.0, 30.0),
            ));
            self.btn_paste.setFrame(NSRect::new(
                NSPoint::new(184.0, 8.0),
                NSSize::new(32.0, 30.0),
            ));
            self.btn_rename.setFrame(NSRect::new(
                NSPoint::new(218.0, 8.0),
                NSSize::new(32.0, 30.0),
            ));
            self.btn_trash.setFrame(NSRect::new(
                NSPoint::new(252.0, 8.0),
                NSSize::new(32.0, 30.0),
            ));
            292.0f64
        };

        self.btn_sort.setFrame(NSRect::new(
            NSPoint::new(next_x, 8.0),
            NSSize::new(76.0, 30.0),
        ));
        self.btn_view_mode.setFrame(NSRect::new(
            NSPoint::new(next_x + 80.0, 8.0),
            NSSize::new(76.0, 30.0),
        ));

        self.btn_settings
            .setFrame(super::widgets::rect(w - 108., 8., 96., 30.));
        let more_x = w - 116.0 - 84.0;
        let prev_toggle_x = more_x - 8.0 - 32.0;
        self.btn_more.setFrame(NSRect::new(
            NSPoint::new(more_x, 8.0),
            NSSize::new(84.0, 30.0),
        ));
        self.btn_toggle_preview.setFrame(NSRect::new(
            NSPoint::new(prev_toggle_x, 8.0),
            NSSize::new(32.0, 30.0),
        ));
        self.sep_cmd
            .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, 1.0)));

        // 4. Status bar (bottom 26pt)
        let status_h = 26.0f64;
        self.status_bar.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(w, status_h),
        ));
        self.status.setFrame(NSRect::new(
            NSPoint::new(12.0, 4.0),
            NSSize::new(
                w - 24.0
                    - if self.icon_size.isHidden() { 0. } else { 128. }
                    - if self.btn_status_undo.isHidden() {
                        0.
                    } else {
                        96.
                    },
                18.0,
            ),
        ));
        self.icon_size
            .setFrame(super::widgets::rect(w - 126., 1., 118., 24.));
        self.btn_status_undo.setFrame(super::widgets::rect(
            w - 104. - if self.icon_size.isHidden() { 0. } else { 128. },
            1.,
            96.,
            24.,
        ));
        self.sep_status.setFrame(NSRect::new(
            NSPoint::new(0.0, status_h - 1.0),
            NSSize::new(w, 1.0),
        ));

        // 5. Jobs drawer (height 76pt when shown)
        let jobs_visible = !self.jobs.isHidden();
        let jobs_h = if jobs_visible { 76.0f64 } else { 0.0f64 };
        let jobs_y = status_h;

        if jobs_visible {
            self.jobs.setFrame(NSRect::new(
                NSPoint::new(0.0, jobs_y),
                NSSize::new(w, jobs_h),
            ));
            let btn_w = 72.0f64;
            let text_w = (w - 180.0).max(120.0);
            self.job_title.setFrame(NSRect::new(
                NSPoint::new(16.0, 48.0),
                NSSize::new(text_w, 20.0),
            ));
            self.job_detail.setFrame(NSRect::new(
                NSPoint::new(16.0, 28.0),
                NSSize::new(text_w, 16.0),
            ));
            self.job_progress.setFrame(NSRect::new(
                NSPoint::new(16.0, 12.0),
                NSSize::new(text_w, 10.0),
            ));
            self.btn_jobs_cancel.setFrame(NSRect::new(
                NSPoint::new(w - 160.0, 24.0),
                NSSize::new(btn_w, 28.0),
            ));
            self.btn_jobs_retry.setFrame(NSRect::new(
                NSPoint::new(w - 82.0, 24.0),
                NSSize::new(btn_w, 28.0),
            ));
            self.sep_jobs.setFrame(NSRect::new(
                NSPoint::new(0.0, jobs_h - 1.0),
                NSSize::new(w, 1.0),
            ));
        }

        // 6. Central workspace (Sidebar + List/Icon View + Preview)
        let content_y = status_h + jobs_h;
        let workspace_h = (cmd_y - content_y).max(100.0);
        let intro_h = if self.access_banner.isHidden() {
            0.
        } else {
            56.
        };
        let content_h = (workspace_h - intro_h).max(1.);
        self.access_banner
            .setFrame(super::widgets::rect(0., content_y + content_h, w, intro_h));
        self.access_intro_label
            .setFrame(super::widgets::rect(16., 9., w - 220., 40.));
        self.btn_access_intro
            .setFrame(super::widgets::rect(w - 192., 14., 140., 28.));
        self.btn_access_dismiss
            .setFrame(super::widgets::rect(w - 44., 14., 28., 28.));

        let sidebar_w = if w < 860.0 { 190.0f64 } else { 220.0f64 };
        self.sidebar_scroll.setFrame(NSRect::new(
            NSPoint::new(0.0, content_y),
            NSSize::new(sidebar_w, content_h),
        ));
        if let Some(column) = self.sidebar.tableColumns().firstObject() {
            column.setWidth(sidebar_w - 28.0);
        }
        self.sep_sidebar.setFrame(NSRect::new(
            NSPoint::new(sidebar_w, content_y),
            NSSize::new(1.0, content_h),
        ));

        let preview_visible = !self.preview.isHidden();
        let preview_w = if preview_visible { 260.0f64 } else { 0.0f64 };

        if preview_visible {
            let prev_x = w - preview_w;
            self.preview.setFrame(NSRect::new(
                NSPoint::new(prev_x, content_y),
                NSSize::new(preview_w, content_h),
            ));
            self.sep_preview.setHidden(false);
            self.sep_preview.setFrame(NSRect::new(
                NSPoint::new(prev_x - 1.0, content_y),
                NSSize::new(1.0, content_h),
            ));

            let img_h = (content_h - 128.0).clamp(72.0, 160.0);
            let img_y = (content_h - 12.0 - img_h).max(10.0);
            self.preview_image.setFrame(NSRect::new(
                NSPoint::new(12.0, img_y),
                NSSize::new(preview_w - 24.0, img_h),
            ));
            let title_y = (img_y - 8.0 - 24.0).max(10.0);
            self.preview_title.setFrame(NSRect::new(
                NSPoint::new(12.0, title_y),
                NSSize::new(preview_w - 24.0, 24.0),
            ));
            let details_h = (title_y - 16.0).max(40.0);
            self.preview_details.setFrame(NSRect::new(
                NSPoint::new(12.0, 12.0),
                NSSize::new(preview_w - 24.0, details_h),
            ));
        } else {
            self.sep_preview.setHidden(true);
        }

        let main_x = sidebar_w + 1.0;
        let main_w =
            (w - main_x - preview_w - (if preview_visible { 1.0 } else { 0.0 })).max(100.0);

        self.list_scroll.setFrame(NSRect::new(
            NSPoint::new(main_x, content_y),
            NSSize::new(main_w, content_h),
        ));
        self.icon_scroll.setFrame(NSRect::new(
            NSPoint::new(main_x, content_y),
            NSSize::new(main_w, content_h),
        ));

        self.fit_columns(main_w);

        let empty_w = (main_w - 32.0).min(480.0);
        let buttons: Vec<_> = [
            &self.btn_access_current,
            &self.btn_access_help,
            &self.btn_error_retry,
            &self.btn_error_details,
        ]
        .into_iter()
        .filter(|b| !b.isHidden())
        .collect();
        let per_row = if empty_w >= 440. { 4 } else { 2 };
        let rows = buttons.len().div_ceil(per_row);
        let actions_h = if self.empty_actions.isHidden() {
            0.
        } else {
            rows as f64 * 36.
        };
        let empty_h = (content_h - actions_h - 20.).clamp(32., 100.);
        let actions_y = content_y + ((content_h - empty_h - actions_h) / 2.).max(0.);
        self.empty_label.setFrame(NSRect::new(
            NSPoint::new(main_x + (main_w - empty_w) / 2.0, actions_y + actions_h),
            NSSize::new(empty_w, empty_h),
        ));
        self.empty_actions.setFrame(super::widgets::rect(
            main_x + (main_w - empty_w) / 2.,
            actions_y,
            empty_w,
            actions_h,
        ));
        let cols = per_row.min(buttons.len()).max(1);
        let button_w = (empty_w - (cols - 1) as f64 * 8.) / cols as f64;
        for (index, button) in buttons.iter().enumerate() {
            button.setFrame(super::widgets::rect(
                (index % cols) as f64 * (button_w + 8.),
                actions_h - 32. - (index / cols) as f64 * 36.,
                button_w,
                28.,
            ));
        }
    }

    /// # Safety
    /// Call on the AppKit main thread while the window and its views are alive.
    unsafe fn fit_columns(&self, width: f64) {
        if (self.last_columns_width.get() - width).abs() < 0.5 {
            return;
        }
        self.last_columns_width.set(width);
        // Preserve the full filename column; reveal metadata as space permits.
        // Hidden metadata remains available in the preview/information panel.
        let date = width >= 440.;
        let kind = width >= 670.;
        let size = width >= 790.;
        let name_width = (width
            - 12.
            - if date { 168. } else { 0. }
            - if kind { 124. } else { 0. }
            - if size { 92. } else { 0. })
        .max(260.);
        for col in self.table.tableColumns() {
            let (show, w) = match col.identifier().to_string().as_str() {
                "date" => (date, 168.),
                "kind" => (kind, 124.),
                "size" => (size, 92.),
                _ => (true, name_width),
            };
            col.setHidden(!show);
            col.setWidth(w);
        }
    }
    /// # Safety
    /// Call on the AppKit main thread while the window and controls are alive.
    pub unsafe fn apply_theme(&self, id: crate::core::theme::ThemeId) {
        self.theme_choice.set(id);
        let appearance = if id.index() == 0 {
            None
        } else {
            NSAppearance::appearanceNamed(if id.resolve(false).spec.dark {
                objc2_app_kit::NSAppearanceNameDarkAqua
            } else {
                objc2_app_kit::NSAppearanceNameAqua
            })
        };
        self.window.setAppearance(appearance.as_deref());
        self.apply_appearance();
    }
    /// # Safety
    /// Call on the AppKit main thread while the window and controls are alive.
    pub unsafe fn apply_appearance(&self) {
        use super::theme::color;
        use crate::core::theme::Layout;
        let names = NSArray::from_slice(&[
            objc2_app_kit::NSAppearanceNameDarkAqua,
            objc2_app_kit::NSAppearanceNameAqua,
        ]);
        let dark = self
            .window
            .effectiveAppearance()
            .bestMatchFromAppearancesWithNames(&names)
            .is_some_and(|s| s.to_string().contains("Dark"));
        let p = self.theme_choice.get().resolve(dark);
        self.palette.set(p);
        self.window.setBackgroundColor(Some(&color(p.shell)));
        for view in [&self.root, &self.nav_bar, &self.status_bar] {
            set_layer_bg(view, &color(p.shell));
        }
        set_layer_bg(
            &self.tab_bar,
            &color(if p.spec.layout == Layout::Header {
                p.accent
            } else {
                p.shell
            }),
        );
        set_layer_bg(&self.command_bar, &color(p.canvas));
        self.sidebar.setAppearance(
            if p.spec.layout == Layout::Rail {
                NSAppearance::appearanceNamed(objc2_app_kit::NSAppearanceNameDarkAqua)
            } else {
                None
            }
            .as_deref(),
        );
        self.sidebar.setBackgroundColor(&color(p.rail));
        self.sidebar_scroll.setBackgroundColor(&color(p.rail));
        self.sidebar_scroll
            .contentView()
            .setBackgroundColor(&color(p.rail));
        self.sidebar_scroll.setWantsLayer(true);
        if let Some(layer) = self.sidebar_scroll.layer() {
            layer.setCornerRadius(if p.spec.layout == Layout::Air {
                10.
            } else {
                0.
            });
            layer.setMasksToBounds(true);
        }
        self.table.setBackgroundColor(&color(p.canvas));
        self.table.setUsesAlternatingRowBackgroundColors(false);
        self.collection
            .setBackgroundColors(Some(&NSArray::from_retained_slice(&[color(p.canvas)])));
        for scroll in [&self.list_scroll, &self.icon_scroll] {
            scroll.setBackgroundColor(&color(p.canvas));
            scroll.contentView().setBackgroundColor(&color(p.canvas));
        }
        for view in [&self.preview, &self.jobs, &self.access_banner] {
            set_layer_bg(view, &color(p.canvas));
        }
        for view in [
            &self.sep_tabs,
            &self.sep_nav,
            &self.sep_cmd,
            &self.sep_status,
            &self.sep_sidebar,
            &self.sep_preview,
            &self.sep_jobs,
        ] {
            set_layer_bg(view, &color(p.line));
        }
        for label in [
            &self.status,
            &self.empty_label,
            &self.preview_details,
            &self.job_detail,
        ] {
            label.setTextColor(Some(&color(p.muted)));
        }
        for label in [
            &self.preview_title,
            &self.job_title,
            &self.access_intro_label,
        ] {
            label.setTextColor(Some(&color(p.ink)));
        }
        self.address.setTextColor(Some(&color(p.ink)));
        self.address.setBackgroundColor(Some(&color(p.canvas)));
        self.search.setTextColor(Some(&color(p.ink)));
        self.search.setBackgroundColor(Some(&color(p.canvas)));
        for button in [
            &self.btn_back,
            &self.btn_forward,
            &self.btn_up,
            &self.btn_refresh,
            &self.btn_copy_address,
            &self.btn_cut,
            &self.btn_copy,
            &self.btn_paste,
            &self.btn_rename,
            &self.btn_trash,
            &self.btn_toggle_preview,
            &self.btn_settings,
        ] {
            button.setContentTintColor(Some(&color(p.ink)));
        }
        self.new_tab_btn
            .setContentTintColor(Some(&color(if p.spec.layout == Layout::Header {
                p.on_accent
            } else {
                p.ink
            })));
    }
}

impl ExplorerViews {
    /// # Safety
    /// Call on the AppKit main thread while this window and its controls are alive.
    pub unsafe fn set_error_actions(&self, error: bool, denied: bool) {
        let changed =
            self.empty_actions.isHidden() == error || self.btn_access_current.isHidden() == denied;
        self.empty_actions.setHidden(!error);
        self.btn_access_current.setHidden(!denied);
        self.btn_access_help.setHidden(!denied);
        if changed {
            self.layout();
        }
    }
    pub fn set_actions_enabled(
        &self,
        can_write: bool,
        selected: usize,
        can_paste: bool,
        back: bool,
        forward: bool,
        up: bool,
    ) {
        self.btn_new.setEnabled(can_write);
        self.btn_paste.setEnabled(can_write && can_paste);
        self.btn_copy.setEnabled(selected > 0);
        self.btn_cut.setEnabled(can_write && selected > 0);
        self.btn_rename.setEnabled(can_write && selected == 1);
        self.btn_trash.setEnabled(can_write && selected > 0);
        self.btn_back.setEnabled(back);
        self.btn_forward.setEnabled(forward);
        self.btn_up.setEnabled(up);
    }
}

impl ExplorerViews {
    /// # Safety
    /// Call on the AppKit main thread while these views are alive.
    pub unsafe fn set_icon_size(&self, mode: ViewMode, size: IconSize) {
        let (w, h, _) = size.dimensions();
        self.flow_layout.setItemSize(NSSize::new(w, h));
        self.flow_layout.invalidateLayout();
        self.icon_size.selectItemAtIndex(size.index());
        self.icon_size.setHidden(mode != ViewMode::Icons);
    }
    pub fn show_notice_undo(&self, shown: bool) {
        self.btn_status_undo.setHidden(!shown);
        unsafe {
            self.layout();
        }
    }
    pub fn set_job_controls(&self, running: bool, retry: bool) {
        self.btn_jobs_cancel
            .setTitle(&NSString::from_str(if running {
                "취소"
            } else {
                "닫기"
            }));
        self.btn_jobs_retry.setEnabled(retry);
        self.btn_jobs_retry.setHidden(!retry);
    }
    pub fn set_keyboard_profile(&self, finder: bool) {
        self.btn_rename
            .setToolTip(Some(&NSString::from_str(if finder {
                "이름 변경 (Return · ⌘Enter)"
            } else {
                "이름 변경 (F2 · Ctrl/⌘Enter)"
            })));
        self.btn_cut.setToolTip(Some(&NSString::from_str(if finder {
            "잘라내기 · 다른 폴더로 이동"
        } else {
            "잘라내기 (Ctrl 또는 ⌘X)"
        })));
        self.btn_up.setToolTip(Some(&NSString::from_str(if finder {
            "상위 폴더 (⌘↑)"
        } else {
            "상위 폴더 (⌥↑)"
        })));
        for button in [&self.btn_rename, &self.btn_cut, &self.btn_up] {
            unsafe {
                if let Some(label) = button.toolTip() {
                    let _: () = msg_send![&**button,setAccessibilityLabel:&*label];
                }
            }
        }
        self.breadcrumb
            .setToolTip(Some(&NSString::from_str(if finder {
                "현재 폴더 또는 빈 곳을 한 번 클릭하면 경로 선택 · ⌘L"
            } else {
                "현재 폴더 또는 빈 곳을 한 번 클릭하면 경로 선택 · Ctrl/⌘L"
            })));
    }
}

#[cfg(feature = "native-ui-tests")]
impl ExplorerViews {
    pub(crate) fn theme_tab_bar(&self) -> &NSView {
        &self.tab_bar
    }
}
