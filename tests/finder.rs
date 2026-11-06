use rust_explorer::core::{
    model::Fingerprint,
    operations::*,
    rename::{self, RenamePattern},
    store::Store,
    worker::Cancellation,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn batch(paths: &[PathBuf], pattern: RenamePattern) -> Operation {
    let rows = rename::preview(paths, &pattern, &Cancellation::default()).unwrap();
    Operation {
        completed_sources: vec![],
        renames: rows.iter().map(|r| r.spec.clone().unwrap()).collect(),
        kind: OperationKind::BatchRename,
        sources: paths.iter().map(|p| StoredPath::new(p)).collect(),
        destination: None,
        name: None,
    }
}
fn run(request: Operation, store: &Store) -> Journal {
    execute(
        request,
        store,
        &Cancellation::default(),
        |_| panic!("batch operations must not overwrite"),
        |_| {},
    )
}
fn assert_complete(journal: &Journal) {
    assert_eq!(journal.status, "complete", "{:?}", journal.errors);
}

fn persist_pending_batch(store: &Store, request: Operation, target: &Path) -> PathBuf {
    let source = request.sources[0].path();
    let before = Fingerprint::read(&source).unwrap();
    let journal = Journal {
        id: "interrupted-batch".into(),
        request,
        status: "running".into(),
        undo: vec![],
        pending: Some(PendingInstall {
            staged: StoredPath::new(&source),
            target: StoredPath::new(target),
            backup: None,
            original: Some(StoredPath::new(&source)),
            staged_id: before,
        }),
        errors: vec![],
        completed: vec![],
        skipped: 0,
    };
    store
        .save_job(&journal.id, &journal.status, &journal)
        .unwrap();
    source
}

#[test]
fn recovery_resumes_a_committed_batch_without_renumbering_or_losing_hard_links() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let paths = [dir.path().join("a.txt"), dir.path().join("b.txt")];
    fs::write(&paths[0], b"shared").unwrap();
    fs::hard_link(&paths[0], &paths[1]).unwrap();
    let request = batch(
        &paths,
        RenamePattern::Number {
            base: "자료".into(),
            start: 1,
            digits: 2,
        },
    );
    let target = dir.path().join("자료 01.txt");
    let source = persist_pending_batch(&store, request, &target);
    rust_explorer::platform::rename_exclusive(&source, &target).unwrap();
    let recovered = recover(&store);
    let journal = &recovered[0];
    assert!(journal.can_retry());
    assert_eq!(journal.completed, vec![StoredPath::new(&paths[0])]);
    let remaining = retry_request(journal);
    assert_eq!(remaining.sources, vec![StoredPath::new(&paths[1])]);
    assert_eq!(remaining.renames[0].name, "자료 02.txt".as_bytes());
    assert_complete(&run(remaining, &store));
    assert_eq!(fs::read(target).unwrap(), b"shared");
    assert_eq!(fs::read(dir.path().join("자료 02.txt")).unwrap(), b"shared");
}

#[test]
fn recovery_distinguishes_uncommitted_case_alias_and_external_hard_link() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let source = dir.path().join("lower.txt");
    fs::write(&source, b"keep").unwrap();
    let request = batch(
        std::slice::from_ref(&source),
        RenamePattern::Replace {
            find: "lower".into(),
            replacement: "LOWER".into(),
        },
    );
    let target = dir.path().join("LOWER.txt");
    persist_pending_batch(&store, request, &target);
    let recovered = recover(&store);
    assert!(recovered[0].undo.is_empty());
    assert!(recovered[0].completed.is_empty());
    assert!(recovered[0].can_retry());
    assert_complete(&run(retry_request(&recovered[0]), &store));
    let request = batch(
        std::slice::from_ref(&target),
        RenamePattern::Replace {
            find: "LOWER".into(),
            replacement: "Lower".into(),
        },
    );
    let mixed = dir.path().join("Lower.txt");
    persist_pending_batch(&store, request, &mixed);
    rust_explorer::platform::rename_exclusive(&target, &mixed).unwrap();
    let mut recovered = recover(&store);
    assert_eq!(recovered[0].completed, vec![StoredPath::new(&target)]);
    assert!(recovered[0].can_undo());
    undo(&mut recovered[0], &store, &Cancellation::default()).unwrap();
    let request = batch(
        std::slice::from_ref(&target),
        RenamePattern::Add {
            prefix: "new-".into(),
            suffix: String::new(),
        },
    );
    let extra = dir.path().join("new-LOWER.txt");
    persist_pending_batch(&store, request, &extra);
    fs::hard_link(&target, &extra).unwrap();
    let recovered = recover(&store);
    assert_eq!(recovered[0].status, "review");
    assert!(!recovered[0].can_undo());
    assert!(!recovered[0].can_retry());
    assert_eq!(fs::read(target).unwrap(), b"keep");
    assert_eq!(fs::read(extra).unwrap(), b"keep");
}

#[test]
fn recovered_folder_rename_preserves_edited_contents_and_can_be_undone() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let source = dir.path().join("folder");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/content"), b"original").unwrap();
    let request = batch(
        std::slice::from_ref(&source),
        RenamePattern::Add {
            prefix: "new-".into(),
            suffix: String::new(),
        },
    );
    let target = dir.path().join("new-folder");
    persist_pending_batch(&store, request, &target);
    rust_explorer::platform::rename_exclusive(&source, &target).unwrap();
    fs::write(target.join("nested/content"), b"edited after rename").unwrap();
    let mut recovered = recover(&store);
    assert!(recovered[0].can_undo(), "{:?}", recovered[0]);
    undo(&mut recovered[0], &store, &Cancellation::default()).unwrap();
    assert_eq!(
        fs::read(source.join("nested/content")).unwrap(),
        b"edited after rename"
    );
    assert!(!target.exists());
}

#[test]
fn numbered_batch_preserves_contents_extensions_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let paths: Vec<_> = ["한글.txt", "photo.jpeg", "folder.v2"]
        .map(|n| dir.path().join(n))
        .into();
    fs::write(&paths[0], "내용 유지").unwrap();
    fs::write(&paths[1], b"image fixture").unwrap();
    fs::create_dir(&paths[2]).unwrap();
    fs::write(paths[2].join("child"), b"folder fixture").unwrap();
    let request = batch(
        &paths,
        RenamePattern::Number {
            base: "자료".into(),
            start: 1,
            digits: 3,
        },
    );
    let mut journal = run(request, &store);
    assert_complete(&journal);
    assert_eq!(
        fs::read(dir.path().join("자료 001.txt")).unwrap(),
        "내용 유지".as_bytes()
    );
    assert_eq!(
        fs::read(dir.path().join("자료 002.jpeg")).unwrap(),
        b"image fixture"
    );
    assert_eq!(
        fs::read(dir.path().join("자료 003/child")).unwrap(),
        b"folder fixture"
    );
    undo(&mut journal, &store, &Cancellation::default()).unwrap();
    assert!(paths.iter().all(|p| p.exists()));
    assert!(!dir.path().join("자료 001.txt").exists());
}

#[test]
fn collisions_and_changed_sources_are_rejected_before_any_rename() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let paths = [dir.path().join("a.txt"), dir.path().join("aa.txt")];
    for p in &paths {
        fs::write(p, b"original").unwrap();
    }
    let pattern = RenamePattern::Replace {
        find: "a".into(),
        replacement: String::new(),
    };
    let rows = rename::preview(&paths, &pattern, &Cancellation::default()).unwrap();
    assert!(rows.iter().all(|r| r.error.is_some()));
    let failed = run(batch(&paths, pattern), &store);
    assert_eq!(failed.status, "failed");
    assert!(failed.undo.is_empty());
    assert!(paths.iter().all(|p| fs::read(p).unwrap() == b"original"));
    let request = batch(
        &paths,
        RenamePattern::Add {
            prefix: "new-".into(),
            suffix: String::new(),
        },
    );
    fs::write(&paths[1], b"changed after preview").unwrap();
    let failed = run(request, &store);
    assert_eq!(failed.status, "failed");
    assert!(paths.iter().all(|p| p.exists()));
    assert!(!dir.path().join("new-a.txt").exists());
}

#[test]
fn cancellation_retry_keeps_names_paired_with_their_original_sources() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let paths: Vec<_> = ["c.txt", "a.txt", "b.txt"]
        .map(|name| dir.path().join(name))
        .into();
    for (i, p) in paths.iter().enumerate() {
        fs::write(p, format!("source {i}")).unwrap();
    }
    let request = batch(
        &paths,
        RenamePattern::Number {
            base: "정렬 순서".into(),
            start: 1,
            digits: 2,
        },
    );
    let cancel = Cancellation::default();
    let partial = execute(
        request,
        &store,
        &cancel,
        |_| panic!(),
        |p| {
            if p.files == 1 {
                cancel.cancel();
            }
        },
    );
    assert_eq!(partial.status, "cancelled");
    assert_eq!(partial.completed.len(), 1);
    assert!(partial.can_retry());
    let remaining = retry_request(&partial);
    assert_eq!(remaining.sources[0].path(), paths[1]);
    assert_eq!(remaining.renames[0].name, "정렬 순서 02.txt".as_bytes());
    assert_complete(&run(remaining, &store));
    for i in 0..3 {
        assert_eq!(
            fs::read_to_string(dir.path().join(format!("정렬 순서 {:02}.txt", i + 1))).unwrap(),
            format!("source {i}")
        );
    }
}

#[test]
fn hard_link_ctime_changes_from_our_own_renames_do_not_break_batch_undo() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let paths: Vec<_> = ["first.txt", "second.txt", "third.txt"]
        .map(|name| dir.path().join(name))
        .into();
    fs::write(&paths[0], b"shared inode").unwrap();
    fs::hard_link(&paths[0], &paths[1]).unwrap();
    fs::hard_link(&paths[0], &paths[2]).unwrap();
    let mut result = run(
        batch(
            &paths,
            RenamePattern::Add {
                prefix: "new-".into(),
                suffix: String::new(),
            },
        ),
        &store,
    );
    assert_complete(&result);
    undo(&mut result, &store, &Cancellation::default()).unwrap();
    assert!(
        paths
            .iter()
            .all(|p| fs::read(p).unwrap() == b"shared inode")
    );
    assert_eq!(
        Fingerprint::read(&paths[0]).unwrap().ino,
        Fingerprint::read(&paths[1]).unwrap().ino
    );
}

#[test]
fn ancestor_selection_is_rejected_and_case_only_batch_is_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let parent = dir.path().join("folder");
    fs::create_dir(&parent).unwrap();
    let child = parent.join("child.txt");
    fs::write(&child, b"keep").unwrap();
    let rows = rename::preview(
        &[parent.clone(), child.clone()],
        &RenamePattern::Add {
            prefix: "new-".into(),
            suffix: String::new(),
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert!(rows.iter().all(|r| r.error.is_some()));
    let mut result = run(
        batch(
            std::slice::from_ref(&child),
            RenamePattern::Replace {
                find: "child".into(),
                replacement: "CHILD".into(),
            },
        ),
        &store,
    );
    assert_complete(&result);
    assert!(
        fs::read_dir(&parent)
            .unwrap()
            .any(|e| e.unwrap().file_name() == "CHILD.txt")
    );
    undo(&mut result, &store, &Cancellation::default()).unwrap();
    assert!(
        fs::read_dir(&parent)
            .unwrap()
            .any(|e| e.unwrap().file_name() == "child.txt")
    );
}

#[test]
fn duplicate_files_and_directories_preserves_originals_and_can_be_undone() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let folder = dir.path().join("폴더");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("child"), b"inside").unwrap();
    std::os::unix::fs::symlink("child", folder.join("link")).unwrap();
    let file = dir.path().join("문서.txt");
    fs::write(&file, b"text").unwrap();
    let package = dir.path().join("샘플.app");
    fs::create_dir_all(package.join("Contents")).unwrap();
    fs::write(
        package.join("Contents/Info.plist"),
        b"generated package fixture",
    )
    .unwrap();
    let mut result = run(
        Operation {
            completed_sources: vec![],
            renames: vec![],
            kind: OperationKind::Duplicate,
            sources: vec![
                StoredPath::new(&folder),
                StoredPath::new(&file),
                StoredPath::new(&package),
            ],
            destination: None,
            name: None,
        },
        &store,
    );
    assert_complete(&result);
    assert_eq!(
        fs::read(dir.path().join("폴더 (2)/child")).unwrap(),
        b"inside"
    );
    assert_eq!(
        fs::read_link(dir.path().join("폴더 (2)/link")).unwrap(),
        Path::new("child")
    );
    assert_eq!(fs::read(dir.path().join("문서 (2).txt")).unwrap(), b"text");
    assert!(
        dir.path()
            .join("샘플 (2).app/Contents/Info.plist")
            .is_file()
    );
    undo(&mut result, &store, &Cancellation::default()).unwrap();
    assert!(folder.exists() && file.exists());
    assert!(!dir.path().join("문서 (2).txt").exists());
}

#[test]
fn pre_04_operation_records_still_decode() {
    let old = r#"{"kind":"Copy","sources":[[47,97]],"destination":[47,98],"name":null}"#;
    let request: Operation = serde_json::from_str(old).unwrap();
    assert!(request.renames.is_empty());
    assert!(request.completed_sources.is_empty());
    assert_eq!(request.sources[0].path(), Path::new("/a"));
}

#[test]
fn folder_rename_never_traverses_inaccessible_descendants() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let source = dir.path().join("old");
    let child = source.join("private");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("keep"), b"contents unchanged").unwrap();
    let request = batch(
        std::slice::from_ref(&source),
        RenamePattern::Add {
            prefix: "new-".into(),
            suffix: String::new(),
        },
    );
    fs::set_permissions(&child, fs::Permissions::from_mode(0o000)).unwrap();
    let mut journal = run(request, &store);
    // Always restore the disposable fixture's permission before assertions.
    let moved = dir.path().join("new-old/private");
    let _ = fs::set_permissions(&moved, fs::Permissions::from_mode(0o700));
    let _ = fs::set_permissions(&child, fs::Permissions::from_mode(0o700));
    assert_complete(&journal);
    fs::write(moved.join("keep"), b"an edit after rename").unwrap();
    undo(&mut journal, &store, &Cancellation::default()).unwrap();
    assert_eq!(
        fs::read(child.join("keep")).unwrap(),
        b"an edit after rename"
    );
}

#[test]
fn text_file_application_lookup_is_read_only_and_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("앱 선택 시험.txt");
    fs::write(&path, b"read-only fixture").unwrap();
    let before = Fingerprint::read(&path).unwrap();
    let apps =
        rust_explorer::platform::applications_for_file(&path, &Cancellation::default()).unwrap();
    assert!(
        apps.iter().any(|app| app
            .path
            .file_name()
            .is_some_and(|name| name == "TextEdit.app")),
        "TextEdit must be offered for a text file"
    );
    let paths: std::collections::HashSet<_> = apps.iter().map(|app| &app.path).collect();
    assert_eq!(paths.len(), apps.len());
    assert!(apps.iter().filter(|app| app.is_default).count() <= 1);
    assert!(before.matches(&path));
    assert_eq!(fs::read(path).unwrap(), b"read-only fixture");
}

#[test]
#[ignore = "manual performance measurement on a disposable fixture"]
fn benchmark_1000_batch_renames() {
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let paths: Vec<_> = (0..1000)
        .map(|i| dir.path().join(format!("자료-{i:04}.txt")))
        .collect();
    for path in &paths {
        fs::write(path, b"benchmark").unwrap();
    }
    let started = Instant::now();
    let request = batch(
        &paths,
        RenamePattern::Add {
            prefix: "완료-".into(),
            suffix: String::new(),
        },
    );
    let preview_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let result = run(request, &store);
    let rename_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_complete(&result);
    assert_eq!(result.completed.len(), 1000);
    println!("{{\"items\":1000,\"preview_ms\":{preview_ms:.3},\"rename_ms\":{rename_ms:.3}}}");
}
