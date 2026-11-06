use rust_explorer::core::{operations::*, store::Store, worker::Cancellation};
use std::{
    ffi::CString,
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

fn roots() -> (PathBuf, PathBuf) {
    let a = PathBuf::from(std::env::var_os("RUST_EXPLORER_TEST_APFS").expect("APFS fixture path"));
    let b =
        PathBuf::from(std::env::var_os("RUST_EXPLORER_TEST_EXFAT").expect("exFAT fixture path"));
    for root in [&a, &b] {
        assert!(
            root.join(".rust-explorer-test-volume").exists(),
            "refuse non-fixture volume"
        );
    }
    (a, b)
}
fn run(
    store: &Store,
    kind: OperationKind,
    source: &Path,
    dest: &Path,
    choice: ConflictChoice,
) -> Journal {
    execute(
        Operation::transfer(kind, vec![source.into()], dest.into()),
        store,
        &Cancellation::default(),
        |_| ConflictDecision {
            choice,
            apply_all: false,
        },
        |_| {},
    )
}

#[test]
#[ignore = "requires disposable APFS and exFAT images; scripts/test-volumes.py"]
fn volume_scenarios() {
    let (a, b) = roots();
    assert_ne!(
        fs::metadata(&a).unwrap().dev(),
        fs::metadata(&b).unwrap().dev()
    );
    let src = tempfile::tempdir_in(&a).unwrap();
    let dst = tempfile::tempdir_in(&b).unwrap();
    let db = tempfile::tempdir().unwrap();
    let store = Store::open(&db.path().join("state.sqlite")).unwrap();
    let path = src.path().join("한글😀.txt");
    let bytes: Vec<u8> = (0..262144).map(|n| (n % 251) as u8).collect();
    fs::write(&path, &bytes).unwrap();
    fs::write(dst.path().join("한글😀.txt"), b"existing").unwrap();
    let mut j = run(
        &store,
        OperationKind::Move,
        &path,
        dst.path(),
        ConflictChoice::KeepBoth,
    );
    assert_eq!(j.status, "complete", "{:?}", j.errors);
    assert!(!path.exists());
    assert_eq!(fs::read(dst.path().join("한글😀 (2).txt")).unwrap(), bytes);
    undo(&mut j, &store, &Cancellation::default()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        fs::read(dst.path().join("한글😀.txt")).unwrap(),
        b"existing"
    );
    assert!(!dst.path().join("한글😀 (2).txt").exists());
    println!("PASS: APFS→exFAT move, KeepBoth, exact original name undo, byte comparison");

    let drag_source = src.path().join("드래그.txt");
    fs::write(&drag_source, b"drag fixture").unwrap();
    let local_target = src.path().join("drop-target");
    fs::create_dir(&local_target).unwrap();
    let drop = run(
        &store,
        OperationKind::Drop,
        &drag_source,
        &local_target,
        ConflictChoice::Cancel,
    );
    assert_eq!(drop.status, "complete", "{:?}", drop.errors);
    assert!(!drag_source.exists());
    let moved = local_target.join("드래그.txt");
    let drop = run(
        &store,
        OperationKind::Drop,
        &moved,
        dst.path(),
        ConflictChoice::Cancel,
    );
    assert_eq!(drop.status, "complete", "{:?}", drop.errors);
    assert!(moved.exists());
    assert_eq!(
        fs::read(dst.path().join("드래그.txt")).unwrap(),
        b"drag fixture"
    );
    println!("PASS: default drag moves within APFS and copies across APFS→exFAT");

    let archive = src.path().join("압축.zip");
    fs::write(&archive, include_bytes!("fixtures/unicode.zip")).unwrap();
    let extracted = run(
        &store,
        OperationKind::Extract,
        &archive,
        dst.path(),
        ConflictChoice::Cancel,
    );
    assert_eq!(extracted.status, "complete", "{:?}", extracted.errors);
    assert_eq!(
        fs::read_to_string(dst.path().join("압축/문서/한글😀.txt")).unwrap(),
        "한글 압축 fixture\n".repeat(64)
    );
    assert!(archive.exists());
    println!("PASS: Unicode ZIP extracted to exFAT, original archive preserved");

    let metadata_src = src.path().join("metadata.bin");
    fs::write(&metadata_src, b"metadata fixture").unwrap();
    fs::set_permissions(&metadata_src, fs::Permissions::from_mode(0o640)).unwrap();
    let name = CString::new("com.rust-explorer.fixture").unwrap();
    let p = CString::new(metadata_src.as_os_str().as_bytes()).unwrap();
    let attr = b"preserve me";
    assert_eq!(
        unsafe {
            libc::setxattr(
                p.as_ptr(),
                name.as_ptr(),
                attr.as_ptr().cast(),
                attr.len(),
                0,
                0,
            )
        },
        0
    );
    let same = tempfile::tempdir_in(&a).unwrap();
    let j = run(
        &store,
        OperationKind::Copy,
        &metadata_src,
        same.path(),
        ConflictChoice::Cancel,
    );
    assert_eq!(j.status, "complete", "{:?}", j.errors);
    let copied = same.path().join("metadata.bin");
    assert_eq!(
        fs::metadata(&copied).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let p = CString::new(copied.as_os_str().as_bytes()).unwrap();
    let mut value = [0u8; 128];
    let n = unsafe {
        libc::getxattr(
            p.as_ptr(),
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    assert_eq!(&value[..n as usize], attr);
    println!("PASS: APFS permission and extended attribute preservation");

    let large = src.path().join("cancel.bin");
    fs::File::create(&large)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    let cancel = Cancellation::default();
    let c = cancel.clone();
    let j = execute(
        Operation::transfer(OperationKind::Move, vec![large.clone()], dst.path().into()),
        &store,
        &cancel,
        |_| unreachable!(),
        move |p| {
            if p.current > 0 {
                c.cancel();
            }
        },
    );
    assert_eq!(j.status, "cancelled");
    assert!(large.exists());
    assert!(!dst.path().join("cancel.bin").exists());
    assert!(!fs::read_dir(dst.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".RustExplorer-stage-")
    }));
    println!("PASS: cancel cross-volume transfer keeps source and removes incomplete stage");

    let full = src.path().join("disk-full.bin");
    fs::File::create(&full)
        .unwrap()
        .set_len(320 * 1024 * 1024)
        .unwrap();
    let j = run(
        &store,
        OperationKind::Move,
        &full,
        dst.path(),
        ConflictChoice::Cancel,
    );
    assert_ne!(j.status, "complete");
    assert!(full.exists());
    assert!(!dst.path().join("disk-full.bin").exists());
    println!(
        "PASS: destination full preserves source; error={:?}",
        j.errors
    );
}

#[test]
#[ignore = "requires exFAT image mounted read-only"]
fn read_only_volume() {
    let (a, b) = roots();
    let src = tempfile::tempdir_in(a).unwrap();
    let db = tempfile::tempdir().unwrap();
    let store = Store::open(&db.path().join("state.sqlite")).unwrap();
    let file = src.path().join("read-only.txt");
    fs::write(&file, b"source").unwrap();
    let j = run(
        &store,
        OperationKind::Move,
        &file,
        &b,
        ConflictChoice::Cancel,
    );
    assert_eq!(j.status, "failed", "{:?}", j.errors);
    assert_eq!(fs::read(file).unwrap(), b"source");
    assert!(!b.join("read-only.txt").exists());
    println!("PASS: read-only exFAT destination fails without source removal");
    let marker = b.join(".rust-explorer-test-volume");
    let original = fs::read(&marker).unwrap();
    let rows = rust_explorer::core::rename::preview(
        std::slice::from_ref(&marker),
        &rust_explorer::core::rename::RenamePattern::Add {
            prefix: "renamed-".into(),
            suffix: String::new(),
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert!(rows[0].error.is_some());
    assert_eq!(fs::read(marker).unwrap(), original);
    println!("PASS: batch preview rejects read-only exFAT without changing the source");
}

#[test]
#[ignore = "requires disposable APFS and exFAT images; scripts/test-volumes.py"]
fn finder_volume_scenarios() {
    use rust_explorer::core::{
        model::Fingerprint,
        rename::{self, RenamePattern},
    };
    let (a, b) = roots();
    let db = tempfile::tempdir().unwrap();
    let store = Store::open(&db.path().join("state.sqlite")).unwrap();
    for root in [&a, &b] {
        let dir = tempfile::tempdir_in(root).unwrap();
        let sources: Vec<_> = ["한글 😀.txt", "archive.tar.gz"]
            .map(|name| dir.path().join(name))
            .into();
        for (i, path) in sources.iter().enumerate() {
            fs::write(path, format!("original {i}")).unwrap();
        }
        let rows = rename::preview(
            &sources,
            &RenamePattern::Number {
                base: "자료".into(),
                start: 1,
                digits: 2,
            },
            &Cancellation::default(),
        )
        .unwrap();
        assert!(rows.iter().all(|row| row.error.is_none()), "{rows:?}");
        let request = Operation {
            kind: OperationKind::BatchRename,
            sources: sources.iter().map(|p| StoredPath::new(p)).collect(),
            renames: rows.iter().map(|r| r.spec.clone().unwrap()).collect(),
            destination: None,
            name: None,
            completed_sources: vec![],
        };
        let mut journal = execute(
            request,
            &store,
            &Cancellation::default(),
            |_| panic!("no overwrite"),
            |_| {},
        );
        assert_eq!(journal.status, "complete", "{:?}", journal.errors);
        assert_eq!(
            fs::read(dir.path().join("자료 01.txt")).unwrap(),
            b"original 0"
        );
        assert_eq!(
            fs::read(dir.path().join("자료 02.tar.gz")).unwrap(),
            b"original 1"
        );
        undo(&mut journal, &store, &Cancellation::default()).unwrap();
        assert!(sources.iter().all(|p| p.exists()));
        let copies = execute(
            Operation::transfer(
                OperationKind::Duplicate,
                sources.clone(),
                dir.path().to_owned(),
            ),
            &store,
            &Cancellation::default(),
            |_| panic!(),
            |_| {},
        );
        assert_eq!(copies.status, "complete", "{:?}", copies.errors);
        assert_eq!(
            fs::read(dir.path().join("한글 😀 (2).txt")).unwrap(),
            b"original 0"
        );
        assert_eq!(fs::read(&sources[0]).unwrap(), b"original 0");
        let marker = root.join(".rust-explorer-test-volume");
        let before = Fingerprint::read(&marker).unwrap();
        let guarded = run(
            &store,
            OperationKind::Move,
            root,
            dir.path(),
            ConflictChoice::Cancel,
        );
        assert_eq!(guarded.status, "failed");
        assert!(before.matches(&marker));
        let rows = rename::preview(
            std::slice::from_ref(root),
            &RenamePattern::Add {
                prefix: "new-".into(),
                suffix: String::new(),
            },
            &Cancellation::default(),
        )
        .unwrap();
        assert!(rows[0].error.is_some());
        println!(
            "PASS: {} numbered rename/undo, duplicate, mount-root move and rename guard",
            root.display()
        );
    }
}

#[test]
#[ignore = "requires detached exFAT fixture"]
fn disconnected_destination() {
    let a = PathBuf::from(std::env::var_os("RUST_EXPLORER_TEST_APFS").unwrap());
    let b = PathBuf::from(std::env::var_os("RUST_EXPLORER_TEST_EXFAT").unwrap());
    assert!(a.join(".rust-explorer-test-volume").exists());
    assert!(!b.join(".rust-explorer-test-volume").exists());
    let src = tempfile::tempdir_in(a).unwrap();
    let db = tempfile::tempdir().unwrap();
    let store = Store::open(&db.path().join("state.sqlite")).unwrap();
    let file = src.path().join("disconnect.txt");
    fs::write(&file, b"source").unwrap();
    let j = run(
        &store,
        OperationKind::Move,
        &file,
        &b.join("destination"),
        ConflictChoice::Cancel,
    );
    assert_eq!(j.status, "failed");
    assert_eq!(fs::read(file).unwrap(), b"source");
    println!("PASS: disconnected target fails without source removal");
}

#[test]
#[ignore = "requires scripts/test-interruption.py disposable image"]
fn disconnect_during_move() {
    let source = PathBuf::from(std::env::var_os("RUST_EXPLORER_INTERRUPTION_SOURCE").unwrap());
    let mount = PathBuf::from(std::env::var_os("RUST_EXPLORER_INTERRUPTION_MOUNT").unwrap());
    assert_eq!(mount.file_name().unwrap(), "re-fault");
    assert!(mount.join(".rust-explorer-test-volume").exists());
    assert!(
        source
            .parent()
            .unwrap()
            .join(".rust-explorer-interruption-source")
            .exists()
    );
    let original = fs::metadata(&source).unwrap().len();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("state.sqlite")).unwrap();
    let mut detach = None;
    let mut started_at_bytes = 0;
    let journal = execute(
        Operation::transfer(OperationKind::Move, vec![source.clone()], mount.clone()),
        &store,
        &Cancellation::default(),
        |_| ConflictDecision {
            choice: ConflictChoice::Cancel,
            apply_all: false,
        },
        |progress| {
            if detach.is_none() && progress.current > 0 && progress.current < original {
                started_at_bytes = progress.current;
                let mount = mount.clone();
                detach = Some(std::thread::spawn(move || {
                    std::process::Command::new("hdiutil")
                        .args(["detach", "-force"])
                        .arg(mount)
                        .output()
                        .unwrap()
                }));
            }
        },
    );
    let result = detach
        .expect("must disconnect during active transfer")
        .join()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!mount.exists());
    assert_ne!(
        journal.status, "complete",
        "must not finish before disconnection"
    );
    assert_eq!(fs::metadata(&source).unwrap().len(), original);
    println!(
        "PASS: detached at {started_at_bytes}/{original} bytes; move {} and source preserved; {:?}",
        journal.status, journal.errors
    );
}
