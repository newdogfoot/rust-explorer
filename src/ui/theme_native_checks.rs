//! Hidden AppKit QA: no input events, activation, system preferences or real user DB.
use super::*;
use crate::core::{
    store::Store,
    theme::{THEMES, ThemeId},
};
use std::fs;

unsafe fn pump(windows: &[&Controller], runtime: &Arc<Runtime>) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        while let Ok(event) = runtime.receiver.try_recv() {
            match event {
                Event::PreferencesChanged => {
                    let prefs = runtime.preferences.lock().unwrap().clone();
                    for c in windows {
                        c.apply_preferences(prefs.clone(), true);
                    }
                }
                Event::Volumes(_) | Event::Recovered(_) => {}
                Event::Message { title, detail, .. } => panic!("{title}: {detail}"),
                other => windows[0].handle_event(other),
            }
        }
        if windows.iter().all(|c| c.with_tab(|t| !t.loading)) {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
}
unsafe fn capture(root: &NSView, path: &Path) {
    root.layoutSubtreeIfNeeded();
    let bitmap = root
        .bitmapImageRepForCachingDisplayInRect(root.bounds())
        .unwrap();
    let draw =
        block2::RcBlock::new(|| root.cacheDisplayInRect_toBitmapImageRep(root.bounds(), &bitmap));
    root.effectiveAppearance()
        .performAsCurrentDrawingAppearance(&draw);
    let data = bitmap
        .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        .unwrap();
    fs::write(path, data.to_vec()).unwrap();
}
fn p95(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 * 0.95).ceil() as usize - 1).min(v.len() - 1)]
}
fn cpu_seconds() -> f64 {
    unsafe {
        let mut r = std::mem::zeroed::<libc::rusage>();
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut r), 0);
        r.ru_utime.tv_sec as f64
            + r.ru_utime.tv_usec as f64 / 1e6
            + r.ru_stime.tv_sec as f64
            + r.ru_stime.tv_usec as f64 / 1e6
    }
}

fn resident_bytes() -> u64 {
    unsafe {
        let mut info = std::mem::zeroed::<libc::proc_taskinfo>();
        let size = std::mem::size_of_val(&info) as i32;
        assert_eq!(
            libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDTASKINFO,
                0,
                (&mut info as *mut libc::proc_taskinfo).cast(),
                size
            ),
            size
        );
        info.pti_resident_size
    }
}

pub fn run() {
    objc2::rc::autoreleasepool(|_| unsafe {
        let mtm = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        let reopen = std::env::var_os("RUST_EXPLORER_THEME_REOPEN");
        let root = reopen.clone().map(PathBuf::from).unwrap_or_else(|| {
            std::env::temp_dir().join(format!("RustExplorer-themes-{}", uuid::Uuid::new_v4()))
        });
        let files = root.join("files");
        fs::create_dir_all(&files).unwrap();
        let db = root.join("state.sqlite");
        let store = Store::open(&db).unwrap();
        if reopen.is_none() {
            fs::create_dir_all(files.join("업무 자료")).unwrap();
            fs::create_dir_all(files.join("촬영 원본")).unwrap();
            for name in [
                "프로젝트 제안서.pdf",
                "11월 예산.xlsx",
                "디자인 자료.zip",
                "가나다라마바사아자차카타파하한.txt",
                "1234567890123456789.txt",
                "가족👨‍👩‍👧‍👦_사진_긴_파일명_잘림_확인.png",
            ] {
                fs::write(files.join(name), b"theme QA only").unwrap();
            }
            let prefs = Preferences {
                theme: ThemeId::new(1),
                access_intro_dismissed: true,
                ..Default::default()
            };
            store.set("preferences", &prefs).unwrap();
        }
        let runtime = Runtime::new(store.clone());
        let start = Instant::now();
        let c = Controller::new_hidden(
            mtm,
            runtime.clone(),
            vec![SavedTab {
                path: path_bytes(&files),
                sort: SortField::Name,
                descending: false,
            }],
        );
        let construction_ms = start.elapsed().as_secs_f64() * 1000.;
        pump(&[&c], &runtime);
        if reopen.is_some() {
            assert_eq!(c.ivars().state.borrow().prefs.theme, ThemeId::new(30));
            assert_eq!(c.views().palette.get().spec.id, 30);
            assert!(!c.views().window.isVisible());
            println!(
                "ok: a fresh process restored preset 30 from SQLite and painted the hidden window"
            );
            return;
        }
        let output = std::env::var_os("RUST_EXPLORER_THEME_RENDER_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("renders"));
        fs::create_dir_all(&output).unwrap();
        // A second window shares the runtime; no duplicate scans during theme changes.
        let second = Controller::new_hidden(
            mtm,
            runtime.clone(),
            vec![SavedTab {
                path: path_bytes(&files),
                sort: SortField::Name,
                descending: false,
            }],
        );
        // Deliver events by owner to each controller until both folders are loaded.
        let end = Instant::now() + Duration::from_secs(10);
        while second.with_tab(|t| t.loading) {
            if let Ok(e) = runtime.receiver.recv_timeout(Duration::from_millis(50)) {
                second.handle_event(e);
            }
            assert!(Instant::now() < end);
        }
        let panel = super::super::appearance::AppearancePanel::new(mtm, runtime.clone(), &c);
        super::super::THEME_TEST_WINDOWS
            .with(|w| *w.borrow_mut() = vec![c.clone(), second.clone()]);
        c.ivars().state.borrow_mut().appearance_panel = Some(panel.clone());
        let row_count = c.with_tab(|t| t.visible.len());
        assert_eq!(row_count, 8);
        c.views()
            .table
            .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(2), false);
        c.selection_changed();
        let chosen = c.selected_paths();
        let generation = c.with_tab(|t| t.generation);
        let entries = c.with_tab(|t| t.entries.clone());
        let mut switch_ms = vec![];
        let mut configurations = 0;
        for id in 1..=30 {
            objc2::rc::autoreleasepool(|_| {
                // The actual native selection delegate invokes choose/save.
                panel.table().selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(id as usize),
                    false,
                );
                panel.choose(ThemeId::new(id));
                assert_eq!(
                    c.views().palette.get().spec.id,
                    id,
                    "selection must paint before any persistence wait/event pump"
                );
                assert_eq!(second.views().palette.get().spec.id, id);
                assert!(runtime.persistence.flush(Duration::from_secs(5)));
                pump(&[&c, &second], &runtime);
                assert_eq!(store.preferences().theme, ThemeId::new(id));
                assert_eq!(second.views().palette.get().spec.id, id);
                let saved = runtime.preferences.lock().unwrap().clone();
                for (width, height) in [(1040., 680.), (760., 480.)] {
                    c.views().window.setContentSize(NSSize::new(width, height));
                    for mode in [ViewMode::Details, ViewMode::Icons] {
                        for size in if mode == ViewMode::Icons {
                            vec![IconSize::Small, IconSize::Medium, IconSize::Large]
                        } else {
                            vec![IconSize::Small]
                        } {
                            let mut prefs = saved.clone();
                            prefs.view = mode;
                            prefs.icon_size = size;
                            let start = Instant::now();
                            c.apply_preferences(prefs, true);
                            c.views().root.layoutSubtreeIfNeeded();
                            switch_ms.push(start.elapsed().as_secs_f64() * 1000.);
                            for table in [&*c.views().table, &*c.views().sidebar] {
                                table.enumerateAvailableRowViewsUsingBlock(&block2::RcBlock::new(
                                    |view: std::ptr::NonNull<NSTableRowView>, _| {
                                        assert_eq!(
                                            super::super::theme::row_palette(view.as_ref()),
                                            Some(c.views().palette.get()),
                                            "stale buffered row"
                                        );
                                    },
                                ));
                            }
                            if mode == ViewMode::Details {
                                let mut end = 0f64;
                                for column in 0..c.views().table.numberOfColumns() {
                                    if !c
                                        .views()
                                        .table
                                        .tableColumns()
                                        .objectAtIndex(column as usize)
                                        .isHidden()
                                    {
                                        let r = c.views().table.rectOfColumn(column);
                                        end = end.max(r.origin.x + r.size.width);
                                    }
                                }
                                assert!(
                                    end <= c.views().list_scroll.contentView().bounds().size.width
                                        + 1.,
                                    "columns extend past viewport: {end} > {}",
                                    c.views().list_scroll.contentView().bounds().size.width
                                );
                            }
                            configurations += 1;
                            assert_eq!(c.selected_paths(), chosen);
                            assert_eq!(c.with_tab(|t| t.generation), generation);
                            assert!(Arc::ptr_eq(&entries, &c.with_tab(|t| t.entries.clone())));
                            if mode == ViewMode::Icons {
                                c.views()
                                    .collection
                                    .collectionViewLayout()
                                    .unwrap()
                                    .prepareLayout();
                                for index in c.views().collection.indexPathsForVisibleItems() {
                                    if let Some(item) = c.views().collection.itemAtIndexPath(&index)
                                    {
                                        let label = item.textField().unwrap();
                                        let name = c.entry_at(index.item()).unwrap().name;
                                        assert_eq!(label.toolTip().unwrap().to_string(), name);
                                        assert!(!label.isEditable() && !label.isSelectable());
                                        assert!(label.frame().size.width >= 126.);
                                        label.setMaximumNumberOfLines(0);
                                        let full = label.sizeThatFits(NSSize::new(
                                            label.frame().size.width,
                                            1000.,
                                        ));
                                        label.setMaximumNumberOfLines(2);
                                        if name.starts_with("가나다") || name.starts_with("123456")
                                        {
                                            assert!(
                                                full.height <= label.frame().size.height,
                                                "theme {id} {name}: {:?}",
                                                full
                                            );
                                        }
                                        let p = c.views().palette.get();
                                        assert!(p.ink.contrast(p.selection) >= 4.5);
                                    }
                                }
                            } else {
                                for column in c.views().table.tableColumns() {
                                    if !column.isHidden() {
                                        let minimum = match column.identifier().to_string().as_str()
                                        {
                                            "date" => 150.,
                                            "kind" => 110.,
                                            "size" => 80.,
                                            _ => 260.,
                                        };
                                        assert!(
                                            column.width() >= minimum,
                                            "column {} too narrow: {}",
                                            column.identifier(),
                                            column.width()
                                        );
                                    }
                                }
                                let col = c.views().table.tableColumns().objectAtIndex(0);
                                assert!(col.width() >= 260.);
                                for row in 0..row_count.min(4) {
                                    let view = c
                                        .views()
                                        .table
                                        .viewAtColumn_row_makeIfNecessary(0, row as isize, true)
                                        .unwrap()
                                        .downcast::<NSTableCellView>()
                                        .unwrap();
                                    let label = view.textField().unwrap();
                                    assert_eq!(
                                        label.toolTip().unwrap().to_string(),
                                        c.entry_at(row as isize).unwrap().name
                                    );
                                    assert!(label.frame().origin.y >= 0.);
                                    assert!(
                                        label.frame().origin.y + label.frame().size.height
                                            <= c.views().table.rowHeight() + 1.
                                    );
                                }
                            }
                            if size == IconSize::Small && width == 1040. {
                                capture(
                                    &c.views().root,
                                    &output.join(format!(
                                        "{id:02}-{}.png",
                                        if mode == ViewMode::Icons {
                                            "icons"
                                        } else {
                                            "details"
                                        }
                                    )),
                                );
                            }
                            if width == 760.
                                && [1, 12, 18, 26, 30].contains(&id)
                                && size == IconSize::Small
                            {
                                capture(
                                    &c.views().root,
                                    &output.join(format!(
                                        "{id:02}-narrow-{}.png",
                                        if mode == ViewMode::Icons {
                                            "icons"
                                        } else {
                                            "details"
                                        }
                                    )),
                                );
                            }
                        }
                    }
                }
                assert!(c.ivars().state.borrow().images.icon_count() <= 13);
            });
        }
        assert_eq!(configurations, 240);
        for id in [1, 12, 18, 26] {
            let mut prefs = runtime.preferences.lock().unwrap().clone();
            prefs.theme = ThemeId::new(id);
            c.apply_preferences(prefs, true);
            capture(
                c.views().theme_tab_bar(),
                &output.join(format!("{id:02}-tabs.png")),
            );
        }
        capture(
            &panel.window().contentView().unwrap(),
            &output.join("settings.png"),
        );
        // Only this isolated test application's appearance changes, not macOS.
        panel.choose(ThemeId::default());
        for (name, expected) in [(NSAppearanceNameAqua, 1), (NSAppearanceNameDarkAqua, 18)] {
            app.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
            c.ivars().state.borrow_mut().last_theme_check = Instant::now() - Duration::from_secs(2);
            c.tick();
            panel.sync_environment();
            assert_eq!(c.views().palette.get().spec.id, expected);
            assert_eq!(
                panel.preview_theme().spec.id,
                expected,
                "system-mode settings preview must match the explorer"
            );
            capture(
                &panel.window().contentView().unwrap(),
                &output.join(format!("settings-system-{expected}.png")),
            );
        }
        panel.choose(ThemeId::new(12));
        app.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameAqua).as_deref());
        c.ivars().state.borrow_mut().last_theme_check = Instant::now() - Duration::from_secs(2);
        c.tick();
        assert_eq!(c.views().palette.get().spec.id, 12);
        app.setAppearance(None);
        // A rapid burst must persist the last choice rather than lose a full queue.
        for i in 0..240 {
            let mut prefs = runtime.preferences.lock().unwrap().clone();
            prefs.theme = ThemeId::new((i % 30 + 1) as u8);
            runtime.save_preferences(prefs);
        }
        assert!(runtime.persistence.flush(Duration::from_secs(5)));
        pump(&[&c, &second], &runtime);
        assert_eq!(store.preferences().theme, ThemeId::new(30));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env("RUST_EXPLORER_THEME_REOPEN", &root)
            .status()
            .unwrap();
        assert!(status.success());
        // Probe O(visible rows) recoloring with a large in-memory folder. Metadata
        // is already loaded: this isolates appearance cost from storage latency.
        let large: Vec<Entry> = (0..100_000)
            .map(|n| {
                let name = format!("자료_{n:06}.txt");
                Entry {
                    id: n + 100,
                    path: files.join(&name),
                    key: name.clone(),
                    name,
                    kind: Kind::File,
                    size: Some(100),
                    modified: None,
                    metadata_loaded: true,
                    readonly: false,
                }
            })
            .collect();
        c.with_tab_mut(|t| {
            t.entries = Arc::new(large);
            t.visible = (0..100_000).collect();
            t.selected.clear();
            t.metadata.clear();
            t.first_displayed = true;
        });
        let mut prefs = runtime.preferences.lock().unwrap().clone();
        prefs.view = ViewMode::Details;
        prefs.theme = ThemeId::new(1);
        c.apply_preferences(prefs.clone(), false);
        c.views().table.scrollRowToVisible(50_000);
        c.remember_scroll();
        c.views()
            .table
            .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(50_000), false);
        c.selection_changed();
        let mut large_ms = vec![];
        let anchor = c
            .views()
            .table
            .rowAtPoint(NSPoint::new(1., c.views().table.visibleRect().origin.y));
        let mut resident_rounds = vec![];
        for _ in 0..6 {
            for id in 1..=30 {
                objc2::rc::autoreleasepool(|_| {
                    prefs.theme = ThemeId::new(id);
                    let start = Instant::now();
                    c.apply_preferences(prefs.clone(), true);
                    c.views().root.layoutSubtreeIfNeeded();
                    large_ms.push(start.elapsed().as_secs_f64() * 1000.);
                    assert!(c.views().table.isRowSelected(50_000));
                    assert_eq!(
                        c.views()
                            .table
                            .rowAtPoint(NSPoint::new(1., c.views().table.visibleRect().origin.y)),
                        anchor,
                        "theme changes must keep the top visible row"
                    );
                    assert_eq!(c.with_tab(|t| t.generation), generation);
                });
            }
            objc2_quartz_core::CATransaction::flush();
            NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02));
            resident_rounds.push(resident_bytes());
            println!("resident theme round: {}", resident_rounds.last().unwrap());
        }
        let steady_growth = resident_rounds[5].saturating_sub(resident_rounds[2]);
        assert!(
            steady_growth < 8 * 1024 * 1024,
            "repeated theme changes grew resident memory by {steady_growth}"
        );
        prefs.view = ViewMode::Icons;
        c.apply_preferences(prefs.clone(), true);
        c.views()
            .collection
            .collectionViewLayout()
            .unwrap()
            .prepareLayout();
        let mut large_icons_ms = vec![];
        for id in 1..=30 {
            objc2::rc::autoreleasepool(|_| {
                prefs.theme = ThemeId::new(id);
                let start = Instant::now();
                c.apply_preferences(prefs.clone(), true);
                c.views().root.layoutSubtreeIfNeeded();
                large_icons_ms.push(start.elapsed().as_secs_f64() * 1000.);
            });
        }
        assert!(p95(&large_icons_ms) < 50.);
        prefs.view = ViewMode::Details;
        c.apply_preferences(prefs.clone(), true);
        // Existing tick path, process CPU time, no native event dispatch/input.
        let idle_start = Instant::now();
        let cpu = cpu_seconds();
        for _ in 0..100 {
            c.tick();
            std::thread::sleep(Duration::from_millis(50));
        }
        let idle_percent = (cpu_seconds() - cpu) / idle_start.elapsed().as_secs_f64() * 100.;
        assert!(
            !c.views().window.isVisible()
                && !second.views().window.isVisible()
                && !panel.window().isVisible()
        );
        let report = serde_json::json!({"presets":30,"configurations":configurations,"hidden_window_construction_ms":construction_ms,"layout_update_p95_ms":p95(&switch_ms),"theme_only_100k_p95_ms":p95(&large_ms),"theme_only_100k_max_ms":large_ms.iter().copied().fold(0.,f64::max),"theme_only_100k_icons_p95_ms":p95(&large_icons_ms),"resident_after_30_theme_rounds_bytes":resident_rounds,"steady_theme_switch_growth_bytes":steady_growth,"idle_cpu_percent_5s":idle_percent,"max_icon_categories":13,"restart_process_passed":true,"burst_240_last_choice_persisted":true,"input_automation":false,"scope":"Hidden AppKit windows; 100k performance fixture has loaded in-memory metadata; not a physical input latency benchmark."});
        fs::write(
            output.join("results.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("{report}");
        assert!(
            p95(&large_ms) < 50.,
            "theme-only large-folder latency target exceeded"
        );
        assert!(runtime.persistence.flush(Duration::from_secs(5)));
        super::super::THEME_TEST_WINDOWS.with(|w| w.borrow_mut().clear());
        for controller in [&c, &second] {
            runtime.unwatch(controller.id(), None);
            controller.views().window.setDelegate(None);
            controller.views().window.setNextResponder(None);
        }
        assert_eq!(THEMES.len(), 30);
        println!(
            "ok: all 30 themes, 240 native layouts, two windows, persistence, fresh process, cache bound and 100k theme switch"
        );
    });
}
