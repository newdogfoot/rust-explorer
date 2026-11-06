use objc2_foundation::{NSString, NSURL};
use rust_explorer::{core::store::Store, platform};
use std::{ffi::OsString, fs, os::unix::fs::MetadataExt};

#[test]
fn file_drop_urls_reject_nuls_remote_authorities_credentials_and_non_file_schemes() {
    for value in [
        "https://example.invalid/file",
        "file://remote.invalid/tmp/file",
        "file://user@localhost/tmp/file",
        "file:///tmp/name%00suffix",
        "file:///tmp/name?query=1",
        "file:///tmp/name#fragment",
        "javascript:alert(1)",
    ] {
        if let Some(url) = NSURL::URLWithString(&NSString::from_str(value)) {
            assert!(
                platform::checked_file_url_path(&url).is_none(),
                "accepted {value}"
            );
        }
    }
    let url =
        NSURL::URLWithString(&NSString::from_str("file://localhost/tmp/name%2500.txt")).unwrap();
    assert_eq!(
        platform::checked_file_url_path(&url)
            .unwrap()
            .to_str()
            .unwrap(),
        "/tmp/name%00.txt"
    );
}

#[test]
fn native_file_urls_preserve_unicode_and_special_filename_identity() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        OsString::from("한글😀 #?%.txt"),
        OsString::from("literal%00.txt"),
    ] {
        let path = dir.path().join(name);
        fs::write(&path, b"private fixture").unwrap();
        let url = platform::file_url(&path);
        let decoded = platform::checked_file_url_path(&url).expect("native file URL");
        assert_eq!(fs::read(&decoded).unwrap(), b"private fixture");
        assert_eq!(
            fs::metadata(&decoded).unwrap().ino(),
            fs::metadata(&path).unwrap().ino()
        );
    }
}

#[test]
fn settings_and_journals_are_private_and_a_database_symlink_is_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("state.sqlite");
    let store = Store::open(&database).unwrap();
    store.set("private-path", &"/private/example").unwrap();
    assert_eq!(
        fs::metadata(&database).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for suffix in ["-wal", "-shm"] {
        let sidecar = dir.path().join(format!("state.sqlite{suffix}"));
        if sidecar.exists() {
            assert_eq!(
                fs::metadata(sidecar).unwrap().permissions().mode() & 0o077,
                0
            );
        }
    }
    drop(store);
    let victim = dir.path().join("untouched");
    fs::write(&victim, b"original").unwrap();
    let link = dir.path().join("linked.sqlite");
    symlink(&victim, &link).unwrap();
    assert!(Store::open(&link).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"original");
}
