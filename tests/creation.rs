use rust_explorer::core::{
    archive::{self, ArchiveFormat},
    operations::*,
    store::Store,
    worker::Cancellation,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use unicode_normalization::UnicodeNormalization;

fn request(kind: OperationKind, sources: &[PathBuf], destination: &Path, name: &str) -> Operation {
    Operation {
        completed_sources: vec![],
        renames: vec![],
        kind,
        sources: sources.iter().map(|p| StoredPath::new(p)).collect(),
        destination: Some(StoredPath::new(destination)),
        name: Some(name.as_bytes().to_vec()),
    }
}
fn run(operation: Operation, store: &Store) -> Journal {
    execute(
        operation,
        store,
        &Cancellation::default(),
        |_| panic!("new items must not overwrite"),
        |_| {},
    )
}
fn created(journal: &Journal) -> PathBuf {
    assert_eq!(journal.status, "complete", "{:?}", journal.errors);
    match journal.undo.last().unwrap() {
        UndoStep::Created { path, .. } => path.path(),
        _ => panic!("creation must be undoable"),
    }
}
fn no_stages(folder: &Path) {
    assert!(fs::read_dir(folder).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".RustExplorer-stage-")
    }));
}

#[test]
fn new_document_templates_produce_usable_files_and_custom_names_stay_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    for template in FileTemplate::PRESETS {
        let kind = OperationKind::CreateFile(template);
        let make = Operation::transfer(kind, vec![], dir.path().into());
        let first = created(&run(make.clone(), &store));
        let next = created(&run(make, &store));
        assert_ne!(first, next);
        assert_eq!(first.extension(), next.extension());
        match template {
            FileTemplate::Json => {
                let value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&first).unwrap()).unwrap();
                assert_eq!(value, serde_json::json!({}));
            }
            FileTemplate::RichText => {
                let plain = Command::new("/usr/bin/textutil")
                    .args(["-convert", "txt", "-stdout"])
                    .arg(&first)
                    .output()
                    .unwrap();
                assert!(plain.status.success());
                assert!(String::from_utf8(plain.stdout).unwrap().trim().is_empty());
            }
            _ => assert!(fs::read(&first).unwrap().is_empty()),
        }
    }
    let kind = OperationKind::CreateFile(FileTemplate::Empty);
    let empty = created(&run(
        request(kind, &[], dir.path(), "직접 입력.json"),
        &store,
    ));
    assert!(fs::read(empty).unwrap().is_empty());
    let refused = run(request(kind, &[], dir.path(), "../escape.txt"), &store);
    assert_eq!(refused.status, "failed");
    assert!(refused.undo.is_empty());
    no_stages(dir.path());
}

#[test]
fn default_folders_number_collisions_but_an_explicit_name_is_respected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    fs::write(dir.path().join("새 폴더"), b"existing file").unwrap();
    let make = Operation::transfer(OperationKind::CreateFolder, vec![], dir.path().into());
    let folder = created(&run(make, &store));
    assert_eq!(folder.file_name().unwrap(), "새 폴더 (2)");
    assert!(folder.is_dir());
    let named = run(
        request(OperationKind::CreateFolder, &[], dir.path(), "새 폴더"),
        &store,
    );
    assert_eq!(named.status, "failed");
    assert_eq!(
        fs::read(dir.path().join("새 폴더")).unwrap(),
        b"existing file"
    );
}

#[test]
fn new_text_documents_are_empty_and_collisions_are_safe_even_concurrently() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    fs::write(dir.path().join("새 텍스트 문서.txt"), "기존 내용").unwrap();
    let journals = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    run(
                        request(
                            OperationKind::CreateTextFile,
                            &[],
                            dir.path(),
                            "새 텍스트 문서.txt",
                        ),
                        &store,
                    )
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut outputs: Vec<_> = journals.iter().map(created).collect();
    outputs.sort();
    outputs.dedup();
    assert_eq!(outputs.len(), 8);
    for output in outputs {
        assert!(fs::read(output).unwrap().is_empty());
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("새 텍스트 문서.txt")).unwrap(),
        "기존 내용"
    );
    assert!(journals.iter().all(Journal::can_undo));
    no_stages(dir.path());
}

#[test]
fn zip_and_7z_round_trip_unicode_empty_directories_links_and_multiple_roots() {
    for (kind, extension, magic) in [
        (OperationKind::CompressZip, "zip", b"PK\x03\x04".as_slice()),
        (
            OperationKind::Compress7z,
            "7z",
            b"7z\xbc\xaf\x27\x1c".as_slice(),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state/db")).unwrap();
        let root = dir.path().join("자료 📁");
        fs::create_dir_all(root.join("빈 폴더")).unwrap();
        fs::write(root.join("한글 🦀.txt"), "원본 유지\n").unwrap();
        std::os::unix::fs::symlink("한글 🦀.txt", root.join("링크")).unwrap();
        let second = dir.path().join("추가.txt");
        fs::write(&second, b"extra").unwrap();
        let name = format!("묶음.{extension}");
        fs::write(dir.path().join(&name), b"keep existing").unwrap();
        let journal = run(
            request(kind, &[root.clone(), second.clone()], dir.path(), &name),
            &store,
        );
        let output = created(&journal);
        assert_eq!(
            output.file_name().unwrap(),
            format!("묶음 (2).{extension}").as_str()
        );
        assert!(fs::read(&output).unwrap().starts_with(magic));
        assert!(
            fs::metadata(&output).unwrap().len() < 8192,
            "small archives must not be padded to 10 KiB"
        );
        if kind == OperationKind::CompressZip {
            let bytes = fs::read(&output).unwrap();
            // Inspect raw ZIP headers as well: the reader on macOS exposes
            // decomposed names, which must not hide changes in the writer.
            let name = "자료 📁/한글 🦀.txt".as_bytes();
            assert!(bytes.windows(name.len()).any(|part| part == name));
        }
        assert_eq!(fs::read(dir.path().join(name)).unwrap(), b"keep existing");
        let extracted = run(
            Operation {
                completed_sources: vec![],
                renames: vec![],
                kind: OperationKind::Extract,
                sources: vec![StoredPath::new(&output)],
                destination: None,
                name: None,
            },
            &store,
        );
        let destination = created(&extracted);
        assert_eq!(
            fs::read_to_string(destination.join("자료 📁/한글 🦀.txt")).unwrap(),
            "원본 유지\n"
        );
        assert!(destination.join("자료 📁/빈 폴더").is_dir());
        assert!(
            fs::symlink_metadata(destination.join("자료 📁/링크"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(destination.join("자료 📁/링크"))
                .unwrap()
                .to_str()
                .unwrap()
                .nfc()
                .collect::<String>(),
            fs::read_link(root.join("링크"))
                .unwrap()
                .to_str()
                .unwrap()
                .nfc()
                .collect::<String>()
        );
        assert_eq!(
            fs::read(destination.join("자료 📁/링크")).unwrap(),
            "원본 유지\n".as_bytes()
        );
        assert_eq!(fs::read(destination.join("추가.txt")).unwrap(), b"extra");
        assert_eq!(
            fs::read_to_string(root.join("한글 🦀.txt")).unwrap(),
            "원본 유지\n"
        );
        assert_eq!(fs::read(second).unwrap(), b"extra");
        // A separate system command must also understand the emitted archive.
        let independent = dir.path().join("bsdtar-output");
        fs::create_dir(&independent).unwrap();
        let status = Command::new("/usr/bin/bsdtar")
            .arg("-xf")
            .arg(&output)
            .arg("-C")
            .arg(&independent)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        assert_eq!(
            fs::read(independent.join("자료 📁/한글 🦀.txt")).unwrap(),
            "원본 유지\n".as_bytes()
        );
        no_stages(dir.path());
    }
}

#[test]
fn extracts_a_system_generated_7z_and_rejects_a_truncated_7z_transactionally() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    fs::write(dir.path().join("외부 🦀.txt"), b"external archive").unwrap();
    let source = dir.path().join("system.7z");
    let status = Command::new("/usr/bin/bsdtar")
        .arg("--format=7zip")
        .arg("-cf")
        .arg(&source)
        .arg("-C")
        .arg(dir.path())
        .arg("외부 🦀.txt")
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let operation = Operation {
        completed_sources: vec![],
        renames: vec![],
        kind: OperationKind::Extract,
        sources: vec![StoredPath::new(&source)],
        destination: None,
        name: None,
    };
    let extracted = created(&run(operation.clone(), &store));
    assert_eq!(
        fs::read(extracted.join("외부 🦀.txt")).unwrap(),
        b"external archive"
    );
    let data = fs::read(&source).unwrap();
    fs::write(&source, &data[..data.len() - 20]).unwrap();
    let failed = run(operation, &store);
    assert_eq!(failed.status, "failed");
    assert!(!dir.path().join("system (2)").exists());
    no_stages(dir.path());
}

#[test]
fn cancelled_compression_cleans_stage_and_preserves_sources() {
    for kind in [OperationKind::CompressZip, OperationKind::Compress7z] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state/db")).unwrap();
        let source = dir.path().join("large.txt");
        let bytes = vec![b'z'; 2 * 1024 * 1024];
        fs::write(&source, &bytes).unwrap();
        let cancel = Cancellation::default();
        let journal = execute(
            request(
                kind,
                std::slice::from_ref(&source),
                dir.path(),
                "cancelled.archive",
            ),
            &store,
            &cancel,
            |_| panic!(),
            |_| cancel.cancel(),
        );
        assert_eq!(journal.status, "cancelled", "{:?}", journal.errors);
        assert!(!dir.path().join("cancelled.archive").exists());
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert!(journal.can_retry());
        no_stages(dir.path());
        let format = if kind == OperationKind::CompressZip {
            ArchiveFormat::Zip
        } else {
            ArchiveFormat::SevenZip
        };
        let cancel = Cancellation::default();
        let result = archive::create(
            std::slice::from_ref(&source),
            &dir.path().join("direct-stage"),
            format,
            &cancel,
            &mut |_, bytes, _| {
                if bytes > 0 {
                    cancel.cancel();
                }
            },
        );
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    }
}

#[test]
fn creation_refuses_self_inclusion_external_links_and_source_changes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let source = dir.path().join("folder");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), vec![b'a'; 512 * 1024]).unwrap();
    let failed = run(
        request(
            OperationKind::CompressZip,
            std::slice::from_ref(&source),
            &source,
            "self.zip",
        ),
        &store,
    );
    assert_eq!(failed.status, "failed");
    assert!(!source.join("self.zip").exists());
    let link = source.join("escape");
    std::os::unix::fs::symlink("../../outside", &link).unwrap();
    let failed = run(
        request(
            OperationKind::Compress7z,
            std::slice::from_ref(&source),
            dir.path(),
            "outside.7z",
        ),
        &store,
    );
    assert_eq!(failed.status, "failed");
    assert!(!dir.path().join("outside.7z").exists());
    fs::remove_file(link).unwrap();
    let file = source.join("file");
    let stage = dir.path().join("stage.zip");
    let mut changed = false;
    let result = archive::create(
        std::slice::from_ref(&file),
        &stage,
        ArchiveFormat::Zip,
        &Cancellation::default(),
        &mut |_, bytes, _| {
            if bytes > 0 && !changed {
                changed = true;
                fs::write(&file, b"new content").unwrap();
            }
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(file).unwrap(), b"new content");
    no_stages(&source);
    no_stages(dir.path());
}

#[test]
fn created_files_can_be_undone_but_edited_outputs_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    for kind in [
        OperationKind::CreateTextFile,
        OperationKind::CompressZip,
        OperationKind::Compress7z,
    ] {
        let source = dir.path().join("source.txt");
        fs::write(&source, b"original").unwrap();
        let mut journal = run(
            request(kind, std::slice::from_ref(&source), dir.path(), "output"),
            &store,
        );
        let output = created(&journal);
        fs::write(&output, b"user edited").unwrap();
        assert!(undo(&mut journal, &store, &Cancellation::default()).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"user edited");
        let mut untouched = run(
            request(kind, std::slice::from_ref(&source), dir.path(), "untouched"),
            &store,
        );
        let path = created(&untouched);
        undo(&mut untouched, &store, &Cancellation::default()).unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read(&source).unwrap(), b"original");
    }
}

#[test]
fn compression_keeps_quarantine_and_zip_keeps_decomposed_name_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let name = "한글.txt";
    let source = dir.path().join(name);
    fs::write(&source, b"generated downloaded-file fixture").unwrap();
    let quarantine = b"0081;00000000;RustExplorerTest;00000000-0000-0000-0000-000000000000";
    rust_explorer::platform::set_quarantine_attribute(&source, quarantine).unwrap();
    for kind in [OperationKind::CompressZip, OperationKind::Compress7z] {
        let journal = run(
            request(
                kind,
                std::slice::from_ref(&source),
                dir.path(),
                "downloaded.archive",
            ),
            &store,
        );
        let output = created(&journal);
        assert_eq!(
            rust_explorer::platform::quarantine_attribute(&output)
                .unwrap()
                .as_deref(),
            Some(quarantine.as_slice())
        );
        if kind == OperationKind::CompressZip {
            assert!(
                fs::read(output)
                    .unwrap()
                    .windows(name.len())
                    .any(|bytes| bytes == name.as_bytes())
            );
        }
    }
}

#[test]
fn compression_rejects_link_chains_that_would_escape_its_root() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("a");
    let second = root.path().join("escape");
    symlink(".", &first).unwrap();
    symlink("a/../outside", &second).unwrap();
    let store = Store::open(&root.path().join("state/db")).unwrap();
    let request = Operation {
        completed_sources: vec![],
        renames: vec![],
        kind: OperationKind::CompressZip,
        sources: vec![StoredPath::new(&first), StoredPath::new(&second)],
        destination: Some(StoredPath::new(root.path())),
        name: Some(b"unsafe.zip".to_vec()),
    };
    let result = run(request, &store);
    assert_eq!(result.status, "failed");
    assert!(!root.path().join("unsafe.zip").exists());
}
