use rust_explorer::core::{
    model::{Preferences, path_bytes},
    shortcuts::PinGroup,
    store::Store,
};
use rust_explorer::platform::server::server_url;
use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

#[test]
fn pins_keep_groups_order_labels_and_native_paths_across_relaunch() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let path = home.join(OsString::from_vec(vec![b'n', 0xff]));
    let other = home.join("프로젝트");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("원본"), b"original").unwrap();
    let mut prefs: Preferences =
        serde_json::from_value(serde_json::json!({"favorites":[path_bytes(&other)]})).unwrap();
    prefs.sidebar.collapsed = vec!["favorites".into(), "locations".into()];
    prefs.pin_folders(PinGroup::Locations, &[path.clone(), other.clone()], home);
    prefs.pin_folders(PinGroup::Favorites, &[path.clone(), path.clone()], home);
    let locations = prefs.shortcuts(PinGroup::Locations, home);
    let key = locations[0].key.clone();
    prefs.shortcut_labels.insert(key.clone(), "내 자료".into());
    prefs.sidebar.order = vec![locations[1].key.clone(), key];
    let db = home.join("state.sqlite");
    Store::open(&db)
        .unwrap()
        .set("preferences", &prefs)
        .unwrap();
    let mut restored = Store::open(&db).unwrap().preferences();
    let locations = restored.shortcuts(PinGroup::Locations, home);
    assert_eq!(
        (locations[1].path.as_path(), locations[1].label.as_str()),
        (path.as_path(), "내 자료")
    );
    assert_eq!(restored.favorites.len(), 2);
    assert!(restored.sidebar.collapsed.is_empty());
    restored.unpin_folder(PinGroup::Locations, &path);
    assert_eq!(restored.pinned_locations, vec![path_bytes(&other)]);
    assert!(restored.favorites.contains(&path_bytes(&path)));
    assert!(restored.shortcut_labels.is_empty());
    restored.unpin_folder(PinGroup::Favorites, &other);
    restored.unpin_folder(PinGroup::Locations, &other);
    assert_eq!(std::fs::read(other.join("원본")).unwrap(), b"original");
}

#[test]
fn repinning_a_builtin_unhides_it_and_locations_are_independent() {
    let home = Path::new("/test-home");
    let downloads = home.join("Downloads");
    let volume = PathBuf::from("/Volumes/공유");
    let mut prefs = Preferences::default();
    prefs.sidebar.hidden.push("downloads".into());
    prefs.pin_folders(PinGroup::Favorites, &[downloads, volume.clone()], home);
    prefs.pin_folders(PinGroup::Locations, std::slice::from_ref(&volume), home);
    assert!(prefs.sidebar.visible("downloads"));
    assert_eq!(prefs.favorites.len(), 1);
    assert_eq!(prefs.pinned_locations.len(), 1);
    assert_ne!(
        prefs
            .shortcuts(PinGroup::Favorites, home)
            .last()
            .unwrap()
            .key,
        prefs.shortcuts(PinGroup::Locations, home)[0].key
    );
}

#[test]
fn relocating_an_offline_shortcut_preserves_its_label_and_order_only_in_that_group() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("분리된 폴더");
    let new = temp.path().join("다시 연결한 폴더");
    let other = temp.path().join("다른 항목");
    let mut prefs = Preferences::default();
    prefs.pin_folders(PinGroup::Favorites, std::slice::from_ref(&old), temp.path());
    prefs.pin_folders(
        PinGroup::Locations,
        &[other.clone(), old.clone()],
        temp.path(),
    );
    let old_key = prefs.shortcuts(PinGroup::Locations, temp.path())[1]
        .key
        .clone();
    prefs.shortcut_labels.insert(old_key.clone(), "팀".into());
    prefs.sidebar.order.push(old_key.clone());
    assert!(
        prefs
            .retarget_folder(PinGroup::Locations, &old, &other, temp.path())
            .is_err()
    );
    prefs
        .retarget_folder(PinGroup::Locations, &old, &new, temp.path())
        .unwrap();
    let shortcuts = prefs.shortcuts(PinGroup::Locations, temp.path());
    assert_eq!(
        (&shortcuts[0].path, shortcuts[0].label.as_str()),
        (&new, "팀")
    );
    assert!(!prefs.shortcut_labels.contains_key(&old_key));
    assert_eq!(prefs.favorites, vec![path_bytes(&old)]);
    assert!(!old.exists() && !new.exists());
}

#[test]
fn server_addresses_are_validated_without_contacting_the_network() {
    for (input, scheme, host) in [
        (r"\\nas\공유 폴더", "smb", "nas"),
        ("smb://server.example:445/share", "smb", "server.example"),
        ("nas/share", "smb", "nas"),
        ("nfs://server/exports", "nfs", "server"),
        ("https://host/dav", "https", "host"),
    ] {
        let url = server_url(input).unwrap();
        assert_eq!(url.scheme().unwrap().to_string(), scheme);
        assert_eq!(url.host().unwrap().to_string(), host);
        assert!(url.user().is_none() && url.password().is_none());
    }
    let unc = server_url(r"\\nas\a#b%20").unwrap();
    assert!(
        unc.absoluteString()
            .unwrap()
            .to_string()
            .ends_with("/a%23b%2520")
    );
    for input in [
        "",
        "file:///tmp",
        "/tmp",
        "smb:///share",
        "ssh://server/home",
        "smb://user:pass@host/share",
        "smb://host:65536/share",
        "https://host/share?token=secret",
        "smb://host/share#x",
        "smb://bad host/x",
        "C:/Windows",
        "smb://nas:/share",
        "smb://nas:bad/share",
        "smb://../path",
    ] {
        assert!(server_url(input).is_err(), "accepted {input}");
    }
    assert!(rust_explorer::platform::server::is_server_address(
        r"\\nas\folder"
    ));
    assert!(rust_explorer::platform::server::is_server_address(
        "SMB://nas"
    ));
    assert!(!rust_explorer::platform::server::is_server_address(
        "~/Documents"
    ));
    assert!(!rust_explorer::platform::server::is_server_address(
        "relative-dir"
    ));
}
