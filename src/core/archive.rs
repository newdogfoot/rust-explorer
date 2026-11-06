//! Streaming archive extraction into a new private staging directory.
//! Archive paths and links are validated before materialization; no existing
//! user file is overwritten and links are created only after regular files.
use super::worker::Cancellation;
mod create;
pub use create::{ArchiveFormat, create};
use std::{
    collections::VecDeque,
    ffi::{CStr, CString, OsString},
    fs,
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

type Archive = libc::c_void;
#[link(name = "archive")]
unsafe extern "C" {
    fn archive_read_new() -> *mut Archive;
    fn archive_read_free(a: *mut Archive) -> libc::c_int;
    fn archive_read_support_filter_all(a: *mut Archive) -> libc::c_int;
    fn archive_read_support_format_all(a: *mut Archive) -> libc::c_int;
    fn archive_read_open_filename(
        a: *mut Archive,
        name: *const libc::c_char,
        block: usize,
    ) -> libc::c_int;
    fn archive_read_next_header(a: *mut Archive, entry: *mut *mut Archive) -> libc::c_int;
    fn archive_read_data(a: *mut Archive, buffer: *mut libc::c_void, size: usize) -> isize;
    fn archive_error_string(a: *mut Archive) -> *const libc::c_char;
    fn archive_entry_pathname(a: *mut Archive) -> *const libc::c_char;
    fn archive_entry_pathname_utf8(a: *mut Archive) -> *const libc::c_char;
    fn archive_entry_filetype(a: *mut Archive) -> libc::mode_t;
    fn archive_entry_perm(a: *mut Archive) -> libc::mode_t;
    fn archive_entry_mtime(a: *mut Archive) -> libc::time_t;
    fn archive_entry_mtime_is_set(a: *mut Archive) -> libc::c_int;
    fn archive_entry_symlink(a: *mut Archive) -> *const libc::c_char;
    fn archive_entry_symlink_utf8(a: *mut Archive) -> *const libc::c_char;
    fn archive_entry_hardlink(a: *mut Archive) -> *const libc::c_char;
    fn archive_entry_hardlink_utf8(a: *mut Archive) -> *const libc::c_char;
}
struct Reader(*mut Archive);
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            archive_read_free(self.0);
        }
    }
}
impl Reader {
    fn error(&self) -> io::Error {
        unsafe {
            let ptr = archive_error_string(self.0);
            let text = if ptr.is_null() {
                "압축파일을 읽을 수 없습니다".into()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            };
            io::Error::other(
                if text.to_lowercase().contains("passphrase")
                    || text.to_lowercase().contains("encrypted")
                {
                    format!("암호가 설정된 압축파일은 아직 지원하지 않습니다: {text}")
                } else {
                    text
                },
            )
        }
    }
}
const EXTENSIONS: &[&str] = &[
    ".tar.gz",
    ".tar.bz2",
    ".tar.xz",
    ".tar.zst",
    ".tar.lzma",
    ".zip",
    ".tar",
    ".tgz",
    ".tbz2",
    ".txz",
    ".7z",
    ".rar",
];
pub fn extension(path: &Path) -> Option<&'static str> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    EXTENSIONS.iter().copied().find(|ext| name.ends_with(ext))
}
pub fn is_archive(path: &Path) -> bool {
    extension(path).is_some()
}
pub fn folder_name(path: &Path) -> OsString {
    let name = path.file_name().unwrap_or_default().as_bytes();
    for ext in EXTENSIONS {
        if name.len() > ext.len()
            && name[name.len() - ext.len()..].eq_ignore_ascii_case(ext.as_bytes())
        {
            return super::model::path_from_bytes(name[..name.len() - ext.len()].to_vec())
                .into_os_string();
        }
    }
    "압축 해제".into()
}
fn safe_relative(path: &Path) -> io::Result<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                let bytes = name.as_bytes();
                if bytes.contains(&b'\\') || bytes.get(1) == Some(&b':') {
                    return Err(io::Error::other("지원하지 않는 압축 경로입니다"));
                }
                result.push(name);
            }
            Component::CurDir => {}
            _ => return Err(io::Error::other("압축 경로가 대상 폴더를 벗어납니다")),
        }
    }
    Ok(result)
}
fn safe_link(parent: &Path, target: &Path) -> bool {
    let mut depth = parent.components().count();
    for part in target.components() {
        match part {
            Component::ParentDir => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Component::CurDir => {}
            Component::Normal(name) => {
                if name.as_bytes().contains(&b'\\') || name.as_bytes().get(1) == Some(&b':') {
                    return false;
                }
                depth += 1;
            }
            _ => return false,
        }
    }
    true
}
// Lexical validation alone misses a -> . followed by a/../outside.
// Resolve every component through the completed link graph before committing
// the stage. The stage is private and links are never used for file writes.
fn validate_materialized_link(root: &Path, link: &Path) -> io::Result<()> {
    let mut current = link
        .parent()
        .ok_or_else(|| io::Error::other("링크의 상위 경로가 없습니다"))?
        .to_path_buf();
    let mut pending: VecDeque<OsString> = fs::read_link(link)?
        .components()
        .map(|p| p.as_os_str().to_owned())
        .collect();
    let mut hops = 0;
    while let Some(part) = pending.pop_front() {
        if part == "." {
            continue;
        }
        if part == ".." {
            if current == root || !current.pop() {
                return Err(io::Error::other(
                    "연결된 압축 링크가 대상 폴더를 벗어납니다",
                ));
            }
            continue;
        }
        if Path::new(&part).is_absolute() {
            return Err(io::Error::other("절대 경로 링크는 압축 해제할 수 없습니다"));
        }
        let next = current.join(&part);
        match fs::symlink_metadata(&next) {
            Ok(meta) if meta.file_type().is_symlink() => {
                hops += 1;
                if hops > 40 {
                    return Err(io::Error::other("압축 링크가 순환하거나 너무 깊습니다"));
                }
                let target = fs::read_link(&next)?;
                if target.is_absolute() {
                    return Err(io::Error::other(
                        "연결된 압축 링크가 절대 경로를 가리킵니다",
                    ));
                }
                for component in target.components().rev() {
                    pending.push_front(component.as_os_str().to_owned());
                }
            }
            Ok(_) => current = next,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                current = next
            }
            Err(error) => return Err(error),
        }
        if !current.starts_with(root) {
            return Err(io::Error::other("압축 링크가 대상 폴더를 벗어납니다"));
        }
    }
    Ok(())
}

unsafe fn path_string(ptr: *const libc::c_char) -> Option<PathBuf> {
    if ptr.is_null() {
        None
    } else {
        Some(super::model::path_from_bytes(
            unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec(),
        ))
    }
}
pub fn extract(
    source: &Path,
    stage: &Path,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(u64, u64, &Path),
) -> io::Result<()> {
    cancel.check()?;
    let quarantine = crate::platform::quarantine_attribute(source)?;
    let name = CString::new(source.as_os_str().as_bytes())?;
    let reader = Reader(unsafe { archive_read_new() });
    if reader.0.is_null() {
        return Err(io::Error::other("압축 해제 상태를 만들 수 없습니다"));
    }
    unsafe {
        archive_read_support_filter_all(reader.0);
        archive_read_support_format_all(reader.0);
        if archive_read_open_filename(reader.0, name.as_ptr(), 128 * 1024) < 0 {
            return Err(reader.error());
        }
    }
    // The caller owns this newly created stage. 0700 also prevents another
    // user replacing one of its components with a symlink during extraction.
    fs::set_permissions(stage, fs::Permissions::from_mode(0o700))?;
    let mut buffer = vec![0u8; 128 * 1024];
    let mut bytes = 0u64;
    let mut count = 0u64;
    let mut symlinks = vec![];
    let mut hardlinks = vec![];
    let mut metadata = vec![];
    loop {
        cancel.check()?;
        let mut entry = std::ptr::null_mut();
        let status = unsafe { archive_read_next_header(reader.0, &mut entry) };
        if status == 1 {
            break;
        }
        if status < 0 || entry.is_null() {
            return Err(reader.error());
        }
        let raw = unsafe {
            path_string(archive_entry_pathname_utf8(entry))
                .or_else(|| path_string(archive_entry_pathname(entry)))
        }
        .ok_or_else(|| io::Error::other("압축 항목에 이름이 없습니다"))?;
        let relative = safe_relative(&raw)?;
        let kind = unsafe { archive_entry_filetype(entry) };
        if relative.as_os_str().is_empty() {
            if kind == libc::S_IFDIR {
                continue;
            }
            return Err(io::Error::other("빈 압축 파일 경로입니다"));
        }
        let destination = stage.join(&relative);
        fs::create_dir_all(destination.parent().unwrap())?;
        let hard = unsafe {
            path_string(archive_entry_hardlink_utf8(entry))
                .or_else(|| path_string(archive_entry_hardlink(entry)))
        };
        if let Some(target) = hard {
            hardlinks.push((destination, safe_relative(&target)?));
        } else if kind == libc::S_IFLNK {
            let target = unsafe {
                path_string(archive_entry_symlink_utf8(entry))
                    .or_else(|| path_string(archive_entry_symlink(entry)))
            }
            .ok_or_else(|| io::Error::other("링크 대상이 없습니다"))?;
            if !safe_link(relative.parent().unwrap(), &target) {
                return Err(io::Error::other("압축 링크가 대상 폴더를 벗어납니다"));
            }
            symlinks.push((destination, target));
        } else if kind == libc::S_IFDIR {
            fs::create_dir_all(&destination)?;
        } else if kind == libc::S_IFREG {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&destination)?;
            loop {
                cancel.check()?;
                let n = unsafe {
                    archive_read_data(reader.0, buffer.as_mut_ptr().cast(), buffer.len())
                };
                if n < 0 {
                    return Err(reader.error());
                }
                if n == 0 {
                    break;
                }
                file.write_all(&buffer[..n as usize])?;
                bytes = bytes.saturating_add(n as u64);
                progress(count, bytes, &relative);
            }
            file.sync_all()?;
        } else {
            return Err(io::Error::other(
                "장치·소켓 등 특수 압축 항목은 지원하지 않습니다",
            ));
        }
        if matches!(kind, libc::S_IFREG | libc::S_IFDIR) {
            let mode = (unsafe { archive_entry_perm(entry) } as u32 & 0o777)
                | if kind == libc::S_IFDIR { 0o700 } else { 0o600 };
            let modified = unsafe {
                (archive_entry_mtime_is_set(entry) != 0).then(|| archive_entry_mtime(entry))
            };
            metadata.push((stage.join(&relative), mode, modified));
        }
        count += 1;
        progress(count, bytes, &relative);
    }
    for (path, target) in hardlinks {
        cancel.check()?;
        let target = stage.join(target);
        if !fs::symlink_metadata(&target)?.file_type().is_file() {
            return Err(io::Error::other("하드링크 대상은 일반 파일이어야 합니다"));
        }
        fs::hard_link(target, path)?;
    }
    for (path, target) in &symlinks {
        cancel.check()?;
        std::os::unix::fs::symlink(target, path)?;
    }
    for (path, _) in &symlinks {
        cancel.check()?;
        validate_materialized_link(stage, path)?;
    }
    // Apply directory permissions last, after all children are written.
    metadata.sort_by_key(|(p, _, _)| std::cmp::Reverse(p.components().count()));
    for (path, mode, modified) in metadata {
        cancel.check()?;
        if let Some(value) = &quarantine {
            crate::platform::set_quarantine_attribute(&path, value)?;
        }
        if let Some(seconds) = modified {
            let time = if seconds >= 0 {
                UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
            } else {
                UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
            };
            if let Some(time) = time {
                fs::File::open(&path)?.set_times(fs::FileTimes::new().set_modified(time))?;
            }
        }
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    if let Some(value) = &quarantine {
        crate::platform::set_quarantine_attribute(stage, value)?;
    }
    Ok(())
}
