//! Tauri 命令模块 — IPC 桥接 + 托盘 + 通知 + 自动刷新

mod api;
mod crypto;
mod db;
mod error;
mod login;

use chrono::{Local, NaiveDate};
use db::Db;
use error::{AppError, poisoned};
use std::sync::Mutex;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

pub struct AppState {
    db: Mutex<Db>,
    client: reqwest::Client,
    refresh_lock: tokio::sync::Mutex<()>,
    low_quota_notified: Mutex<bool>,
    /// 最近一次成功刷新时刻，用于启动任务去重（窗口可见时前端已触发刷新）
    last_successful_refresh: Mutex<Option<std::time::Instant>>,
    /// 最近一次刷新得到的 user_id，供看板请求并行（缓存失效时自动回退重查）
    user_id_cache: Mutex<Option<i64>>,
}

/// 校验统计区间：格式、先后、不超今天、跨度上限 1096 天
fn validate_date_range(
    start_date: &str,
    end_date: &str,
) -> Result<(NaiveDate, NaiveDate), AppError> {
    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .map_err(|_| AppError::input("起始日期格式无效，应为 YYYY-MM-DD"))?;
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .map_err(|_| AppError::input("结束日期格式无效，应为 YYYY-MM-DD"))?;
    if start > end {
        return Err(AppError::input("起始日期不能晚于结束日期"));
    }
    if end > Local::now().date_naive() {
        return Err(AppError::input("结束日期不能超过今天"));
    }
    if (end - start).num_days() > 1096 {
        return Err(AppError::input("区间跨度不能超过 1096 天"));
    }
    Ok((start, end))
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
    Ok(serde_json::json!({
        "has_cookie": db.has_cookie(),
        "has_api_key": db.has_api_key(),
        "cookie_expires_at": db.get_cookie_expires_at(),
    }))
}

/// 本地统计每日行（余额 + 降级估算 + 请求数差分）
#[derive(serde::Serialize)]
struct LocalDaily {
    date: String,
    remaining: f64,
    yuan: f64,
    tokens: i64,
    new_requests: Option<i64>,
}

#[derive(serde::Serialize)]
struct LocalSummary {
    total_yuan: f64,
    avg_yuan: f64,
    peak_yuan: f64,
    peak_date: String,
    total_tokens: i64,
    days_with_data: usize,
}

#[derive(serde::Serialize)]
struct LocalStats {
    daily: Vec<LocalDaily>,
    summary: LocalSummary,
}

#[tauri::command]
fn get_local_stats(
    state: tauri::State<AppState>,
    start_date: String,
    end_date: String,
) -> Result<LocalStats, AppError> {
    validate_date_range(&start_date, &end_date)?;
    let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
    let snapshots = db.get_daily_snapshots(&start_date, &end_date)?;
    let diffs = db::derive_daily_requests(&snapshots);
    let daily = snapshots
        .iter()
        .zip(diffs)
        .map(|(row, new_requests)| LocalDaily {
            date: row.date.clone(),
            remaining: row.remaining,
            yuan: row.yuan,
            tokens: row.tokens,
            new_requests,
        })
        .collect();
    let (total_yuan, avg_yuan, peak_yuan, peak_date, total_tokens) =
        db::summarize_local(&snapshots);
    Ok(LocalStats {
        daily,
        summary: LocalSummary {
            total_yuan,
            avg_yuan,
            peak_yuan,
            peak_date,
            total_tokens,
            days_with_data: snapshots.len(),
        },
    })
}

#[tauri::command]
async fn fetch_usage_stats(
    state: tauri::State<'_, AppState>,
    start_date: String,
    end_date: String,
) -> Result<api::UsageStats, AppError> {
    let (start, end) = validate_date_range(&start_date, &end_date)?;
    let cookie = {
        let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
        db.get_cookie()
    };
    api::fetch_usage_stats(&state.client, &cookie.unwrap_or_default(), start, end).await
}

#[tauri::command]
fn save_config(
    state: tauri::State<AppState>,
    cookie: String,
    api_key: String,
    expires_at: Option<String>,
) -> Result<(), AppError> {
    if cookie.is_empty() && api_key.is_empty() {
        return Err(AppError::input("请至少填写一项认证信息"));
    }
    let mut db = state.db.lock().map_err(|_| poisoned("数据库"))?;
    db.save_config(&cookie, &api_key, expires_at.as_deref())?;
    // Cookie 变更后 user_id 可能不同，清缓存避免用旧 id 并发查询
    if let Ok(mut cache) = state.user_id_cache.lock() {
        *cache = None;
    }
    Ok(())
}

#[tauri::command]
async fn fetch_dashboard(app: tauri::AppHandle) -> Result<api::DashboardData, AppError> {
    refresh_dashboard(&app, false, false).await
}

fn hide_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main")
        && let Err(error) = window.set_skip_taskbar(true).and_then(|_| window.hide())
    {
        log::error!("隐藏主窗口失败: {error}");
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main")
        && let Err(error) = window
            .set_skip_taskbar(false)
            .and_then(|_| window.show())
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
    {
        log::error!("显示主窗口失败: {error}");
    }
}

fn update_tray_tooltip(app: &tauri::AppHandle, data: &api::DashboardData) {
    if let Some(tray) = app.tray_by_id("main")
        && let Err(error) = tray.set_tooltip(Some(&format!(
            "AMAX Dashboard\n今日消耗 ¥{:.6}\n剩余额度 ¥{:.2}（{:.1}%）",
            data.today_yuan, data.remaining, data.percent
        )))
    {
        log::error!("更新托盘提示失败: {error}");
    }
}

async fn refresh_dashboard(
    app: &tauri::AppHandle,
    notify_low_quota: bool,
    emit_update: bool,
) -> Result<api::DashboardData, AppError> {
    let state = app.state::<AppState>();
    let _refresh_guard = state.refresh_lock.lock().await;
    let cookie = {
        let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
        db.get_cookie().unwrap_or_default()
    };
    // 缓存 user_id 时并行拉账户信息与当日用量，省一个 RTT；id 变化自动回退重查
    let cached_user_id = state.user_id_cache.lock().ok().and_then(|cache| *cache);
    let (data, user_id) = api::fetch_dashboard(&state.client, &cookie, cached_user_id).await?;
    if let Ok(mut cache) = state.user_id_cache.lock() {
        *cache = Some(user_id);
    }

    {
        let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
        db.save_snapshot(
            data.today_yuan,
            data.today_tokens,
            data.remaining,
            data.used,
            data.total,
            data.request_count,
        )?;
    }

    update_tray_tooltip(app, &data);
    if emit_update {
        app.emit("dashboard-updated", &data)
            .map_err(|error| AppError::storage(format!("推送看板更新失败: {error}")))?;
    }

    let should_notify = {
        let mut notified = state
            .low_quota_notified
            .lock()
            .map_err(|_| poisoned("通知状态锁"))?;
        if data.percent >= 10.0 {
            *notified = false;
            false
        } else if notify_low_quota && !*notified {
            *notified = true;
            true
        } else {
            false
        }
    };
    if should_notify {
        app.notification()
            .builder()
            .title("⚠️ AMAX 额度不足")
            .body(format!(
                "剩余 ¥{:.2} / ¥{:.2} ({:.1}%), 请及时充值",
                data.remaining, data.total, data.percent
            ))
            .show()
            .map_err(|error| AppError::storage(format!("发送额度通知失败: {error}")))?;
    }

    if let Ok(mut last) = state.last_successful_refresh.lock() {
        *last = Some(std::time::Instant::now());
    }

    Ok(data)
}

fn build_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let refresh = MenuItemBuilder::with_id("tray_refresh", "🔄 刷新数据").build(app)?;
    let show = MenuItemBuilder::with_id("tray_show", "📊 显示窗口").build(app)?;
    let check_update = MenuItemBuilder::with_id("tray_update", "⬇️ 检查更新").build(app)?;
    let quit = MenuItemBuilder::with_id("tray_quit", "❌ 退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&refresh)
        .item(&show)
        .separator()
        .item(&check_update)
        .separator()
        .item(&quit)
        .build()?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("应用未配置默认图标")?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("AMAX Dashboard")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "tray_refresh" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    refresh_and_notify(&app).await;
                });
            }
            "tray_update" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    check_for_updates(&app).await;
                });
            }
            "tray_show" => show_main_window(app),
            "tray_quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// 后台路径（定时/托盘/启动）的刷新失败处理：认证失效广播给前端引导重登，
/// 否则用户不主动点刷新就永远看不到 Cookie 已失效，看板会停在陈旧数据
fn report_refresh_error(app: &tauri::AppHandle, source: &str, error: AppError) {
    log::error!("{source}刷新失败: {error:?}");
    if error.is_auth()
        && let Err(emit_error) = app.emit("auth://expired", ())
    {
        log::error!("推送认证失效事件失败: {emit_error}");
    }
}

/// 检查并安装更新：从 GitHub Releases 拉取 latest.json，发现新版本即下载并安装，
/// 安装完成后调用 tauri 核心 restart() 重启应用。
/// 开发模式（debug 构建）与无网环境下静默跳过，失败仅记日志。
async fn check_for_updates(app: &tauri::AppHandle) {
    if cfg!(debug_assertions) {
        return; // dev 运行不检查更新
    }
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            log::error!("初始化更新器失败: {error}");
            return;
        }
    };
    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => return, // 已是最新版本
        Err(error) => {
            log::warn!("检查更新失败: {error}");
            return;
        }
    };
    log::info!("发现新版本 v{}，开始下载安装", update.version);
    let notify_result = app
        .notification()
        .builder()
        .title("⬇️ AMAX Dashboard 有新版本")
        .body(format!(
            "v{} 正在后台下载并安装，完成后将自动重启",
            update.version
        ))
        .show();
    if let Err(error) = notify_result {
        log::warn!("更新通知发送失败: {error}");
    }
    if let Err(error) = update
        .download_and_install(
            // 回调签名 (chunk_len: usize, content_length: Option<u64>)
            |chunk_len, content_length| {
                log::debug!("更新下载进度: 块 {chunk_len} 字节，总长度 {content_length:?}");
            },
            || log::info!("更新下载完成"),
        )
        .await
    {
        log::error!("更新安装失败: {error}");
        return;
    }
    // 安装完成（MSI/NSIS 已替换文件）后重启进入新版本
    log::info!("更新安装完成，重启应用");
    app.restart();
}

/// 后台刷新：成功与否返回给调用方，供自动刷新排程决定下次间隔
async fn refresh_and_notify(app: &tauri::AppHandle) -> bool {
    match refresh_dashboard(app, true, true).await {
        Ok(_) => true,
        Err(error) => {
            report_refresh_error(app, "后台", error);
            false
        }
    }
}

/// 后台自动刷新间隔：10 分钟
const AUTO_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// 刷新失败后的快速重试：60s 起，封顶 5 分钟，成功即复位
const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_secs(60);
const RETRY_MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(5 * 60);

fn start_auto_refresh(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(AUTO_REFRESH_INTERVAL).await;
        let mut retry_delay = RETRY_BASE_DELAY;
        loop {
            let success = refresh_and_notify(&app_handle).await;
            let delay = if success {
                retry_delay = RETRY_BASE_DELAY;
                AUTO_REFRESH_INTERVAL
            } else {
                let current = retry_delay;
                retry_delay = (retry_delay * 2).min(RETRY_MAX_DELAY);
                current
            };
            tokio::time::sleep(delay).await;
        }
    });
}

fn db_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("amax_dashboard.db")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动：不新建进程，唤醒已有实例主窗口（托盘仍在原实例中）
            show_main_window(app);
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("amax.log".into()),
                    }),
                ])
                .max_file_size(5 * 1024 * 1024)
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let path = db_path(app.handle());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let db = Db::open(&path)?;
            let client = api::build_client()?;
            app.manage(AppState {
                db: Mutex::new(db),
                client,
                refresh_lock: tokio::sync::Mutex::new(()),
                low_quota_notified: Mutex::new(false),
                last_successful_refresh: Mutex::new(None),
                user_id_cache: Mutex::new(None),
            });

            let window = app.get_webview_window("main").ok_or("找不到主窗口")?;
            let event_handle = app.handle().clone();
            window.on_window_event(move |event| match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    hide_main_window(&event_handle);
                }
                tauri::WindowEvent::Resized(size) if size.width == 0 || size.height == 0 => {
                    hide_main_window(&event_handle);
                }
                _ => {}
            });

            let handle = app.handle().clone();
            build_tray(&handle)?;
            start_auto_refresh(&handle);

            // 启动 15 秒后自动检查更新（debug 构建跳过）；托盘菜单可手动触发
            let updater_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                check_for_updates(&updater_handle).await;
            });

            // 启动时显式申请一次系统通知权限
            // （Windows 上 permission_state 恒为 Granted，此检查仅防平台差异；失败不阻塞启动）
            let notify_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let notification = notify_handle.notification();
                if !matches!(
                    notification.permission_state(),
                    Ok(tauri_plugin_notification::PermissionState::Granted)
                ) && let Err(error) = notification.request_permission()
                {
                    log::error!("申请系统通知权限失败: {error}");
                }
            });

            let startup_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                // 窗口可见启动时前端已触发 fetch_dashboard，10 秒内有成功刷新则跳过，
                // 避免重复网络请求；窗口隐藏/前端未加载时仍兜底刷新
                let recently_refreshed = startup_handle
                    .state::<AppState>()
                    .last_successful_refresh
                    .lock()
                    .map(|last| {
                        last.is_some_and(|time| time.elapsed() < std::time::Duration::from_secs(10))
                    })
                    .unwrap_or(false);
                if recently_refreshed {
                    return;
                }
                let has_cookie = startup_handle
                    .state::<AppState>()
                    .db
                    .lock()
                    .map(|db| db.has_cookie())
                    .unwrap_or(false);
                if has_cookie
                    && let Err(error) = refresh_dashboard(&startup_handle, false, false).await
                {
                    report_refresh_error(&startup_handle, "启动", error);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            fetch_dashboard,
            get_local_stats,
            fetch_usage_stats,
            login::open_login_window,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

#[cfg(test)]
mod tests {
    use super::error::ErrorCode;
    use super::validate_date_range;
    use chrono::{Duration, Local};

    fn today() -> chrono::NaiveDate {
        Local::now().date_naive()
    }

    #[test]
    fn valid_range_passes() {
        let end = today();
        let start = end - Duration::days(6);
        let result = validate_date_range(
            &start.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        );
        assert!(result.is_ok());
        let (s, e) = result.expect("合法区间应通过");
        assert_eq!((e - s).num_days(), 6);
    }

    #[test]
    fn start_after_end_rejected() {
        let end = today();
        let start = end + Duration::days(0);
        let error = validate_date_range(
            &(start + Duration::days(1)).format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .expect_err("start 晚于 end 应拒绝");
        assert_eq!(error.code, ErrorCode::Input);
        assert!(error.message.contains("不能晚于"));
    }

    #[test]
    fn future_end_rejected() {
        let end = today() + Duration::days(1);
        let error = validate_date_range(
            &today().format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .expect_err("未来日期应拒绝");
        assert_eq!(error.code, ErrorCode::Input);
        assert!(error.message.contains("不能超过今天"));
    }

    #[test]
    fn span_over_1096_days_rejected() {
        let end = today();
        let start = end - Duration::days(1097);
        let error = validate_date_range(
            &start.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .expect_err("超 1096 天应拒绝");
        assert_eq!(error.code, ErrorCode::Input);
        assert!(error.message.contains("1096"));

        // 恰好 1096 天应通过
        let start_ok = end - Duration::days(1096);
        assert!(
            validate_date_range(
                &start_ok.format("%Y-%m-%d").to_string(),
                &end.format("%Y-%m-%d").to_string(),
            )
            .is_ok()
        );
    }

    #[test]
    fn invalid_format_rejected() {
        for (start, end) in [
            ("2026/08/01", "2026-08-02"), // 分隔符非法
            ("2026-02-30", "2026-08-02"), // 日期不存在
            ("2026-08-01", "not-a-date"), // 结束日期不可解析
        ] {
            let error = validate_date_range(start, end).expect_err("非法日期应拒绝");
            assert_eq!(error.code, ErrorCode::Input, "{start}~{end}");
        }
    }
}
