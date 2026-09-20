//! 官网 WebView 登录提取 Cookie — 对齐手机端 LoginPage.ets 的 session Cookie 判据
//!
//! 判定通道：每 800ms 轮询 `cookies_for_url`（可读 HTTP-only Cookie），session 出现即登录成功。
//! 手机端另有「URL 跳转 /dashboard」快路径，桌面端**不实现**页面加载时机的同步检查：
//! Tauri 文档明确 `cookies()` / `cookies_for_url()` 在 Windows 上于同步 command 或
//! 事件处理器中调用会死锁（wry#583，须在 async command 与独立线程中读取），
//! 故仅在 spawned 轮询任务里读取。
//! 提取成功后广播 `login://success`（载荷为完整 Cookie 请求头串），由前端走现有
//! save_config 保存验证链路；取消/超时分别广播 `login://cancelled` / `login://timeout`。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{Emitter, Manager, Url};

const LOGIN_URL: &str = "https://ai.amaxsmp.com/login";
const SITE_URL: &str = "https://ai.amaxsmp.com";
/// 与手机端 LOGIN_POLL_INTERVAL_MS 对齐
const POLL_INTERVAL: Duration = Duration::from_millis(800);
/// 看门狗：约 10 分钟未完成登录即放弃
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// 登录窗 label，同时用于 capability/窗口查找
const LOGIN_LABEL: &str = "login";

/// 从 Cookie 请求头串提取 session 值，口径同手机端 Format.ets::parseSessionCookie
fn extract_session(cookie_header: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("session=")
            && !value.is_empty()
        {
            return Some(value.to_string());
        }
    }
    None
}

/// 把 Cookie 数组组装为请求头串 `name=value; name2=value2; …`
fn build_cookie_header(cookies: &[tauri::webview::Cookie<'static>]) -> String {
    cookies
        .iter()
        .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
        .collect::<Vec<_>>()
        .join("; ")
}

/// 从登录窗口的 Cookie 存储取整串请求头；窗口销毁竞态等错误返回 None（轮询下轮重试）
fn fetch_cookie_header(window: &tauri::WebviewWindow) -> Option<String> {
    let url = Url::parse(SITE_URL).ok()?;
    let cookies = window.cookies_for_url(url).ok()?;
    let header = build_cookie_header(&cookies);
    (!header.is_empty()).then_some(header)
}

/// 本次登录流程是否已结束（成功/取消/超时），用于防重入与终止轮询
fn settle(flag: &AtomicBool) -> bool {
    !flag.swap(true, Ordering::SeqCst)
}

#[tauri::command]
pub async fn open_login_window(app: tauri::AppHandle) -> Result<(), String> {
    // 幂等：窗口已存在时仅聚焦，不重复创建/重复轮询
    if let Some(existing) = app.get_webview_window(LOGIN_LABEL) {
        return existing
            .set_focus()
            .map_err(|error| format!("聚焦登录窗口失败: {error}"));
    }

    let settled = Arc::new(AtomicBool::new(false));

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        LOGIN_LABEL,
        tauri::WebviewUrl::External(LOGIN_URL.parse().map_err(|_| "登录地址无效".to_string())?),
    )
    .title("登录 AMAX")
    .inner_size(1000.0, 720.0)
    .center()
    // 首帧空白闪烁规避：页面加载完成后再显示（此处不读 Cookie，见模块注释）
    .visible(false)
    .on_page_load(|window, payload| {
        if payload.event() == tauri::webview::PageLoadEvent::Finished {
            let _ = window.show().and_then(|_| window.set_focus());
        }
    })
    .build()
    .map_err(|error| format!("创建登录窗口失败: {error}"))?;

    // 取消：用户成功前关窗（成功/超时路径我们自己 close()，彼时 settled 已置位不误报）
    let cancel_settled = Arc::clone(&settled);
    let cancel_handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::CloseRequested { .. })
            && settle(&cancel_settled)
            && let Err(error) = cancel_handle.emit("login://cancelled", ())
        {
            eprintln!("推送登录取消事件失败: {error}");
        }
    });

    // 轮询判据：官网写入 session Cookie 即视为登录成功（不依赖页面跳转）
    let poll_window = window.clone();
    let poll_settled = Arc::clone(&settled);
    tauri::async_runtime::spawn(async move {
        let deadline = tokio::time::Instant::now() + LOGIN_TIMEOUT;
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            interval.tick().await;
            if poll_settled.load(Ordering::SeqCst) {
                return; // 已被取消/成功/超时终止
            }
            if tokio::time::Instant::now() >= deadline {
                if settle(&poll_settled) {
                    let _ = poll_window.close();
                    if let Err(error) = poll_window.app_handle().emit("login://timeout", ()) {
                        eprintln!("推送登录超时事件失败: {error}");
                    }
                }
                return;
            }
            let Some(header) = fetch_cookie_header(&poll_window) else {
                continue;
            };
            if extract_session(&header).is_none() {
                continue;
            }
            if settle(&poll_settled) {
                if let Err(error) = poll_window
                    .app_handle()
                    .emit("login://success", serde_json::json!({ "cookie": header }))
                {
                    eprintln!("推送登录成功事件失败: {error}");
                }
                let _ = poll_window.close();
            }
            return;
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_cookie_header, extract_session};
    use tauri::webview::Cookie;

    fn cookie(name: &str, value: &str) -> Cookie<'static> {
        Cookie::new(name.to_string(), value.to_string())
    }

    #[test]
    fn builds_header_from_cookies() {
        let cookies = [
            cookie("session", "abc123"),
            cookie("theme", "dark"),
            cookie("csrf", "xyz"),
        ];
        assert_eq!(
            build_cookie_header(&cookies),
            "session=abc123; theme=dark; csrf=xyz"
        );
    }

    #[test]
    fn builds_empty_header_for_no_cookies() {
        assert_eq!(build_cookie_header(&[]), "");
    }

    #[test]
    fn extracts_session_value() {
        assert_eq!(
            extract_session("session=MTc4MzQy; theme=dark").as_deref(),
            Some("MTc4MzQy")
        );
    }

    #[test]
    fn extracts_session_not_first() {
        assert_eq!(
            extract_session("theme=dark; session=v9; csrf=x").as_deref(),
            Some("v9")
        );
    }

    #[test]
    fn returns_none_when_session_absent_or_empty() {
        assert_eq!(extract_session("theme=dark"), None);
        assert_eq!(extract_session("session=; theme=dark"), None);
        assert_eq!(extract_session(""), None);
    }
}
