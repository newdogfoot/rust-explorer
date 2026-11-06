use rust_explorer::core::{archive, operations::*, store::Store, worker::Cancellation};
use std::{fs, io::Write, path::Path};

fn tar(path: &Path, items: &[(&str, u8, &str, &[u8])]) {
    let mut file = fs::File::create(path).unwrap();
    for (name, kind, link, data) in items {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000755\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[136..148].copy_from_slice(b"14500000000\0");
        header[148..156].fill(b' ');
        header[156] = *kind;
        header[157..157 + link.len()].copy_from_slice(link.as_bytes());
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u32 = header.iter().map(|b| *b as u32).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        file.write_all(&header).unwrap();
        file.write_all(data).unwrap();
        file.write_all(&vec![0; (512 - data.len() % 512) % 512])
            .unwrap();
    }
    file.write_all(&[0; 1024]).unwrap();
}
fn execute_archive(path: &Path, store: &Store, cancel: &Cancellation) -> Journal {
    execute(
        Operation {
            completed_sources: vec![],
            renames: vec![],
            kind: OperationKind::Extract,
            sources: vec![StoredPath::new(path)],
            destination: None,
            name: None,
        },
        store,
        cancel,
        |_| panic!("extraction must never replace an existing destination"),
        |_| {},
    )
}
#[test]
fn extracts_unicode_and_contained_links_without_changing_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("한글 자료.tar");
    tar(
        &source,
        &[
            ("문서/", b'5', "", b""),
            ("문서/안녕.txt", b'0', "", b"hello"),
            ("link", b'2', "문서/안녕.txt", b""),
        ],
    );
    let original = fs::read(&source).unwrap();
    fs::create_dir(dir.path().join("한글 자료")).unwrap();
    fs::write(dir.path().join("한글 자료/keep"), b"unchanged").unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let result = execute_archive(&source, &store, &Cancellation::default());
    assert_eq!(result.status, "complete", "{:?}", result.errors);
    assert_eq!(
        fs::read(dir.path().join("한글 자료 (2)/문서/안녕.txt")).unwrap(),
        b"hello"
    );
    assert_eq!(
        fs::read(dir.path().join("한글 자료 (2)/link")).unwrap(),
        b"hello"
    );
    assert_eq!(
        fs::read(dir.path().join("한글 자료/keep")).unwrap(),
        b"unchanged"
    );
    assert_eq!(fs::read(source).unwrap(), original);
    assert!(result.can_undo());
}
#[test]
fn traversal_links_and_duplicate_entries_never_escape_or_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    for items in [
        vec![("../escape", b'0', "", b"x".as_slice())],
        vec![("/absolute", b'0', "", b"x".as_slice())],
        vec![("link", b'2', "../escape", b"".as_slice())],
        vec![
            ("same", b'0', "", b"a".as_slice()),
            ("same", b'0', "", b"b".as_slice()),
        ],
        vec![
            ("dir", b'2', "inside", b"".as_slice()),
            ("dir/file", b'0', "", b"x".as_slice()),
        ],
    ] {
        let source = dir.path().join("malicious.tar");
        tar(&source, &items);
        let result = execute_archive(&source, &store, &Cancellation::default());
        assert_eq!(result.status, "failed", "{:?}", result.errors);
        assert!(!dir.path().join("malicious").exists());
        assert!(!dir.path().join("escape").exists());
        assert!(!fs::read_dir(dir.path()).unwrap().flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".RustExplorer-stage-")
        }));
    }
}
#[test]
fn cancellation_interrupts_streaming_and_preserves_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("large.tar");
    let data = vec![5; 2 * 1024 * 1024];
    tar(&source, &[("large.bin", b'0', "", &data)]);
    let stage = dir.path().join("stage");
    fs::create_dir(&stage).unwrap();
    let cancel = Cancellation::default();
    let result = archive::extract(&source, &stage, &cancel, &mut |_, bytes, _| {
        if bytes > 0 {
            cancel.cancel();
        }
    });
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    assert!(source.exists());
    assert!(fs::metadata(stage.join("large.bin")).unwrap().len() < data.len() as u64);
}

#[test]
fn zip_unicode_roundtrip_and_crc_failure_are_transactional() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let source = dir.path().join("unicode.zip");
    let bytes = include_bytes!("fixtures/unicode.zip");
    fs::write(&source, bytes).unwrap();
    let journal = execute_archive(&source, &store, &Cancellation::default());
    assert_eq!(journal.status, "complete", "{:?}", journal.errors);
    assert_eq!(
        fs::read_to_string(dir.path().join("unicode/문서/한글😀.txt")).unwrap(),
        "한글 압축 fixture\n".repeat(64)
    );
    let mut corrupt = bytes.to_vec();
    let compressed = u32::from_le_bytes(bytes[18..22].try_into().unwrap()) as usize;
    let offset = 30
        + u16::from_le_bytes(bytes[26..28].try_into().unwrap()) as usize
        + u16::from_le_bytes(bytes[28..30].try_into().unwrap()) as usize;
    corrupt[offset + compressed / 2] ^= 0xff;
    let damaged = dir.path().join("damaged.zip");
    fs::write(&damaged, &corrupt).unwrap();
    let journal = execute_archive(&damaged, &store, &Cancellation::default());
    assert_eq!(journal.status, "failed", "{:?}", journal.errors);
    assert!(!dir.path().join("damaged").exists());
    assert_eq!(fs::read(damaged).unwrap(), corrupt);
    assert!(!fs::read_dir(dir.path()).unwrap().flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with(".RustExplorer-stage-")
    }));
}
#[test]
fn password_protected_archive_reports_failure_without_output() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let path = dir.path().join("encrypted.zip");
    fs::write(&path, include_bytes!("fixtures/encrypted.zip")).unwrap();
    let result = execute_archive(&path, &store, &Cancellation::default());
    assert_eq!(result.status, "failed");
    assert!(
        result.errors.iter().any(|s| s.contains("암호")),
        "{:?}",
        result.errors
    );
    assert!(!dir.path().join("encrypted").exists());
}

#[test]
fn downloaded_archive_keeps_quarantine_on_extracted_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("download.zip");
    fs::write(&source, include_bytes!("fixtures/unicode.zip")).unwrap();
    let value = b"0081;00000000;Rust Explorer QA;";
    rust_explorer::platform::set_quarantine_attribute(&source, value).unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let result = execute_archive(&source, &store, &Cancellation::default());
    assert_eq!(result.status, "complete", "{:?}", result.errors);
    for path in [
        dir.path().join("download"),
        dir.path().join("download/문서/한글😀.txt"),
    ] {
        assert_eq!(
            rust_explorer::platform::quarantine_attribute(&path).unwrap(),
            Some(value.to_vec())
        );
    }
}

#[test]
fn symlink_chains_must_not_escape_after_expanding_an_earlier_link() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.txt");
    fs::write(&outside, b"keep private").unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let source = dir.path().join("chain.tar");
    // Each string is lexically contained, but expanding a -> . makes a/..
    // escape the extraction root. No victim outside this fixture is touched.
    tar(
        &source,
        &[
            ("a", b'2', ".", b""),
            ("escape", b'2', "a/../outside.txt", b""),
        ],
    );
    let result = execute_archive(&source, &store, &Cancellation::default());
    assert_eq!(
        result.status, "failed",
        "escaped chain was accepted: {:?}",
        result.errors
    );
    assert_eq!(fs::read(&outside).unwrap(), b"keep private");
    assert!(!dir.path().join("chain").exists());
}

#[test]
fn contained_link_chains_and_dangling_links_are_preserved_but_cycles_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state/db")).unwrap();
    let good = dir.path().join("good-chain.tar");
    tar(
        &good,
        &[
            ("data/file", b'0', "", b"ok"),
            ("alias", b'2', "data", b""),
            ("link", b'2', "alias/file", b""),
            ("dangling", b'2', "not-yet-created", b""),
        ],
    );
    let result = execute_archive(&good, &store, &Cancellation::default());
    assert_eq!(result.status, "complete", "{:?}", result.errors);
    assert_eq!(fs::read(dir.path().join("good-chain/link")).unwrap(), b"ok");
    assert!(
        fs::symlink_metadata(dir.path().join("good-chain/dangling"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let bad = dir.path().join("cycle.tar");
    tar(&bad, &[("a", b'2', "b", b""), ("b", b'2', "a", b"")]);
    let result = execute_archive(&bad, &store, &Cancellation::default());
    assert_eq!(result.status, "failed");
    assert!(!dir.path().join("cycle").exists());
}
