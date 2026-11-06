use block2::{DynBlock, RcBlock};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSMutableDictionary, NSString, NSURL, ns_string};
use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    ptr,
};

/// Distinguish an explicit server address from a relative filesystem path.
pub fn is_server_address(input: &str) -> bool {
    let value = input.trim();
    value.starts_with(r"\\")
        || value.split_once("://").is_some_and(|(scheme, _)| {
            ["smb", "nfs", "http", "https"]
                .iter()
                .any(|known| scheme.eq_ignore_ascii_case(known))
        })
}

pub fn server_url(input: &str) -> Result<Retained<NSURL>, String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 4096 || input.chars().any(char::is_control) {
        return Err("서버 주소를 입력하세요.".into());
    }
    let (scheme, address) = if let Some(unc) = input
        .strip_prefix(r"\\")
        .or_else(|| input.strip_prefix("//"))
    {
        (
            "smb".into(),
            unc.replace('\\', "/")
                .replace('%', "%25")
                .replace('#', "%23")
                .replace('?', "%3F"),
        )
    } else if let Some((scheme, address)) = input.split_once("://") {
        (scheme.to_ascii_lowercase(), address.to_owned())
    } else {
        ("smb".into(), input.to_owned())
    };
    if !matches!(scheme.as_str(), "smb" | "nfs" | "https" | "http") {
        return Err("SMB, NFS 또는 WebDAV(http/https) 주소를 입력하세요.".into());
    }
    let authority = address.split('/').next().unwrap_or_default();
    if authority.contains('@') {
        return Err("계정 정보는 주소에서 빼고 macOS 로그인 창에 입력하세요.".into());
    }
    let port = if authority.starts_with('[') {
        let (_, tail) = authority
            .split_once(']')
            .ok_or("IPv6 서버 주소를 확인하세요.")?;
        if tail.is_empty() {
            None
        } else {
            Some(tail.strip_prefix(':').ok_or("포트 번호를 확인하세요.")?)
        }
    } else {
        authority.split_once(':').map(|(_, port)| port)
    };
    if port.is_some_and(|value| {
        value.is_empty()
            || !value.bytes().all(|b| b.is_ascii_digit())
            || value.parse::<u16>().map_or(true, |n| n == 0)
    }) {
        return Err("포트 번호는 1~65535 사이의 숫자를 입력하세요.".into());
    }
    if authority.is_empty()
        || authority.chars().any(char::is_whitespace)
        || address.contains(['\\', '?', '#'])
    {
        return Err("서버 이름과 공유 경로를 확인하세요.".into());
    }
    let url = NSURL::URLWithString_encodingInvalidCharacters(
        &NSString::from_str(&format!("{scheme}://{address}")),
        true,
    )
    .ok_or("서버 주소 형식을 확인하세요.")?;
    let host = url.host().map(|h| h.to_string()).unwrap_or_default();
    if matches!(host.as_str(), "" | "." | ".." | "~")
        || host.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '@' | '?' | '#')
        })
        || url.user().is_some()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url
            .port()
            .is_some_and(|port| !(1..=65535).contains(&port.integerValue()))
    {
        return Err("서버 이름 또는 포트 번호를 확인하세요.".into());
    }
    Ok(url)
}

pub enum MountResult {
    Connected(Vec<PathBuf>),
    AlreadyMounted,
    Cancelled,
}
impl MountResult {
    fn from_status(status: i32, paths: Vec<PathBuf>) -> Result<Self, String> {
        match status {
            0 => Ok(Self::Connected(paths)),
            libc::EEXIST if !paths.is_empty() => Ok(Self::Connected(paths)),
            libc::EEXIST => Ok(Self::AlreadyMounted),
            -128 | libc::ECANCELED => Ok(Self::Cancelled),
            code => Err(mount_error(code)),
        }
    }
}
type MountReport = DynBlock<dyn Fn(i32, *mut c_void, *const NSArray<NSString>)>;

#[link(name = "NetFS", kind = "framework")]
unsafe extern "C" {
    fn NetFSMountURLAsync(
        url: *const NSURL,
        mount_path: *const NSURL,
        user: *const NSString,
        password: *const NSString,
        open_options: *const NSMutableDictionary<NSString, NSString>,
        mount_options: *mut c_void,
        request: *mut *mut c_void,
        queue: &DispatchQueue,
        report: &MountReport,
    ) -> i32;
    fn NetFSMountURLCancel(request: *mut c_void) -> i32;
}

pub struct MountRequest(ptr::NonNull<c_void>);
pub enum MountStart {
    Pending(MountRequest),
    Finished(MountResult),
}
impl MountRequest {
    pub fn cancel(self) -> bool {
        unsafe { NetFSMountURLCancel(self.0.as_ptr()) == 0 }
    }
}

fn mount_error(code: i32) -> String {
    match code {
        -5998 | -6003 => "이 서버에 사용할 수 있는 공유 폴더가 없습니다.".into(),
        -5999 | -5045 | -5046 | -5997 => {
            "서버가 로그인을 허용하지 않았습니다. 계정과 권한을 확인하세요.".into()
        }
        -5996 => "서버와 호환되는 파일 공유 프로토콜이 없습니다.".into(),
        n if n > 0 => format!(
            "서버에 연결할 수 없습니다. {}",
            std::io::Error::from_raw_os_error(n)
        ),
        n => format!("서버에 연결할 수 없습니다. 주소와 네트워크를 확인하세요. (macOS 오류 {n})"),
    }
}

pub fn mount_server(
    url: &NSURL,
    done: impl Fn(Result<MountResult, String>) + Send + Sync + 'static,
) -> Result<MountStart, String> {
    // NetFS copies the completion block and submits it to the main queue.
    // The callback's CFArray and its CFStrings are borrowed, toll-free bridged objects.
    let report = RcBlock::new(
        move |status: i32, _: *mut c_void, points: *const NSArray<NSString>| {
            let paths = unsafe { points.as_ref() }
                .into_iter()
                .flat_map(|list| list.iter())
                .map(|name| PathBuf::from(name.to_string()))
                .filter(|path| path.is_absolute())
                .collect::<Vec<_>>();
            done(MountResult::from_status(status, paths));
        },
    );
    let options = NSMutableDictionary::<NSString, NSString>::from_slices(
        &[ns_string!("UIOption")],
        &[ns_string!("AllowUI")],
    );
    let mut handle = ptr::null_mut();
    let status = unsafe {
        NetFSMountURLAsync(
            url,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            &*options,
            ptr::null_mut(),
            &mut handle,
            DispatchQueue::main(),
            &report,
        )
    };
    if status != 0 {
        return MountResult::from_status(status, vec![]).map(MountStart::Finished);
    }
    ptr::NonNull::new(handle)
        .map(MountRequest)
        .map(MountStart::Pending)
        .ok_or_else(|| "서버 연결 요청을 시작할 수 없습니다.".into())
}

pub fn disconnect_network_volume(path: &Path) -> Result<(), String> {
    use objc2_foundation::{
        NSFileManager, NSNumber, NSURLVolumeIsLocalKey, NSVolumeEnumerationOptions,
    };
    objc2::rc::autoreleasepool(|_| unsafe {
        let keys = NSArray::from_slice(&[NSURLVolumeIsLocalKey]);
        let volumes = NSFileManager::new()
            .mountedVolumeURLsIncludingResourceValuesForKeys_options(
                Some(&keys),
                NSVolumeEnumerationOptions::SkipHiddenVolumes,
            )
            .ok_or("연결된 볼륨 목록을 확인할 수 없습니다.")?;
        for url in volumes.iter().filter(|url| super::url_path(url) == path) {
            let mut local = None;
            url.getResourceValue_forKey_error(&mut local, NSURLVolumeIsLocalKey)
                .map_err(|e| e.localizedDescription().to_string())?;
            if local
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .is_some_and(|v| !v.boolValue())
            {
                return objc2_app_kit::NSWorkspace::sharedWorkspace()
                    .unmountAndEjectDeviceAtURL_error(&url)
                    .map_err(|e| e.localizedDescription().to_string());
            }
        }
        Err("현재 연결된 네트워크 볼륨이 아닙니다.".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_completion_distinguishes_cancel_existing_mount_and_failure() {
        let path = PathBuf::from("/Volumes/test-share");
        for status in [0, libc::EEXIST] {
            assert!(
                matches!(MountResult::from_status(status,vec![path.clone()]),Ok(MountResult::Connected(paths)) if paths==[path.clone()])
            );
        }
        assert!(matches!(
            MountResult::from_status(libc::EEXIST, vec![]),
            Ok(MountResult::AlreadyMounted)
        ));
        for status in [-128, libc::ECANCELED] {
            assert!(matches!(
                MountResult::from_status(status, vec![]),
                Ok(MountResult::Cancelled)
            ));
        }
        assert!(MountResult::from_status(libc::EACCES, vec![]).is_err());
        assert!(MountResult::from_status(-5998, vec![]).is_err());
    }
}
