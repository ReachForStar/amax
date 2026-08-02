//! Tauri 命令模块 — IPC 桥接 + 托盘 + 通知 + 自动刷新

mod api;
mod crypto;
mod db;

use chrono::{Local, NaiveDate};
use db::Db;
use std::sync::Mutex;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

pub struct AppState {
    db: Mutex<Db>,
    client: reqwest::Client,
    refresh_lock: tokio::sync::Mutex<()>,
    low_quota_notified: Mutex<bool>,
}

/// 校验统计区间：格式、先后、不超今天、跨度上限 1096 天
fn validate_date_range(start_date: &str, end_date: &str) -> Result<(NaiveDate, NaiveDate), String> {
    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .map_err(|_| "起始日期格式无效，应为 YYYY-MM-DD".to_string())?;
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .map_err(|_| "结束日期格式无效，应为 YYYY-MM-DD".to_string())?;
    if start > end {
        return Err("起始日期不能晚于结束日期".into());
    }
    if end > Local::now().date_naive() {
        return Err("结束日期不能超过今天".into());
    }
    if (end - start).num_days() > 1096 {
        return Err("区间跨度不能超过 1096 天".into());
    }
    Ok((start, end))
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Result<serde_json::Value, String> {
    let db = state
        .db
        .lock()
        .map_err(|_| "数据库状态锁已损坏".to_string())?;
    Ok(serde_json::json!({
        "has_cookie": db.has_cookie(),
        "has_api_key": db.has_api_key(),
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
) -> Result<LocalStats, String> {
    validate_date_range(&start_date, &end_date)?;
    let db = state
        .db
        .lock()
        .map_err(|_| "数据库状态锁已损坏".to_string())?;
    let snapshots = db
        .get_daily_snapshots(&start_date, &end_date)
        .map_err(|error| format!("查询本地快照失败: {error}"))?;
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
) -> Result<api::UsageStats, String> {
    let (start, end) = validate_date_range(&start_date, &end_date)?;
    let cookie = {
        let db = state
            .db
            .lock()
            .map_err(|_| "数据库状态锁已损坏".to_string())?;
        db.get_cookie()
    };
    api::fetch_usage_stats(&state.client, &cookie.unwrap_or_default(), start, end).await
}

#[tauri::command]
fn save_config(
    state: tauri::State<AppState>,
    cookie: String,
    api_key: String,
) -> Result<(), String> {
    if cookie.is_empty() && api_key.is_empty() {
        return Err("请至少填写一项认证信息".into());
    }
    let mut db = state
        .db
        .lock()
        .map_err(|_| "数据库状态锁已损坏".to_string())?;
    db.save_config(&cookie, &api_key)
}

#[tauri::command]
async fn fetch_dashboard(app: tauri::AppHandle) -> Result<api::DashboardData, String> {
    refresh_dashboard(&app, false, false).await
}

fn hide_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main")
        && let Err(error) = window.set_skip_taskbar(true).and_then(|_| window.hide())
    {
        eprintln!("隐藏主窗口失败: {error}");
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
        eprintln!("显示主窗口失败: {error}");
    }
}

fn update_tray_tooltip(app: &tauri::AppHandle, data: &api::DashboardData) {
    if let Some(tray) = app.tray_by_id("main")
        && let Err(error) = tray.set_tooltip(Some(&format!(
            "AMAX Dashboard\n今日消耗 ¥{:.6}\n剩余额度 ¥{:.2}（{:.1}%）",
            data.today_yuan, data.remaining, data.percent
        )))
    {
        eprintln!("更新托盘提示失败: {error}");
    }
}

async fn refresh_dashboard(
    app: &tauri::AppHandle,
    notify_low_quota: bool,
    emit_update: bool,
) -> Result<api::DashboardData, String> {
    let state = app.state::<AppState>();
    let _refresh_guard = state.refresh_lock.lock().await;
    let cookie = {
        let db = state
            .db
            .lock()
            .map_err(|_| "数据库状态锁已损坏".to_string())?;
        db.get_cookie().unwrap_or_default()
    };
    let data = api::fetch_dashboard(&state.client, &cookie).await?;

    {
        let db = state
            .db
            .lock()
            .map_err(|_| "数据库状态锁已损坏".to_string())?;
        db.save_snapshot(
            data.today_yuan,
            data.today_tokens,
            data.remaining,
            data.used,
            data.total,
            data.request_count,
        )
        .map_err(|error| format!("保存数据快照失败: {error}"))?;
    }

    update_tray_tooltip(app, &data);
    if emit_update {
        app.emit("dashboard-updated", &data)
            .map_err(|error| format!("推送看板更新失败: {error}"))?;
    }

    let should_notify = {
        let mut notified = state
            .low_quota_notified
            .lock()
            .map_err(|_| "通知状态锁已损坏".to_string())?;
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
            .map_err(|error| format!("发送额度通知失败: {error}"))?;
    }

    Ok(data)
}

fn build_tray(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let refresh = MenuItemBuilder::with_id("tray_refresh", "🔄 刷新数据").build(app)?;
    let show = MenuItemBuilder::with_id("tray_show", "📊 显示窗口").build(app)?;
    let quit = MenuItemBuilder::with_id("tray_quit", "❌ 退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&refresh)
        .item(&show)
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

async fn refresh_and_notify(app: &tauri::AppHandle) {
    if let Err(error) = refresh_dashboard(app, true, true).await {
        eprintln!("后台刷新失败: {error}");
    }
}

/// 后台自动刷新间隔：10 分钟
const AUTO_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

fn start_auto_refresh(app: &tauri::AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(AUTO_REFRESH_INTERVAL).await;
        loop {
            refresh_and_notify(&app_handle).await;
            tokio::time::sleep(AUTO_REFRESH_INTERVAL).await;
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
        .setup(|app| {
            let path = db_path(app.handle());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let db = Db::open(&path)?;
            let client = api::build_client().map_err(std::io::Error::other)?;
            app.manage(AppState {
                db: Mutex::new(db),
                client,
                refresh_lock: tokio::sync::Mutex::new(()),
                low_quota_notified: Mutex::new(false),
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

            let startup_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let has_cookie = startup_handle
                    .state::<AppState>()
                    .db
                    .lock()
                    .map(|db| db.has_cookie())
                    .unwrap_or(false);
                if has_cookie
                    && let Err(error) = refresh_dashboard(&startup_handle, false, false).await
                {
                    eprintln!("启动刷新失败: {error}");
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
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

#[cfg(test)]
mod tests {
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
        assert!(error.contains("不能晚于"));
    }

    #[test]
    fn future_end_rejected() {
        let end = today() + Duration::days(1);
        let error = validate_date_range(
            &today().format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .expect_err("未来日期应拒绝");
        assert!(error.contains("不能超过今天"));
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
        assert!(error.contains("1096"));

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
        assert!(validate_date_range("2026/08/01", "2026-08-02").is_err());
        assert!(validate_date_range("2026-02-30", "2026-08-02").is_err());
    }
}
