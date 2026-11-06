use rust_explorer::core::{drag_drop, operations::*, store::Store, worker::Cancellation};
use std::{fs, os::unix::fs::MetadataExt};

#[test]
fn drop_moves_selected_file_and_folder_on_the_same_volume_and_undo_restores_them() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("원본");
    let target = root.path().join("받는 폴더");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let file = source.join("한글😀.txt");
    let folder = source.join("폴더");
    fs::write(&file, b"selected file").unwrap();
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("inside"), b"nested content").unwrap();
    let untouched = source.join("선택하지 않은 파일");
    fs::write(&untouched, b"leave this alone").unwrap();
    let inode = fs::metadata(&file).unwrap().ino();
    let paths = drag_drop::effective_sources(vec![file.clone(), folder.clone()], &target, false);
    let store = Store::open(&root.path().join("state.sqlite")).unwrap();
    let mut journal = execute(
        Operation::transfer(OperationKind::Drop, paths, target.clone()),
        &store,
        &Cancellation::default(),
        |_| panic!("no conflict expected"),
        |_| {},
    );
    assert_eq!(journal.status, "complete", "{:?}", journal.errors);
    assert_eq!(journal.completed.len(), 2);
    assert!(!file.exists() && !folder.exists());
    assert_eq!(
        fs::read(target.join("한글😀.txt")).unwrap(),
        b"selected file"
    );
    assert_eq!(
        fs::metadata(target.join("한글😀.txt")).unwrap().ino(),
        inode
    );
    assert_eq!(
        fs::read(target.join("폴더/inside")).unwrap(),
        b"nested content"
    );
    assert_eq!(fs::read(&untouched).unwrap(), b"leave this alone");
    undo(&mut journal, &store, &Cancellation::default()).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"selected file");
    assert_eq!(fs::read(folder.join("inside")).unwrap(), b"nested content");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn same_folder_through_a_directory_alias_is_noop_or_explicit_duplicate() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("real");
    fs::create_dir(&source).unwrap();
    let alias = root.path().join("alias");
    symlink(&source, &alias).unwrap();
    let file = source.join("item.txt");
    fs::write(&file, b"original").unwrap();
    let store = Store::open(&root.path().join("db")).unwrap();
    for kind in [OperationKind::Drop, OperationKind::Copy] {
        let result = execute(
            Operation::transfer(kind, vec![file.clone()], alias.clone()),
            &store,
            &Cancellation::default(),
            |_| panic!("same-location action needs no overwrite"),
            |_| {},
        );
        assert_eq!(result.status, "complete", "{:?}", result.errors);
        assert_eq!(fs::read(&file).unwrap(), b"original");
    }
    assert_eq!(fs::read(source.join("item (2).txt")).unwrap(), b"original");
}

#[test]
fn same_volume_folder_move_and_undo_do_not_read_locked_descendants() {
    use std::os::unix::fs::PermissionsExt;
    struct Restore(Vec<std::path::PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for p in &self.0 {
                let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o700));
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("folder");
    let dest = root.path().join("destination");
    fs::create_dir_all(source.join("locked")).unwrap();
    fs::create_dir(&dest).unwrap();
    fs::write(source.join("locked/file"), b"preserve").unwrap();
    let _restore = Restore(vec![source.join("locked"), dest.join("folder/locked")]);
    fs::set_permissions(source.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let store = Store::open(&root.path().join("state/db")).unwrap();
    let mut journal = execute(
        Operation::transfer(OperationKind::Drop, vec![source.clone()], dest.clone()),
        &store,
        &Cancellation::default(),
        |_| panic!("unexpected collision"),
        |_| {},
    );
    assert_eq!(journal.status, "complete", "{:?}", journal.errors);
    assert!(!source.exists());
    undo(&mut journal, &store, &Cancellation::default()).unwrap();
    assert!(source.exists());
    fs::set_permissions(source.join("locked"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(fs::read(source.join("locked/file")).unwrap(), b"preserve");
}

#[test]
fn recovery_of_committed_folder_move_preserves_nested_edits_and_skips_retry() {
    use rust_explorer::core::model::Fingerprint;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("folder");
    let destination = root.path().join("destination");
    let target = destination.join("folder");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("nested/file"), b"before").unwrap();
    let store = Store::open(&root.path().join("state/db")).unwrap();
    let journal = Journal {
        id: "interrupted-folder-drop".into(),
        request: Operation::transfer(OperationKind::Drop, vec![source.clone()], destination),
        status: "running".into(),
        undo: vec![],
        pending: Some(PendingInstall {
            staged: StoredPath::new(&source),
            target: StoredPath::new(&target),
            backup: None,
            original: Some(StoredPath::new(&source)),
            staged_id: Fingerprint::read(&source).unwrap(),
        }),
        errors: vec![],
        completed: vec![],
        skipped: 0,
    };
    store
        .save_job(&journal.id, &journal.status, &journal)
        .unwrap();
    rust_explorer::platform::rename_exclusive(&source, &target).unwrap();
    // Moving the same directory back must preserve edits inside it, not
    // reject them as though undo would delete the folder as a copied result.
    fs::write(target.join("nested/file"), b"changed after move").unwrap();
    let mut journals = recover(&store);
    assert_eq!(journals.len(), 1);
    let journal = &mut journals[0];
    assert_eq!(journal.status, "interrupted", "{:?}", journal.errors);
    assert!(journal.can_undo());
    assert!(retry_request(journal).sources.is_empty());
    undo(journal, &store, &Cancellation::default()).unwrap();
    assert_eq!(
        fs::read(source.join("nested/file")).unwrap(),
        b"changed after move"
    );
    assert!(!target.exists());
}
