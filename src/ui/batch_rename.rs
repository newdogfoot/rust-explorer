use super::*;
use crate::core::rename::{RenamePattern, RenameRow};

pub struct PreviewIvars {
    rows: RefCell<Arc<Vec<RenameRow>>>,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[name = "RERenamePreviewDataSource"]
    #[thread_kind=MainThreadOnly]
    #[ivars=PreviewIvars]
    pub struct RenamePreview;
    unsafe impl NSObjectProtocol for RenamePreview {}
    unsafe impl NSTableViewDataSource for RenamePreview {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn count(&self, _: &NSTableView) -> isize {
            self.ivars().rows.borrow().len() as isize
        }
        #[unsafe(method_id(tableView:objectValueForTableColumn:row:))]
        fn value(
            &self,
            _: &NSTableView,
            column: Option<&NSTableColumn>,
            index: isize,
        ) -> Option<Retained<AnyObject>> {
            (|| {
                let rows = self.ivars().rows.borrow();
                let row = rows.get(index as usize)?;
                let column = column
                    .map(|c| c.identifier().to_string())
                    .unwrap_or_default();
                let value = match column.as_str() {
                    "before" => row.old_name.clone(),
                    "after" => row.new_name.clone(),
                    _ => row.error.clone().unwrap_or_else(|| {
                        if row.changed {
                            "변경".into()
                        } else {
                            "유지".into()
                        }
                    }),
                };
                Some(Retained::into_super(Retained::into_super(
                    NSString::from_str(&value),
                )))
            })()
        }
    }
);
impl RenamePreview {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PreviewIvars {
            rows: RefCell::new(Arc::new(vec![])),
        });
        unsafe { msg_send![super(this), init] }
    }
}
pub struct BatchDialog {
    pub id: u64,
    pub generation: u64,
    pub ready_generation: u64,
    pub due: Option<Instant>,
    pub cancel: Cancellation,
    pub paths: Arc<Vec<PathBuf>>,
    pub mode: Retained<NSPopUpButton>,
    pub first: Retained<NSTextField>,
    pub second: Retained<NSTextField>,
    pub first_label: Retained<NSTextField>,
    pub second_label: Retained<NSTextField>,
    pub summary: Retained<NSTextField>,
    pub table: Retained<NSTableView>,
    pub model: Retained<RenamePreview>,
    pub apply: Retained<NSButton>,
    pub rows: Arc<Vec<RenameRow>>,
    pub signature: Option<(isize, String, String)>,
}
impl BatchDialog {
    fn input(&self) -> (isize, String, String) {
        (
            self.mode.indexOfSelectedItem(),
            self.first.stringValue().to_string(),
            self.second.stringValue().to_string(),
        )
    }
}

impl Controller {
    pub(super) fn selected_paths_in_order(&self) -> Vec<PathBuf> {
        self.with_tab(|t| {
            t.visible
                .iter()
                .filter_map(|&i| t.entries.get(i))
                .filter(|e| t.selected.contains(&e.id))
                .map(|e| e.path.clone())
                .collect()
        })
    }
    pub(super) fn show_batch_rename(&self) {
        let paths = self.selected_paths_in_order();
        if paths.len() < 2 || self.views().window.attachedSheet().is_some() {
            return;
        }
        self.cancel_inline_rename();
        self.ivars().state.borrow_mut().folder_request = 0;
        unsafe {
            let mtm = self.mtm();
            let panel = NSView::initWithFrame(NSView::alloc(mtm), rect(0., 0., 600., 280.));
            let mode_label = text(mtm, "방식", 12.);
            mode_label.setFrame(rect(0., 248., 95., 22.));
            panel.addSubview(&mode_label);
            let mode = NSPopUpButton::initWithFrame_pullsDown(
                NSPopUpButton::alloc(mtm),
                rect(108., 245., 250., 28.),
                false,
            );
            for title in ["글자 바꾸기", "앞·뒤에 글자 붙이기", "이름과 번호 붙이기"]
            {
                mode.addItemWithTitle(&NSString::from_str(title));
            }
            mode.setTarget(Some(self));
            mode.setAction(Some(sel!(batchRenameModeChanged:)));
            mode.setAccessibilityLabel(Some(ns_string!("이름 변경 방식")));
            panel.addSubview(&mode);
            let refresh =
                NSButton::initWithFrame(NSButton::alloc(mtm), rect(464., 245., 136., 28.));
            refresh.setTitle(ns_string!("다시 확인"));
            refresh.setBezelStyle(NSBezelStyle::Push);
            refresh.setTarget(Some(self));
            refresh.setAction(Some(sel!(refreshRenamePreview:)));
            panel.addSubview(&refresh);
            let first_label = text(mtm, "찾을 내용", 12.);
            first_label.setFrame(rect(0., 216., 100., 22.));
            panel.addSubview(&first_label);
            let second_label = text(mtm, "바꿀 내용", 12.);
            second_label.setFrame(rect(0., 184., 100., 22.));
            panel.addSubview(&second_label);
            let first =
                NSTextField::initWithFrame(NSTextField::alloc(mtm), rect(108., 212., 490., 26.));
            let second =
                NSTextField::initWithFrame(NSTextField::alloc(mtm), rect(108., 180., 490., 26.));
            for field in [&first, &second] {
                single_line_editor(field);
                field.setDelegate(Some(ProtocolObject::from_ref(self)));
                panel.addSubview(field);
            }
            first.setAccessibilityLabel(Some(ns_string!("찾을 내용")));
            second.setAccessibilityLabel(Some(ns_string!("바꿀 내용")));
            first.setPlaceholderString(Some(ns_string!("파일 이름에서 바꿀 부분")));
            second.setPlaceholderString(Some(ns_string!("비워 두면 찾은 글자를 지웁니다")));
            let summary = text(
                mtm,
                "내용을 입력하면 변경 결과와 이름 충돌을 확인합니다.",
                12.,
            );
            summary.setFrame(rect(0., 151., 600., 23.));
            panel.addSubview(&summary);
            let scroll =
                NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0., 0., 600., 144.));
            scroll.setHasVerticalScroller(true);
            scroll.setBorderType(NSBorderType::BezelBorder);
            let table =
                NSTableView::initWithFrame(NSTableView::alloc(mtm), rect(0., 0., 580., 144.));
            table.setRowHeight(24.);
            table.setUsesAlternatingRowBackgroundColors(true);
            for (key, title, width) in [
                ("before", "현재 이름", 205.),
                ("after", "변경할 이름", 220.),
                ("state", "확인", 155.),
            ] {
                let column = NSTableColumn::initWithIdentifier(
                    NSTableColumn::alloc(mtm),
                    &NSString::from_str(key),
                );
                column.setTitle(&NSString::from_str(title));
                column.setWidth(width);
                column.setEditable(false);
                table.addTableColumn(&column);
            }
            let model = RenamePreview::new(mtm);
            table.setDataSource(Some(ProtocolObject::from_ref(&*model)));
            scroll.setDocumentView(Some(&table));
            panel.addSubview(&scroll);
            let alert = NSAlert::new(mtm);
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(&NSString::from_str(&format!(
                "항목 {}개 이름 변경",
                paths.len()
            )));
            alert.setInformativeText(ns_string!("결과를 확인한 뒤 적용하세요. 번호는 현재 목록 순서를 따르며 기존 파일을 덮어쓰지 않습니다."));
            let apply = alert.addButtonWithTitle(ns_string!("이름 변경"));
            apply.setEnabled(false);
            alert.addButtonWithTitle(ns_string!("취소"));
            alert.setAccessoryView(Some(&panel));
            let id = self.ivars().runtime.next_id();
            self.ivars().state.borrow_mut().batch_dialog = Some(BatchDialog {
                id,
                generation: 0,
                ready_generation: 0,
                due: None,
                cancel: Cancellation::default(),
                paths: Arc::new(paths),
                mode,
                first: first.clone(),
                second,
                first_label,
                second_label,
                summary,
                table,
                model,
                apply,
                rows: Arc::new(vec![]),
                signature: None,
            });
            let controller = self.retained();
            let callback = block2::RcBlock::new(move |response: isize| {
                let dialog = controller.ivars().state.borrow_mut().batch_dialog.take();
                let Some(dialog) = dialog else { return };
                dialog.cancel.cancel();
                if response != 1000 {
                    return;
                }
                if dialog.ready_generation != dialog.generation
                    || dialog.signature.as_ref() != Some(&dialog.input())
                    || dialog.rows.iter().any(|r| r.error.is_some())
                {
                    controller.notice("입력값이 바뀌었습니다. 미리보기를 다시 확인하세요.", None);
                    return;
                }
                let rows: Vec<_> = dialog.rows.iter().filter(|r| r.changed).collect();
                if rows.is_empty() {
                    return;
                }
                controller.start_operation(Operation {
                    completed_sources: vec![],
                    renames: rows.iter().filter_map(|r| r.spec.clone()).collect(),
                    kind: OperationKind::BatchRename,
                    sources: rows.iter().map(|r| StoredPath::new(&r.source)).collect(),
                    destination: None,
                    name: None,
                });
            });
            alert.beginSheetModalForWindow_completionHandler(&self.views().window, Some(&callback));
            alert.window().makeFirstResponder(Some(&first));
        }
    }
    pub(super) fn batch_mode_changed(&self) {
        let controls = self.ivars().state.borrow().batch_dialog.as_ref().map(|d| {
            (
                d.mode.indexOfSelectedItem(),
                d.first.clone(),
                d.second.clone(),
                d.first_label.clone(),
                d.second_label.clone(),
            )
        });
        let Some((mode, first, second, a, b)) = controls else {
            return;
        };
        let (first_name, second_name, first_hint, second_hint) = match mode {
            1 => (
                "앞에 붙일 글자",
                "뒤에 붙일 글자",
                "예: 초안_",
                "예: _수정본",
            ),
            2 => ("새 이름", "시작 번호", "예: 여행 사진", "예: 001"),
            _ => (
                "찾을 내용",
                "바꿀 내용",
                "파일 이름에서 바꿀 부분",
                "비워 두면 찾은 글자를 지웁니다",
            ),
        };
        unsafe {
            a.setStringValue(&NSString::from_str(first_name));
            b.setStringValue(&NSString::from_str(second_name));
            first.setAccessibilityLabel(Some(&NSString::from_str(first_name)));
            second.setAccessibilityLabel(Some(&NSString::from_str(second_name)));
            first.setPlaceholderString(Some(&NSString::from_str(first_hint)));
            second.setPlaceholderString(Some(&NSString::from_str(second_hint)));
            first.setStringValue(ns_string!(""));
            second.setStringValue(&NSString::from_str(if mode == 2 { "001" } else { "" }));
        }
        self.queue_batch_preview();
    }
    pub(super) fn batch_text_changed(&self, notification: &NSNotification) {
        let relevant = self
            .ivars()
            .state
            .borrow()
            .batch_dialog
            .as_ref()
            .is_some_and(|d| {
                let first: &AnyObject = &d.first;
                let second: &AnyObject = &d.second;
                notification
                    .object()
                    .is_some_and(|o| std::ptr::eq(&*o, first) || std::ptr::eq(&*o, second))
            });
        if relevant {
            self.queue_batch_preview();
        }
    }
    pub(super) fn queue_batch_preview(&self) {
        let controls = {
            let mut state = self.ivars().state.borrow_mut();
            let Some(d) = state.batch_dialog.as_mut() else {
                return;
            };
            d.cancel.cancel();
            d.generation += 1;
            d.due = Some(Instant::now() + Duration::from_millis(180));
            (d.apply.clone(), d.summary.clone())
        };
        controls.0.setEnabled(false);
        controls.1.setStringValue(ns_string!("미리보기 갱신 중…"));
    }
    pub(super) fn tick_batch_preview(&self) {
        let request = {
            let mut state = self.ivars().state.borrow_mut();
            let Some(d) = state.batch_dialog.as_mut() else {
                return;
            };
            if !d.due.is_some_and(|due| due <= Instant::now()) {
                return;
            }
            d.due = None;
            d.cancel = Cancellation::default();
            (
                d.id,
                d.generation,
                d.paths.clone(),
                d.input(),
                d.cancel.clone(),
                d.summary.clone(),
            )
        };
        let (id, generation, paths, input, cancel, summary) = request;
        let pattern = (|| -> Result<RenamePattern, &'static str> {
            if input.1.len() > 1024 || input.2.len() > 1024 {
                return Err("입력한 글자가 너무 깁니다");
            }
            match input.0 {
                0 if input.1.is_empty() => {
                    Err("찾을 내용을 입력하세요. 대소문자를 구분하며 확장자도 포함합니다.")
                }
                0 => Ok(RenamePattern::Replace {
                    find: input.1.clone(),
                    replacement: input.2.clone(),
                }),
                1 if input.1.is_empty() && input.2.is_empty() => {
                    Err("앞 또는 뒤에 붙일 글자를 입력하세요. 확장자는 유지합니다.")
                }
                1 => Ok(RenamePattern::Add {
                    prefix: input.1.clone(),
                    suffix: input.2.clone(),
                }),
                _ => {
                    if input.1.trim().is_empty() {
                        return Err("새 이름을 입력하세요. 파일 확장자는 유지합니다.");
                    }
                    let start = input
                        .2
                        .parse::<u64>()
                        .map_err(|_| "시작 번호에는 0 이상의 정수를 입력하세요")?;
                    Ok(RenamePattern::Number {
                        base: input.1.clone(),
                        start,
                        digits: input.2.len().min(20),
                    })
                }
            }
        })();
        let Ok(pattern) = pattern else {
            summary.setStringValue(&NSString::from_str(pattern.err().unwrap()));
            return;
        };
        if let Some(d) = self.ivars().state.borrow_mut().batch_dialog.as_mut() {
            d.signature = Some(input);
        }
        let sender = self.ivars().runtime.sender.clone();
        let window = self.id();
        if let Err(error) = self.ivars().runtime.io.submit(move || {
            let result =
                crate::core::rename::preview(&paths, &pattern, &cancel).map_err(|e| e.to_string());
            let _ = sender.send(Event::RenamePreview {
                window,
                dialog: id,
                generation,
                result,
            });
        }) {
            summary.setStringValue(&NSString::from_str(error));
        }
    }
    pub(super) fn show_rename_preview(
        &self,
        id: u64,
        generation: u64,
        result: Result<Vec<RenameRow>, String>,
    ) {
        let controls = {
            let mut state = self.ivars().state.borrow_mut();
            let Some(d) = state
                .batch_dialog
                .as_mut()
                .filter(|d| d.id == id && d.generation == generation)
            else {
                return;
            };
            match result {
                Ok(rows) => {
                    d.rows = Arc::new(rows);
                    d.ready_generation = generation;
                }
                Err(error) => {
                    let summary = d.summary.clone();
                    drop(state);
                    summary.setStringValue(&NSString::from_str(&error));
                    return;
                }
            }
            (
                d.rows.clone(),
                d.model.clone(),
                d.table.clone(),
                d.apply.clone(),
                d.summary.clone(),
            )
        };
        let (rows, model, table, apply, summary) = controls;
        let changed = rows.iter().filter(|r| r.changed).count();
        let errors = rows.iter().filter(|r| r.error.is_some()).count();
        *model.ivars().rows.borrow_mut() = rows.clone();
        unsafe {
            table.reloadData();
        }
        apply.setEnabled(changed > 0 && errors == 0);
        summary.setStringValue(&NSString::from_str(&format!(
            "{}개 항목 · 변경 {}개 · 오류 {}개{}",
            rows.len(),
            changed,
            errors,
            if changed == 0 && errors == 0 {
                " · 바뀌는 이름이 없습니다"
            } else {
                ""
            }
        )));
    }
}
