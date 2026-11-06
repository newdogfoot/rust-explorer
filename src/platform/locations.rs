use super::*;
use objc2_foundation::*;

#[derive(Clone, Debug)]
pub struct Location {
    pub path: PathBuf,
    pub name: String,
    pub kind: LocationKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationKind {
    Internal,
    External,
    Network,
    Cloud,
}

// Called on the navigation worker, never from an AppKit drawing callback.
pub fn locations(home: &Path) -> Vec<Location> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let mut result = vec![];
        let keys = NSArray::from_slice(&[
            NSURLVolumeNameKey,
            NSURLVolumeIsInternalKey,
            NSURLVolumeIsLocalKey,
        ]);
        if let Some(urls) = NSFileManager::new()
            .mountedVolumeURLsIncludingResourceValuesForKeys_options(
                Some(&keys),
                NSVolumeEnumerationOptions::SkipHiddenVolumes,
            )
        {
            for url in urls.iter() {
                let path = url_path(&url);
                let mut name = None;
                let mut internal = None;
                let mut local = None;
                let _ = url.getResourceValue_forKey_error(&mut name, NSURLVolumeNameKey);
                let _ = url.getResourceValue_forKey_error(&mut internal, NSURLVolumeIsInternalKey);
                let _ = url.getResourceValue_forKey_error(&mut local, NSURLVolumeIsLocalKey);
                let internal = internal
                    .and_then(|n| n.downcast::<NSNumber>().ok())
                    .is_some_and(|n| n.boolValue());
                let local = local
                    .and_then(|n| n.downcast::<NSNumber>().ok())
                    .is_none_or(|n| n.boolValue());
                let name = name
                    .and_then(|n| n.downcast::<NSString>().ok())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    });
                result.push(Location {
                    path,
                    name,
                    kind: if !local {
                        LocationKind::Network
                    } else if internal {
                        LocationKind::Internal
                    } else {
                        LocationKind::External
                    },
                });
            }
        }
        if !result.iter().any(|l| l.path == Path::new("/")) {
            result.insert(
                0,
                Location {
                    path: "/".into(),
                    name: {
                        let mut value = None;
                        let _ = file_url(Path::new("/"))
                            .getResourceValue_forKey_error(&mut value, NSURLVolumeNameKey);
                        value
                            .and_then(|v| v.downcast::<NSString>().ok())
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "Mac 저장장치".into())
                    },
                    kind: LocationKind::Internal,
                },
            );
        }
        if let Ok(entries) = fs::read_dir(home.join("Library/CloudStorage")) {
            for entry in entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            {
                result.push(Location {
                    path: entry.path(),
                    name: entry.file_name().to_string_lossy().into_owned(),
                    kind: LocationKind::Cloud,
                });
            }
        }
        let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
        if icloud.is_dir() {
            result.push(Location {
                path: icloud,
                name: "iCloud Drive".into(),
                kind: LocationKind::Cloud,
            });
        }
        result.sort_by(|a, b| a.path.cmp(&b.path));
        result.dedup_by(|a, b| a.path == b.path);
        result
    })
}

pub fn volume_capacity(path: &Path) -> io::Result<(u64, u64)> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    Ok((
        (info.f_blocks as u64).saturating_mul(info.f_frsize),
        (info.f_bavail as u64).saturating_mul(info.f_frsize),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_volume_enumeration_always_contains_the_boot_drive() {
        let temp = tempfile::tempdir().unwrap();
        let volumes = locations(temp.path());
        let boot = volumes
            .iter()
            .find(|v| v.path == Path::new("/"))
            .expect("boot volume");
        assert_eq!(boot.kind, LocationKind::Internal);
        assert!(!boot.name.is_empty());
    }
    #[test]
    fn capacity_uses_filesystem_totals_and_reports_missing_mounts() {
        let (total, available) = volume_capacity(Path::new("/")).unwrap();
        assert!(total > 0 && available <= total);
        let temp = tempfile::tempdir().unwrap();
        assert!(volume_capacity(&temp.path().join("no-volume")).is_err());
    }
}
