//! external.rs — 用系统默认浏览器打开外链。
//!
//! 背景：桌面端 WebView 不处理 `target="_blank"`（既不开新窗也不报错），
//! 页面里的外链点了毫无反应。前端把这类点击统一拦下并调用本命令，
//! 由系统浏览器接管 —— 与 `preview.rs` 的 `reveal_path` 同一套做法。
//!
//! 安全边界：只放行 `http` / `https`，拒绝控制字符与超长 URL；

/// 允许打开的外链（纯函数，便于单测）。
///
/// 规则：非空、长度 ≤ 2048、无控制字符（含换行/制表）、scheme 为 http/https
/// 且带非空 host。
pub fn is_allowed_external_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || u.len() > 2048 {
        return false;
    }
    if u.chars().any(|c| c.is_control()) {
        return false;
    }
    let rest = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .or_else(|| u.strip_prefix("HTTP://"))
        .or_else(|| u.strip_prefix("HTTPS://"));
    let Some(rest) = rest else {
        return false;
    };
    // host 部分必须非空，且不含空白（`http:// /x` 之类形态直接拒绝）
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.chars().any(char::is_whitespace)
}

/**
 * 用系统默认浏览器打开外链。Windows 直接传递 UTF-16 URL，避免 shell 拆分 OAuth 查询参数。
 * ShellExecuteW 调用期间字符串保持有效且以 NUL 结尾；验证器已拒绝内嵌 NUL。
 */
#[tauri::command(async)]
pub fn open_external(url: String) -> Result<(), String> {
    let url = url.trim().to_string();
    if !is_allowed_external_url(&url) {
        return Err(format!("不支持的链接（仅允许 http/https）：{url}"));
    }

    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let target: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if result <= 32 {
            return Err(format!("打开链接失败（Windows 错误 {result}）"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开链接失败：{e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开链接失败：{e}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_allowed_external_url;

    /** Exercise the actual system browser boundary; mocks cannot detect URL truncation by cmd. */
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "opens the system browser for a local loopback smoke test"]
    fn windows_browser_preserves_oauth_query_parameters() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let query = "/verify?response_type=code&client_id=synthetic&scope=openid+profile&name=%E4%B8%AD%E6%96%87&encoded=%26%25%22";
        let url = format!(
            "http://127.0.0.1:{}{query}",
            listener.local_addr().unwrap().port()
        );
        super::open_external(url).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "browser did not reach the callback"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("callback accept failed: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = String::new();
        BufReader::new(&mut stream).read_line(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .unwrap();
        assert_eq!(request.trim_end(), format!("GET {query} HTTP/1.1"));
    }

    #[test]
    fn allows_http_and_https() {
        assert!(is_allowed_external_url(
            "https://github.com/mrpulor-gh/nuphus"
        ));
        assert!(is_allowed_external_url("http://example.com/a?b=1#c"));
        assert!(is_allowed_external_url("HTTPS://Example.com"));
    }

    #[test]
    fn rejects_non_http_schemes_and_malformed() {
        assert!(!is_allowed_external_url("mailto:a@b.com"));
        assert!(!is_allowed_external_url("file:///C:/Windows"));
        assert!(!is_allowed_external_url("javascript:alert(1)"));
        assert!(!is_allowed_external_url(""));
        assert!(!is_allowed_external_url("https://"));
        assert!(!is_allowed_external_url("http:// /x"));
    }

    #[test]
    fn rejects_control_chars_and_overlong() {
        assert!(!is_allowed_external_url("https://example.com/\nnext"));
        assert!(!is_allowed_external_url("https://example.com/\u{0}"));
        let long = format!("https://example.com/{}", "a".repeat(3000));
        assert!(!is_allowed_external_url(&long));
    }
}
