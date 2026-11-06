//! Archive creation writes only to the caller's private stage. Sources are
//! opened without following leaf symlinks and checked for changes while read.
use super::{Archive, archive_error_string, safe_link, safe_relative, validate_materialized_link};
use crate::core::{model::Fingerprint, worker::Cancellation};
use std::{
    ffi::{CStr, CString},
    fs,
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
}
impl ArchiveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
        }
    }
}

#[link(name = "archive")]
unsafe extern "C" {
    fn archive_write_new() -> *mut Archive;
    fn archive_write_free(a: *mut Archive) -> libc::c_int;
    fn archive_write_fail(a: *mut Archive) -> libc::c_int;
    fn archive_write_set_format_zip(a: *mut Archive) -> libc::c_int;
    fn archive_write_set_format_7zip(a: *mut Archive) -> libc::c_int;
    fn archive_write_set_options(a: *mut Archive, options: *const libc::c_char) -> libc::c_int;
    fn archive_write_set_bytes_in_last_block(a: *mut Archive, bytes: libc::c_int) -> libc::c_int;
    fn archive_write_open(
        a: *mut Archive,
        data: *mut libc::c_void,
        open: Option<unsafe extern "C" fn(*mut Archive, *mut libc::c_void) -> libc::c_int>,
        write: Option<
            unsafe extern "C" fn(
                *mut Archive,
                *mut libc::c_void,
                *const libc::c_void,
                usize,
            ) -> isize,
        >,
        close: Option<unsafe extern "C" fn(*mut Archive, *mut libc::c_void) -> libc::c_int>,
    ) -> libc::c_int;
    fn archive_write_header(a: *mut Archive, entry: *mut Archive) -> libc::c_int;
    fn archive_write_data(a: *mut Archive, data: *const libc::c_void, size: usize) -> isize;
    fn archive_write_finish_entry(a: *mut Archive) -> libc::c_int;
    fn archive_write_close(a: *mut Archive) -> libc::c_int;
    fn archive_entry_new() -> *mut Archive;
    fn archive_entry_free(entry: *mut Archive);
    fn archive_entry_set_pathname(entry: *mut Archive, path: *const libc::c_char);
    fn archive_entry_set_symlink(entry: *mut Archive, path: *const libc::c_char);
    fn archive_entry_set_filetype(entry: *mut Archive, kind: libc::c_uint);
    fn archive_entry_set_perm(entry: *mut Archive, mode: libc::mode_t);
    fn archive_entry_set_size(entry: *mut Archive, size: i64);
    fn archive_entry_set_mtime(
        entry: *mut Archive,
        seconds: libc::time_t,
        nanoseconds: libc::c_long,
    );
}
struct Entry(*mut Archive);
impl Drop for Entry {
    fn drop(&mut self) {
        unsafe {
            archive_entry_free(self.0);
        }
    }
}
struct Writer {
    raw: *mut Archive,
    finished: bool,
}
impl Writer {
    fn check(&self, status: libc::c_int) -> io::Result<()> {
        if status >= 0 {
            return Ok(());
        }
        let ptr = unsafe { archive_error_string(self.raw) };
        Err(io::Error::other(if ptr.is_null() {
            "압축파일을 만들 수 없습니다".into()
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        }))
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        unsafe {
            if self.raw.is_null() {
                return;
            }
            if !self.finished {
                archive_write_fail(self.raw);
            }
            archive_write_free(self.raw);
        }
    }
}
struct Output<'a> {
    file: fs::File,
    cancel: &'a Cancellation,
    error: Option<io::Error>,
}
unsafe extern "C" fn write_output(
    _: *mut Archive,
    context: *mut libc::c_void,
    data: *const libc::c_void,
    size: usize,
) -> isize {
    let output = unsafe { &mut *context.cast::<Output<'_>>() };
    if size == 0 {
        return 0;
    }
    if data.is_null() || size > isize::MAX as usize {
        output.error = Some(io::Error::other("압축 출력 버퍼가 올바르지 않습니다"));
        return -1;
    }
    let result = output.cancel.check().and_then(|_| {
        output
            .file
            .write_all(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) })
    });
    match result {
        Ok(()) => size as isize,
        Err(error) => {
            output.error = Some(error);
            -1
        }
    }
}
enum Visit {
    Entry(PathBuf, PathBuf),
    VerifyDirectory(PathBuf, Fingerprint),
}

pub fn create(
    sources: &[PathBuf],
    stage: &Path,
    format: ArchiveFormat,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(u64, u64, &Path),
) -> io::Result<()> {
    cancel.check()?;
    let first = sources
        .first()
        .ok_or_else(|| io::Error::other("압축할 항목을 선택하세요"))?;
    let mut base = first
        .parent()
        .ok_or_else(|| io::Error::other("루트 폴더는 직접 압축할 수 없습니다"))?
        .to_path_buf();
    for path in sources.iter().skip(1) {
        while !path.parent().is_some_and(|p| p.starts_with(&base)) {
            if !base.pop() {
                return Err(io::Error::other("압축 경로의 공통 위치가 없습니다"));
            }
        }
    }
    let output_parent = fs::canonicalize(
        stage
            .parent()
            .ok_or_else(|| io::Error::other("압축 저장 위치가 없습니다"))?,
    )?;
    let mut stack = Vec::new();
    for source in sources.iter().rev() {
        let relative = safe_relative(source.strip_prefix(&base).map_err(io::Error::other)?)?;
        if relative.as_os_str().is_empty() {
            return Err(io::Error::other("압축 항목의 이름이 없습니다"));
        }
        if fs::symlink_metadata(source)?.is_dir()
            && output_parent.starts_with(fs::canonicalize(source)?)
        {
            return Err(io::Error::other(
                "압축파일을 원본 폴더 안에 만들 수 없습니다",
            ));
        }
        stack.push(Visit::Entry(source.clone(), relative));
    }
    // The output and callback storage outlive the writer, including error cleanup.
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(stage)?;
    let mut output = Output {
        file,
        cancel,
        error: None,
    };
    let mut writer = Writer {
        raw: unsafe { archive_write_new() },
        finished: false,
    };
    if writer.raw.is_null() {
        return Err(io::Error::other("압축 상태를 만들 수 없습니다"));
    }
    unsafe {
        writer.check(match format {
            ArchiveFormat::Zip => archive_write_set_format_zip(writer.raw),
            ArchiveFormat::SevenZip => archive_write_set_format_7zip(writer.raw),
        })?;
        writer.check(archive_write_set_options(
            writer.raw,
            match format {
                ArchiveFormat::Zip => {
                    c"compression=deflate,compression-level=6,hdrcharset=UTF-8".as_ptr()
                }
                ArchiveFormat::SevenZip => c"compression=lzma2,compression-level=5".as_ptr(),
            },
        ))?;
        writer.check(archive_write_set_bytes_in_last_block(writer.raw, 1))?;
        writer.check(archive_write_open(
            writer.raw,
            (&mut output as *mut Output<'_>).cast(),
            None,
            Some(write_output),
            None,
        ))?;
    }
    let mut bytes = 0u64;
    let mut count = 0u64;
    let mut buffer = vec![0u8; 128 * 1024];
    let mut quarantine = None;
    let result = (|| -> io::Result<()> {
        while let Some(visit) = stack.pop() {
            cancel.check()?;
            let (path, relative) = match visit {
                Visit::Entry(path, relative) => (path, relative),
                Visit::VerifyDirectory(path, before) => {
                    if !before.matches(&path) {
                        return Err(io::Error::other("압축 중 원본 폴더가 변경되었습니다"));
                    }
                    continue;
                }
            };
            progress(count, bytes, &relative);
            let metadata = fs::symlink_metadata(&path)?;
            let before = Fingerprint::from_metadata(&metadata);
            let kind = metadata.file_type();
            if !kind.is_file() && !kind.is_dir() && !kind.is_symlink() {
                return Err(io::Error::other(
                    "장치·소켓 등 특수 항목은 압축할 수 없습니다",
                ));
            }
            if quarantine.is_none() {
                quarantine = crate::platform::quarantine_attribute(&path)?;
            }
            let utf8 = relative.to_str().ok_or_else(|| {
                io::Error::other("ZIP·7z에는 UTF-8로 표현할 수 있는 파일 이름이 필요합니다")
            })?;
            let name = CString::new(utf8)?;
            let entry = Entry(unsafe { archive_entry_new() });
            if entry.0.is_null() {
                return Err(io::Error::other("압축 항목을 만들 수 없습니다"));
            }
            unsafe {
                archive_entry_set_pathname(entry.0, name.as_ptr());
                archive_entry_set_filetype(
                    entry.0,
                    (metadata.mode() & libc::S_IFMT as u32) as libc::c_uint,
                );
                archive_entry_set_perm(entry.0, (metadata.mode() & 0o777) as libc::mode_t);
                archive_entry_set_mtime(
                    entry.0,
                    metadata.mtime(),
                    metadata.mtime_nsec() as libc::c_long,
                );
                archive_entry_set_size(
                    entry.0,
                    if kind.is_file() {
                        metadata.len().try_into().map_err(io::Error::other)?
                    } else {
                        0
                    },
                );
            }
            if kind.is_symlink() {
                validate_materialized_link(&base, &path)?;
                let link = fs::read_link(&path)?;
                if !safe_link(relative.parent().unwrap_or(Path::new("")), &link) {
                    return Err(io::Error::other(
                        "선택 범위 밖을 가리키는 심볼릭 링크는 압축할 수 없습니다",
                    ));
                }
                let value =
                    CString::new(link.to_str().ok_or_else(|| {
                        io::Error::other("링크 경로를 UTF-8로 표현할 수 없습니다")
                    })?)?;
                unsafe {
                    // ZIP/7z writers read link payload through the native-byte
                    // getter. Supplying only the UTF-8 representation makes
                    // macOS libarchive normalize that payload to NFD.
                    archive_entry_set_symlink(entry.0, value.as_ptr());
                }
            }
            let mut input = if kind.is_file() {
                let file = fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(&path)?;
                if Fingerprint::from_metadata(&file.metadata()?) != before {
                    return Err(io::Error::other("압축할 원본이 변경되었습니다"));
                }
                Some(file)
            } else {
                None
            };
            writer.check(unsafe { archive_write_header(writer.raw, entry.0) })?;
            if let Some(input) = &mut input {
                let mut remaining = metadata.len();
                while remaining > 0 {
                    cancel.check()?;
                    let limit = buffer.len().min(remaining as usize);
                    let size = input.read(&mut buffer[..limit])?;
                    if size == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "압축 중 원본 파일 크기가 변경되었습니다",
                        ));
                    }
                    let mut offset = 0;
                    while offset < size {
                        cancel.check()?;
                        let written = unsafe {
                            archive_write_data(
                                writer.raw,
                                buffer[offset..size].as_ptr().cast(),
                                size - offset,
                            )
                        };
                        if written <= 0 {
                            writer.check(-1)?;
                        }
                        offset += written as usize;
                    }
                    remaining -= size as u64;
                    bytes = bytes.saturating_add(size as u64);
                    progress(count, bytes, &relative);
                }
                if Fingerprint::from_metadata(&input.metadata()?) != before {
                    return Err(io::Error::other("압축 중 원본 파일이 변경되었습니다"));
                }
            }
            writer.check(unsafe { archive_write_finish_entry(writer.raw) })?;
            if !before.matches(&path) {
                return Err(io::Error::other("압축 중 원본 항목이 변경되었습니다"));
            }
            if kind.is_dir() {
                let mut children: Vec<_> = fs::read_dir(&path)?.collect::<Result<_, _>>()?;
                children.sort_by_key(|e| e.file_name());
                stack.push(Visit::VerifyDirectory(path, before));
                for child in children.into_iter().rev() {
                    let relative = relative.join(child.file_name());
                    safe_relative(&relative)?;
                    stack.push(Visit::Entry(child.path(), relative));
                }
            }
            count += 1;
            progress(count, bytes, &relative);
        }
        cancel.check()?;
        writer.check(unsafe { archive_write_close(writer.raw) })?;
        writer.finished = true;
        cancel.check()?;
        Ok(())
    })();
    drop(writer);
    if let Some(error) = output.error {
        return Err(error);
    }
    result?;
    if let Some(value) = quarantine {
        crate::platform::set_quarantine_attribute(stage, &value)?;
    }
    output.file.sync_all()?;
    cancel.check()
}
