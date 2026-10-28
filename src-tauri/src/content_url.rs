use url::Url;

pub fn validate_content_url(value: &str) -> Result<String, String> {
    let parsed = Url::parse(value).map_err(|_| "invalid_content_url".to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("www.douyin.com")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("invalid_content_url".to_string());
    }

    let segments = parsed
        .path_segments()
        .ok_or_else(|| "invalid_content_url".to_string())?
        .collect::<Vec<_>>();
    if segments.len() != 2
        || !matches!(segments[0], "video" | "note")
        || segments[1].is_empty()
        || !segments[1]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("invalid_content_url".to_string());
    }

    Ok(parsed.into())
}

#[cfg(windows)]
pub fn open_content_url(value: &str) -> Result<(), String> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use std::{ffi::OsStr, ptr};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let validated = validate_content_url(value)?;
    let operation = OsStr::new("open")
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let target = OsStr::new(&validated)
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        return Err(format!("open_content_url_failed:{result}"));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn open_content_url(value: &str) -> Result<(), String> {
    validate_content_url(value)?;
    Err("open_content_url_unsupported_platform".to_string())
}

#[cfg(test)]
mod tests {
    use super::validate_content_url;

    #[test]
    fn accepts_only_canonical_douyin_content_urls() {
        assert_eq!(
            validate_content_url("https://www.douyin.com/video/123456").unwrap(),
            "https://www.douyin.com/video/123456"
        );
        assert_eq!(
            validate_content_url("https://www.douyin.com/note/note-id_1").unwrap(),
            "https://www.douyin.com/note/note-id_1"
        );
    }

    #[test]
    fn rejects_noncanonical_or_unsafe_urls() {
        for value in [
            "http://www.douyin.com/video/1",
            "https://douyin.com/video/1",
            "https://www.douyin.com.evil.test/video/1",
            "https://www.douyin.com/video/1?from=app",
            "https://www.douyin.com/video/1/extra",
            "https://www.douyin.com/user/self",
        ] {
            assert_eq!(
                validate_content_url(value).unwrap_err(),
                "invalid_content_url"
            );
        }
    }
}
