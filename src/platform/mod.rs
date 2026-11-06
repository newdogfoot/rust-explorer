use crate::core::worker::Cancellation;
mod applications;
pub mod folder_access;
mod locations;
pub mod server;
pub use applications::{OpenApplication, applications_for_file};
pub use locations::{Location, LocationKind, locations, volume_capacity};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSFileManager, NSURL};
use std::{
    ffi::{CStr, CString},
    fs, io,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    ptr::NonNull,
};

pub fn volume_case_sensitive(path: &Path) -> bool {
    use objc2_foundation::{NSNumber, NSURLVolumeSupportsCaseSensitiveNamesKey};
    objc2::rc::autoreleasepool(|_| unsafe {
        let mut value = None;
        file_url(path)
            .getResourceValue_forKey_error(&mut value, NSURLVolumeSupportsCaseSensitiveNamesKey)
            .ok();
        value
            .and_then(|v| v.downcast::<NSNumber>().ok())
            .is_some_and(|v| v.boolValue())
    })
}
pub fn file_url(path: &Path) -> Retained<NSURL> {
    let c = CString::new(path.as_os_str().as_bytes()).expect("filesystem paths cannot contain NUL");
    unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
            NonNull::new(c.as_ptr().cast_mut()).unwrap(),
            false,
            None,
        )
    }
}
pub fn url_path(url: &NSURL) -> PathBuf {
    unsafe {
        crate::core::model::path_from_bytes(
            CStr::from_ptr(url.fileSystemRepresentation().as_ptr())
                .to_bytes()
                .to_vec(),
        )
    }
}
/// Validate untrusted pasteboard URLs before asking Foundation for a C path.
/// A decoded NUL must never silently truncate a path or become a null pointer.
pub fn checked_file_url_path(url: &NSURL) -> Option<PathBuf> {
    if !url.isFileURL()
        || url.user().is_some()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host().is_some_and(|host| {
            !host.is_empty() && !host.to_string().eq_ignore_ascii_case("localhost")
        })
    {
        return None;
    }
    let encoded = url.absoluteString()?.to_string();
    if encoded.len() > 131_072
        || encoded.contains("%00")
        || encoded.contains('\0')
        || url
            .path()
            .is_some_and(|path| path.to_string().contains('\0'))
    {
        return None;
    }
    let raw: *const libc::c_char = unsafe { objc2::msg_send![url, fileSystemRepresentation] };
    if raw.is_null() {
        return None;
    }
    let path =
        crate::core::model::path_from_bytes(unsafe { CStr::from_ptr(raw) }.to_bytes().to_vec());
    path.is_absolute().then_some(path)
}

pub fn quarantine_attribute(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let size = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"com.apple.quarantine".as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    if size < 0 {
        let error = io::Error::last_os_error();
        return if matches!(
            error.raw_os_error(),
            Some(libc::ENOATTR) | Some(libc::ENOTSUP)
        ) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let mut value = vec![0u8; size as usize];
    let read = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"com.apple.quarantine".as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    if read < 0 {
        return Err(io::Error::last_os_error());
    }
    value.truncate(read as usize);
    Ok(Some(value))
}
pub fn set_quarantine_attribute(path: &Path, value: &[u8]) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::setxattr(
            path.as_ptr(),
            c"com.apple.quarantine".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    if result < 0 {
        Err(io::Error::other(format!(
            "다운로드 파일 보호 정보를 유지할 수 없습니다: {}",
            io::Error::last_os_error()
        )))
    } else {
        Ok(())
    }
}
pub fn trash(path: &Path) -> io::Result<PathBuf> {
    objc2::rc::autoreleasepool(|_| {
        let mut result = None;
        NSFileManager::new()
            .trashItemAtURL_resultingItemURL_error(&file_url(path), Some(&mut result))
            .map_err(|e| io::Error::other(e.localizedDescription().to_string()))?;
        result
            .map(|u| url_path(&u))
            .ok_or_else(|| io::Error::other("휴지통 경로를 확인할 수 없습니다"))
    })
}
pub fn remove_item(path: &Path) -> io::Result<()> {
    objc2::rc::autoreleasepool(|_| {
        NSFileManager::new()
            .removeItemAtURL_error(&file_url(path))
            .map_err(|e| io::Error::other(e.localizedDescription().to_string()))
    })
}

unsafe extern "C" {
    fn copyfile(
        from: *const libc::c_char,
        to: *const libc::c_char,
        state: *mut libc::c_void,
        flags: u32,
    ) -> libc::c_int;
    fn copyfile_state_alloc() -> *mut libc::c_void;
    fn copyfile_state_free(state: *mut libc::c_void) -> libc::c_int;
    fn copyfile_state_set(
        state: *mut libc::c_void,
        flag: u32,
        value: *const libc::c_void,
    ) -> libc::c_int;
    fn copyfile_state_get(
        state: *mut libc::c_void,
        flag: u32,
        value: *mut libc::c_void,
    ) -> libc::c_int;
    fn renamex_np(from: *const libc::c_char, to: *const libc::c_char, flags: u32) -> libc::c_int;
}
struct CopyContext<'a> {
    cancel: &'a Cancellation,
    progress: &'a mut dyn FnMut(u64, u64),
    total: u64,
}
unsafe extern "C" fn copy_callback(
    what: i32,
    stage: i32,
    state: *mut libc::c_void,
    _from: *const libc::c_char,
    _to: *const libc::c_char,
    ctx: *mut libc::c_void,
) -> i32 {
    let ctx = unsafe { &mut *ctx.cast::<CopyContext<'_>>() };
    if ctx.cancel.cancelled() {
        return 2;
    }
    if stage == 3 {
        return 2;
    }
    if what == 4 {
        let mut copied = 0u64;
        unsafe {
            copyfile_state_get(state, 8, (&mut copied as *mut u64).cast());
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (ctx.progress)(copied, ctx.total)
        }))
        .is_err()
        {
            return 2;
        }
    }
    0
}

pub fn copy_node(
    source: &Path,
    target: &Path,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(u64, u64),
) -> io::Result<()> {
    cancel.check()?;
    let source_c = CString::new(source.as_os_str().as_bytes())?;
    let target_c = CString::new(target.as_os_str().as_bytes())?;
    let meta = fs::symlink_metadata(source)?;
    let mut context = CopyContext {
        cancel,
        progress,
        total: meta.len(),
    };
    unsafe {
        let state = copyfile_state_alloc();
        if state.is_null() {
            return Err(io::Error::other("파일 복사 상태를 할당할 수 없습니다"));
        }
        copyfile_state_set(state, 6, copy_callback as *const libc::c_void);
        copyfile_state_set(state, 7, (&mut context as *mut CopyContext<'_>).cast());
        // All metadata/data, exclusive destination, no symlink traversal, APFS clone when supported.
        let flags = 15 | (1 << 17) | (1 << 18) | (1 << 19) | (1 << 24);
        let result = copyfile(source_c.as_ptr(), target_c.as_ptr(), state, flags);
        let error = io::Error::last_os_error();
        copyfile_state_free(state);
        cancel.check()?;
        if result != 0 {
            return Err(io::Error::new(
                error.kind(),
                format!("파일 데이터·메타데이터 복사: {error}"),
            ));
        }
    }
    if !meta.file_type().is_symlink() {
        fs::File::open(target)?
            .sync_all()
            .map_err(|e| io::Error::new(e.kind(), format!("복사본 디스크 기록: {e}")))?;
    }
    (context.progress)(meta.len(), meta.len());
    Ok(())
}
pub fn copy_directory_metadata(source: &Path, target: &Path) -> io::Result<()> {
    let s = CString::new(source.as_os_str().as_bytes())?;
    let t = CString::new(target.as_os_str().as_bytes())?;
    let r = unsafe {
        copyfile(
            s.as_ptr(),
            t.as_ptr(),
            std::ptr::null_mut(),
            7 | (1 << 18) | (1 << 19),
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
pub fn rename_exclusive(source: &Path, target: &Path) -> io::Result<()> {
    let s = CString::new(source.as_os_str().as_bytes())?;
    let t = CString::new(target.as_os_str().as_bytes())?;
    if unsafe { renamex_np(s.as_ptr(), t.as_ptr(), 4) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    // RENAME_EXCL rejects the alternate spelling of the very same directory
    // entry on case-insensitive APFS. A real second hard link is NOT an alias.
    if case_alias(source, target) {
        return if unsafe { libc::rename(s.as_ptr(), t.as_ptr()) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
    }
    if error.kind() == io::ErrorKind::Unsupported || error.raw_os_error() == Some(libc::ENOTSUP) {
        // FSKit exFAT does not implement RENAME_EXCL. Foundation's move operation
        // has a no-overwrite contract and supplies the platform fallback.
        return objc2::rc::autoreleasepool(|_| {
            if fs::symlink_metadata(target).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "같은 이름의 항목이 이미 있습니다",
                ));
            }
            NSFileManager::new()
                .moveItemAtURL_toURL_error(&file_url(source), &file_url(target))
                .map_err(|e| io::Error::other(e.localizedDescription().to_string()))
        });
    }
    Err(io::Error::new(
        error.kind(),
        format!("파일 이름 확정: {error}"),
    ))
}
pub fn case_alias(source: &Path, target: &Path) -> bool {
    if source == target {
        return false;
    }
    let (Ok(a), Ok(b)) = (fs::symlink_metadata(source), fs::symlink_metadata(target)) else {
        return false;
    };
    if a.dev() != b.dev() || a.ino() != b.ino() {
        return false;
    }
    let (Some(ap), Some(bp)) = (source.parent(), target.parent()) else {
        return false;
    };
    if fs::canonicalize(ap).ok() != fs::canonicalize(bp).ok() {
        return false;
    }
    let Some(name) = target.file_name() else {
        return false;
    };
    fs::read_dir(bp)
        .is_ok_and(|mut entries| !entries.any(|e| e.is_ok_and(|e| e.file_name() == name)))
}
pub fn sync_parent(path: &Path) -> io::Result<()> {
    if let Some(p) = path.parent() {
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(p)
            .map_err(|e| io::Error::new(e.kind(), format!("상위 폴더 열기: {e}")))?
            .sync_all()
            .map_err(|e| io::Error::new(e.kind(), format!("폴더 변경 기록: {e}")))?;
    }
    Ok(())
}
pub fn make_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}
pub fn location_info(path: &Path) -> (bool, Option<u64>) {
    let Ok(p) = CString::new(path.as_os_str().as_bytes()) else {
        return (false, None);
    };
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(p.as_ptr(), info.as_mut_ptr()) } != 0 {
        return (false, None);
    }
    let info = unsafe { info.assume_init() };
    let read_only =
        info.f_flag & libc::ST_RDONLY != 0 || unsafe { libc::access(p.as_ptr(), libc::W_OK) } != 0;
    (
        read_only,
        Some((info.f_bavail as u64).saturating_mul(info.f_frsize)),
    )
}

pub fn preview_item(url: &NSURL) -> &ProtocolObject<dyn objc2_quick_look_ui::QLPreviewItem> {
    // NSURL conforms to QLPreviewItem in QuickLookUI, which is loaded by this binary.
    ProtocolObject::from_ref(url)
}
