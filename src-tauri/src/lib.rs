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
    /// 已下载验签、等待空闲安装的更新
    pending_update: Mutex<Option<StagedUpdate>>,
    /// 最近一次前端用户活动上报，窗口可见时用于空闲判定
    last_user_active: Mutex<std::time::Instant>,
    /// 启动定时/手动/周期三路检查互斥
    update_check_lock: tokio::sync::Mutex<()>,
}

/// 已下载并验签、等待空闲安装的更新。
///
/// 分成「下载」与「安装」两步是必须的：Windows 侧 `Update::install` 拉起 msiexec 后
/// 直接 `std::process::exit(0)`，正在用应用时调用等于当场杀掉进程，所以只能先下载好，
/// 等无人使用再安装。
struct StagedUpdate {
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
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

#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 前端按节流上报用户活动（见 dist/app.js 的 pingUserActivity）
#[tauri::command]
fn report_user_activity(state: tauri::State<AppState>) {
    if let Ok(mut last) = state.last_user_active.lock() {
        *last = std::time::Instant::now();
    }
}

/// 设置页「检查更新」：结果一律经 update://status 事件回前端，故不再重复发系统通知
#[tauri::command]
async fn check_for_updates_now(app: tauri::AppHandle) {
    check_for_updates(&app, false).await;
}

/// 「现在重启并安装」：用户主动点即视为空闲，跳过等待
#[tauri::command]
async fn apply_update_now(app: tauri::AppHandle) -> Result<(), AppError> {
    let staged = take_staged_update(&app).ok_or_else(|| AppError::input("没有已下载完成的更新"))?;
    install_staged(&app, staged).map_err(AppError::storage)
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
                    // 托盘触发时界面可能根本没打开，检查结论必须走系统通知
                    check_for_updates(&app, true).await;
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

fn show_notification(app: &tauri::AppHandle, title: &str, body: &str) {
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        log::warn!("系统通知发送失败: {error}");
    }
}

/// 更新状态事件：设置页据此渲染版本状态行与「现在重启并安装」按钮。
/// state ∈ disabled | checking | up_to_date | downloading | staged | installing | error
fn emit_update_status(
    app: &tauri::AppHandle,
    state: &str,
    version: Option<&str>,
    message: Option<&str>,
) {
    let payload = serde_json::json!({ "state": state, "version": version, "message": message });
    if let Err(error) = app.emit("update://status", payload) {
        log::error!("推送更新状态失败: {error}");
    }
}

/// 检查失败：状态行与系统通知都要留下痕迹，静默失败是自动更新最难查的问题
fn update_failed(app: &tauri::AppHandle, message: &str, announce_result: bool) {
    log::error!("{message}");
    emit_update_status(app, "error", None, Some(message));
    if announce_result {
        show_notification(app, "⚠️ AMAX Dashboard 更新失败", message);
    }
}

/// 检查 GitHub Releases → 后台下载并验签 → 暂存等空闲监视器安装（不在此安装，
/// 见 [`StagedUpdate`]）。开发构建无清单可对账，直接跳过。
///
/// `announce_result` 用于托盘这类「没有界面可看」的触发方：把「已是最新 / 开发版不检查」
/// 这类结论也走系统通知。新版本就绪与失败两种结果无论如何都通知——前者预告了即将重启。
async fn check_for_updates(app: &tauri::AppHandle, announce_result: bool) {
    if cfg!(debug_assertions) {
        emit_update_status(app, "disabled", None, Some("开发构建不检查更新"));
        if announce_result {
            show_notification(app, "ℹ️ AMAX Dashboard", "开发构建不检查更新");
        }
        return;
    }
    let state = app.state::<AppState>();
    let Ok(_check_guard) = state.update_check_lock.try_lock() else {
        return; // 已有一路在检查，不并发下载
    };
    if let Some(version) = staged_version(app) {
        // 上一轮已下载好，只等空闲安装，别重复拉包
        let body = staged_notice(&version);
        emit_update_status(app, "staged", Some(&version), Some(&body));
        if announce_result {
            show_notification(app, "⬇️ AMAX Dashboard 更新已就绪", &body);
        }
        return;
    }

    emit_update_status(app, "checking", None, None);
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            return update_failed(
                app,
                &format!("初始化更新器失败（多为 pubkey 配置有误）: {error}"),
                announce_result,
            );
        }
    };
    let update = match updater.check().await {
        Ok(Some(update)) => update,
        Ok(None) => {
            emit_update_status(app, "up_to_date", None, None);
            if announce_result {
                show_notification(app, "✅ AMAX Dashboard", "当前已是最新版本");
            }
            return;
        }
        Err(error) => {
            return update_failed(app, &format!("检查更新失败: {error}"), announce_result);
        }
    };
    let version = update.version.clone();
    log::info!("发现新版本 v{version}，开始后台下载并验签");
    emit_update_status(app, "downloading", Some(&version), None);
    let bytes = match update
        .download(
            // 回调签名 (chunk_len: usize, content_length: Option<u64>)
            |chunk_len, content_length| {
                log::debug!("更新下载进度: 块 {chunk_len} 字节，总长度 {content_length:?}");
            },
            || log::info!("更新下载完成"),
        )
        .await
    {
        Ok(bytes) => bytes,
        // download() 内含 minisign 验签，失败说明签名与配置公钥不匹配或包被篡改
        Err(error) => {
            return update_failed(
                app,
                &format!("下载 v{version} 失败: {error}"),
                announce_result,
            );
        }
    };
    let staged = match state.pending_update.lock() {
        Ok(mut pending) => {
            *pending = Some(StagedUpdate { update, bytes });
            true
        }
        Err(_) => false,
    };
    if !staged {
        return update_failed(app, "更新状态锁不可用", announce_result);
    }
    let body = staged_notice(&version);
    // 阈值只在 Rust 侧定义，状态文案随事件一起下发，前端不再复制一份常量
    emit_update_status(app, "staged", Some(&version), Some(&body));
    show_notification(app, "⬇️ AMAX Dashboard 更新已就绪", &body);
}

/// 「已下载、等空闲安装」的文案
fn staged_notice(version: &str) -> String {
    format!(
        "v{version} 已下载并验签，将在应用空闲时自动安装并重启（约 {} 秒无操作）",
        IDLE_AFTER.as_secs()
    )
}

fn staged_version(app: &tauri::AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let pending = state.pending_update.lock().ok()?;
    Some(pending.as_ref()?.update.version.clone())
}

fn take_staged_update(app: &tauri::AppHandle) -> Option<StagedUpdate> {
    let state = app.state::<AppState>();
    let mut pending = state.pending_update.lock().ok()?;
    pending.take()
}

/// 是否到了可以安装更新的时机：主窗口隐藏/最小化说明用户不在看，
/// 窗口可见时要求前端至少 IDLE_AFTER 没上报过活动；另外避开后台刷新在飞的瞬间。
fn update_install_is_idle(app: &tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    let being_watched = app
        .get_webview_window("main")
        .map(|window| {
            window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false)
        })
        .unwrap_or(false);
    if being_watched {
        let idle_for = state
            .last_user_active
            .lock()
            .map(|last| last.elapsed())
            .unwrap_or(std::time::Duration::MAX);
        if idle_for < IDLE_AFTER {
            return false;
        }
    }
    state.refresh_lock.try_lock().is_ok()
}

/// 安装暂存的更新。成功时不会返回：Windows 的 install() 起好 msiexec 就结束了本进程，
/// 由 MSI 的 AUTOLAUNCHAPP 拉起新版本。返回 Err 只表示「没能开始安装」，应用继续以旧版运行。
fn install_staged(app: &tauri::AppHandle, staged: StagedUpdate) -> Result<(), String> {
    let StagedUpdate { update, bytes } = staged;
    let version = update.version.clone();
    emit_update_status(app, "installing", Some(&version), None);
    log::info!("开始安装 v{version}");
    let result = update
        .install(bytes)
        .map_err(|error| format!("安装 v{version} 失败: {error}"));
    if let Err(message) = &result {
        log::error!("{message}");
        emit_update_status(app, "error", Some(&version), Some(message));
        show_notification(app, "⚠️ AMAX Dashboard 更新失败", message);
    }
    result
}

/// 空闲安装监视器：更新已下载好的那刻起就等着，一旦满足 [`update_install_is_idle`] 就装。
fn start_update_installer(app: &tauri::AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(IDLE_INSTALL_POLL).await;
            if staged_version(&handle).is_none() || !update_install_is_idle(&handle) {
                continue;
            }
            if let Some(staged) = take_staged_update(&handle) {
                let _ = install_staged(&handle, staged);
            }
        }
    });
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

/// 启动后首次检查更新的延迟
const FIRST_UPDATE_CHECK_DELAY: std::time::Duration = std::time::Duration::from_secs(15);
/// 托盘驻留的应用可能几周不重启，只做启动一次检查会让新版本永远追不上
const UPDATE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// 窗口可见时，距最近一次用户活动满这么久才算空闲（可以安装并重启）
const IDLE_AFTER: std::time::Duration = std::time::Duration::from_secs(90);
const IDLE_INSTALL_POLL: std::time::Duration = std::time::Duration::from_secs(5);

fn start_update_watch(app: &tauri::AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_UPDATE_CHECK_DELAY).await;
        loop {
            check_for_updates(&handle, false).await;
            tokio::time::sleep(UPDATE_CHECK_INTERVAL).await;
        }
    });
}

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
                pending_update: Mutex::new(None),
                last_user_active: Mutex::new(std::time::Instant::now()),
                update_check_lock: tokio::sync::Mutex::new(()),
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
            start_update_watch(&handle);
            start_update_installer(&handle);

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
            get_app_version,
            report_user_activity,
            check_for_updates_now,
            apply_update_now,
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
