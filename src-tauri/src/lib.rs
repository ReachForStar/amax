//! Tauri 命令模块 — IPC 桥接 + 托盘 + 通知 + 自动刷新

mod alert;
mod api;
mod crypto;
mod db;
mod deliver;
mod error;
mod login;

use chrono::{Days, Local, NaiveDate};
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
    /// 告警引擎状态：`last_payload` 只在负载变化时推 `alert://status`，避免每 10 分钟重复推送
    alert: Mutex<AlertState>,
    /// 投递任务已起飞、结果还没写库的规则。看板刷新每 10 分钟一轮，而一封超时邮件可能要
    /// 20 秒才回来——没有这把占位锁，同一告警会在结果落地前被下一轮重复投递
    alerts_in_flight: Mutex<std::collections::HashSet<&'static str>>,
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

/// 告警引擎的进程内可观测状态，仅用于给设置页展示「今天还剩几次额度」。
/// 去重本身靠 `alert_state` 表，不依赖这里，所以重启不会导致重复告警。
#[derive(Debug, Default)]
struct AlertState {
    /// 最近一次推给 `alert://status` 的负载，用于只在内容变化时推送
    last_payload: Option<serde_json::Value>,
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

/// 告警参数 + 启用规则 + 可调区间 + 当日进度，设置页据此渲染
#[tauri::command]
fn get_alert_settings(state: tauri::State<AppState>) -> Result<serde_json::Value, AppError> {
    let day = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
    // 读失败时 load_* 内部已按默认值降级并记日志，这里不额外报错：设置页要能打开
    let settings = alert::load_settings(&db);
    Ok(serde_json::json!({
        "settings": settings,
        "rulesEnabled": alert::load_rules_enabled(&db),
        "paramRanges": alert::param_ranges(),
        "status": alert_status(&db, &settings, &day),
    }))
}

/// 当日告警进度；账本读不到时返回 null，让前端宁可留白也不拿 0 假装「今天还没投过」
fn alert_status(db: &Db, settings: &alert::AlertSettings, day: &str) -> Option<serde_json::Value> {
    match alert_day_state(db, day) {
        Ok(day_state) => Some(alert_state_payload(settings, day, day_state.total)),
        Err(error) => {
            log::warn!("读取当日告警次数失败，设置页不显示进度: {error}");
            None
        }
    }
}

/// 保存告警参数与启用规则，回读一次真实落库的值（夹取结果以库里为准）
#[tauri::command]
fn set_alert_settings(
    state: tauri::State<AppState>,
    settings: alert::AlertSettings,
    rules_enabled: Option<Vec<String>>,
) -> Result<serde_json::Value, AppError> {
    let day = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
    let rules = alert::resolve_rules_enabled(&alert::load_rules_enabled(&db), rules_enabled)?;
    let saved = db.set_alert_settings(settings)?;
    db.set_alert_rules_enabled(&rules, &alert::rule_keys())?;
    Ok(serde_json::json!({
        "settings": saved,
        "rulesEnabled": rules,
        "status": alert_status(&db, &saved, &day),
    }))
}

/// 「发一条测试」时写投递账本用的 rule_key。真实规则的 key 只有 `alert::Rule` 那三个，
/// 用 `test` 既不会命中去重查询，也不会消耗当日额度。
const TEST_DELIVERY_RULE_KEY: &str = "test";

/// 通知渠道配置 + 当日投递明细。只有 SMTP 授权码不回显原值，昵称按明文配置回显。
#[tauri::command]
fn get_alert_channels(state: tauri::State<AppState>) -> Result<serde_json::Value, AppError> {
    let day = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
    let config = db.get_delivery_config()?;
    Ok(serde_json::json!({
        "channels": deliver::view(&config),
        "deliveries": db.recent_alert_deliveries(&day, 20)?,
    }))
}

/// 保存渠道设置。校验用「已存配置 + 本次提交」合并后的口径：凭据可以只填一次，
/// 不必每次重新提交；但开着渠道就一定得有能发出去的料。
#[tauri::command]
fn set_alert_channels(
    state: tauri::State<AppState>,
    channels: deliver::ChannelInput,
) -> Result<serde_json::Value, AppError> {
    let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
    channels.validate(&db.get_delivery_config()?)?;
    db.set_delivery_channels(&channels)?;
    Ok(deliver::view(&db.get_delivery_config()?))
}

/// 按指定渠道发一条测试消息：验证凭据与网络可用，只记账不占用当日告警额度。
#[tauri::command]
async fn test_alert_channel(
    app: tauri::AppHandle,
    channel: String,
) -> Result<serde_json::Value, AppError> {
    let channel = deliver::Channel::from_key(&channel)
        .ok_or_else(|| AppError::input("未知渠道，只能是 notification / meow / mail"))?;
    let config = {
        let state = app.state::<AppState>();
        let db = state.db.lock().map_err(|_| poisoned("数据库"))?;
        db.get_delivery_config()?
    };
    if config.availability(channel) != deliver::Availability::Ready {
        return Err(AppError::input(
            config
                .notice(channel)
                .unwrap_or_else(|| "该渠道未开启或配置不完整".to_string()),
        ));
    }
    let title = "AMAX 测试告警";
    let body = format!(
        "这是一条来自 AMAX Dashboard 的测试消息（{}渠道），用于确认凭据与网络可用。",
        channel.label()
    );
    let receipt = send_to_channel(&app, channel, title, &body, &config)
        .await
        .ok_or_else(|| AppError::input("该渠道缺少可用凭据，请重新填写"))?;
    let day = Local::now().date_naive().format("%Y-%m-%d").to_string();
    record_alert_attempt(
        &app,
        &day,
        TEST_DELIVERY_RULE_KEY,
        std::slice::from_ref(&receipt),
        false,
    );
    Ok(serde_json::json!({
        "ok": receipt.outcome.is_success(),
        "channel": channel.key(),
        "error": receipt.error,
    }))
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

/// 拉取看板 → 落快照 → 同步托盘/前端 → （可选）评估告警。
/// 两个开关独立：手动刷新要推前端但不评估告警，后台刷新两者都要。
async fn refresh_dashboard(
    app: &tauri::AppHandle,
    alerts_enabled: bool,
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

    // 告警评估放在看板推送之后：投递失败不该让刷新变成失败，前端此刻已经拿到新数据了
    if alerts_enabled {
        run_alerts(app, &data);
    }

    if let Ok(mut last) = state.last_successful_refresh.lock() {
        *last = Some(std::time::Instant::now());
    }

    Ok(data)
}

/// 当日已投次数。`day` 由调用方一次性给出（见 `run_alerts`），保证与基准用的是同一个日界
fn alert_day_state(db: &Db, day: &str) -> Result<alert::DayState, AppError> {
    let counts = db.get_alert_day_counts(day)?;
    Ok(alert::DayState::from_counts(&counts))
}

/// 评估并投递告警。整条链路只写日志不抛错——看板刷新已经成功，不能因为通知发不出去而回报失败。
///
/// 投递放在独立任务里：系统通知是即时的，MeoW 是一次 HTTP，SMTP 最坏要等满 20 秒超时。
/// 卡在刷新路径上会拖住 `refresh_lock`，让后面每一轮都排在一个坏掉的邮件服务器后面。
fn run_alerts(app: &tauri::AppHandle, data: &api::DashboardData) {
    let state = app.state::<AppState>();
    // 日界只在这里取一次：已投次数与用量基准都基于同一个 day，
    // 分两次取时间会让跨午夜的那一轮读到「新的一天 + 旧的一天的次数」而重复投递
    let day = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let evaluated = {
        let Ok(db) = state.db.lock() else {
            log::error!("读取告警状态时数据库锁不可用，跳过本轮告警");
            return;
        };
        let settings = alert::load_settings(&db);
        let rules = alert::load_rules_enabled(&db);
        match (
            alert_day_state(&db, &day),
            alert_baseline(&db, &day),
            db.get_delivery_config(),
        ) {
            (Ok(day_state), Ok(baseline), Ok(config)) => {
                Some((settings, rules, day_state, baseline, config))
            }
            _ => {
                // 基准或状态读不出来时宁可不报：误报一次比漏报一次的代价更高
                log::error!("读取告警基准/当日状态/渠道配置失败，跳过本轮告警");
                None
            }
        }
    };
    let Some((settings, rules, day_state, baseline, config)) = evaluated else {
        return;
    };

    let current = alert::Current {
        percent: data.percent,
        remaining: data.remaining,
        total: data.total,
        today_yuan: data.today_yuan,
    };
    let findings = alert::detect(&current, &baseline, &settings, &rules, &day_state);
    let usable = config.usable();
    if findings.is_empty() || usable.is_empty() {
        if !findings.is_empty() {
            // 一条渠道都没开时不消耗额度：额度记的是「今日已投出几次」，没投出去就不算投过
            log::warn!(
                "检测到 {} 条告警但没有可用渠道，本轮跳过投递（设置页里开启系统通知或补全渠道凭据）",
                findings.len()
            );
        }
        emit_alert_status_from_db(app, &state, &settings, &day);
        return;
    }

    let mut in_flight = 0usize;
    for finding in findings {
        let Ok(mut set) = state.alerts_in_flight.lock() else {
            log::error!("告警投递占位锁不可用，跳过 {}", finding.rule);
            continue;
        };
        // 上一轮的结果还没写库（例如邮件正在超时），这条告警就不重复起飞
        if !set.insert(finding.rule) {
            continue;
        }
        drop(set);
        in_flight += 1;
        let app = app.clone();
        let day = day.clone();
        let config = config.clone();
        tauri::async_runtime::spawn(async move {
            deliver_alert(app, day, settings, finding, config).await;
        });
    }
    // 有任务起飞时本轮不推额度事件：次数还没写库，推出去的是旧值。
    // 任务收尾会按库里的真实次数补一次，前端不会停在「今天还没投过」。
    if in_flight == 0 {
        emit_alert_status_from_db(app, &state, &settings, &day);
    }
}

/// 投递任务运行期间占住规则 key；任务结束（含 panic 展开）时释放。
/// 释放不了会让这条规则当天再也投不出去，只能等重启。
struct AlertSlotReleaser {
    app: tauri::AppHandle,
    rule: &'static str,
}

impl Drop for AlertSlotReleaser {
    fn drop(&mut self) {
        let Some(state) = self.app.try_state::<AppState>() else {
            return;
        };
        if let Ok(mut set) = state.alerts_in_flight.lock() {
            set.remove(self.rule);
        }
    }
}

/// 把一条告警投给所有可用渠道并记账。
///
/// 额度消耗口径：任一渠道成功即算投过；全部永久失败也算投过（否则每 10 分钟撞一次同一堵墙）；
/// 只有「还能重试」时不消耗，留给下一轮，并由 `deliver::MAX_FAILED_DELIVERIES_PER_RULE`
/// 封顶——当日失败行数到达上限后直接消耗额度收口，不再无限重试。
async fn deliver_alert(
    app: tauri::AppHandle,
    day: String,
    settings: alert::AlertSettings,
    finding: alert::Finding,
    config: deliver::DeliveryConfig,
) {
    let _releaser = AlertSlotReleaser {
        app: app.clone(),
        rule: finding.rule,
    };
    let state = app.state::<AppState>();

    let exhausted = {
        let Ok(db) = state.db.lock() else {
            log::error!("读取告警失败次数时数据库锁不可用，跳过 {}", finding.rule);
            return;
        };
        match db.count_alert_failures(finding.rule, &day) {
            Ok(failures) => failures >= deliver::MAX_FAILED_DELIVERIES_PER_RULE,
            Err(error) => {
                log::error!("读取告警失败次数失败: {error}");
                false
            }
        }
    };
    if exhausted {
        log::warn!(
            "告警 {} 当日已累计 {} 条失败投递，今天不再重试",
            finding.rule,
            deliver::MAX_FAILED_DELIVERIES_PER_RULE
        );
        record_alert_attempt(&app, &day, finding.rule, &[], true);
        emit_alert_status_from_db(&app, &state, &settings, &day);
        return;
    }

    let mut receipts = Vec::new();
    for channel in config.usable() {
        if let Some(receipt) =
            send_to_channel(&app, channel, &finding.title, &finding.message, &config).await
        {
            match receipt.outcome {
                deliver::Outcome::Sent => {
                    log::info!("告警 {} 已经 {} 投递", finding.rule, channel.label())
                }
                deliver::Outcome::Failed | deliver::Outcome::Retryable => log::warn!(
                    "告警 {} 的 {} 投递失败: {}",
                    finding.rule,
                    channel.label(),
                    receipt.error.as_deref().unwrap_or("未知原因")
                ),
            }
            receipts.push(receipt);
        }
    }
    let consume = deliver::summarize(&receipts) != deliver::Outcome::Retryable;
    record_alert_attempt(&app, &day, finding.rule, &receipts, consume);
    emit_alert_status_from_db(&app, &state, &settings, &day);
}

/// 单渠道投递。返回 `None` 表示该渠道缺凭据（`usable()` 已过滤，只有竞态时才会发生）。
async fn send_to_channel(
    app: &tauri::AppHandle,
    channel: deliver::Channel,
    title: &str,
    body: &str,
    config: &deliver::DeliveryConfig,
) -> Option<deliver::Receipt> {
    match channel {
        deliver::Channel::Notification => Some(deliver::send_notification(app, title, body)),
        deliver::Channel::Meow => {
            let nickname = config.meow_nickname.clone()?;
            let client = &app.state::<AppState>().client;
            Some(deliver::send_meow(client, &nickname, title, body).await)
        }
        deliver::Channel::Mail => {
            let target = config.mail_target()?;
            let title = title.to_string();
            let body = body.to_string();
            // lettre 是同步阻塞的：放到阻塞线程池，别占住 async runtime 的工作线程
            Some(
                match tokio::task::spawn_blocking(move || {
                    deliver::send_mail(&target, &title, &body)
                })
                .await
                {
                    Ok(receipt) => receipt,
                    Err(error) => {
                        deliver::Receipt::retryable(channel, format!("邮件投递任务异常: {error}"))
                    }
                },
            )
        }
    }
}

/// 投递结果记账：明细按渠道逐行写，`consume` 决定是否消耗当日额度。
/// 明细与去重账本同源，`alert_delivery` 因此是「实际投出了什么」的事实记录。
fn record_alert_attempt(
    app: &tauri::AppHandle,
    day: &str,
    rule: &'static str,
    receipts: &[deliver::Receipt],
    consume: bool,
) {
    let state = app.state::<AppState>();
    let Ok(db) = state.db.lock() else {
        log::error!("记录告警投递结果时数据库锁不可用");
        return;
    };
    let now = Local::now().to_rfc3339();
    for receipt in receipts {
        let ok = receipt.outcome.is_success();
        if let Err(error) = db.record_alert_delivery(
            rule,
            day,
            receipt.channel.key(),
            ok,
            (!ok).then_some(receipt.error.as_deref()).flatten(),
            &now,
        ) {
            log::error!("写入告警投递记录失败: {error}");
        }
    }
    if consume && let Err(error) = db.record_alert_fire(rule, day, &now) {
        log::error!("写入告警去重状态失败: {error}");
    }
}

/// 以库为准推一次当日额度状态：投递任务可能刚消耗过额度，内存里推不出准确次数
fn emit_alert_status_from_db(
    app: &tauri::AppHandle,
    state: &AppState,
    settings: &alert::AlertSettings,
    day: &str,
) {
    let fired_today = {
        let Ok(db) = state.db.lock() else {
            log::warn!("读取当日告警次数时数据库锁不可用，跳过本轮状态事件");
            return;
        };
        match alert_day_state(&db, day) {
            Ok(day_state) => day_state.total,
            Err(error) => {
                log::warn!("读取当日告警次数失败，跳过本轮状态事件: {error}");
                return;
            }
        }
    };
    emit_alert_state_event(app, state, settings, day, fired_today);
}

/// 用量基准：取窗口内的日末快照，排除当日与空缺日（口径见 `alert::build_baseline`）
fn alert_baseline(db: &Db, day: &str) -> Result<alert::Baseline, AppError> {
    let today = NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| AppError::storage(format!("当日日期格式异常: {day}")))?;
    let start = today
        .checked_sub_days(Days::new(alert::BASELINE_WINDOW_DAYS as u64))
        .ok_or_else(|| AppError::storage("告警基准窗口超出可表示日期范围"))?
        .format("%Y-%m-%d")
        .to_string();
    let snapshots = db.get_daily_snapshots(&start, day)?;
    alert::build_baseline(&snapshots, day, alert::BASELINE_WINDOW_DAYS)
}

/// 当日告警额度事件的负载（三种状态字段一致，前端无需分支解析）
fn alert_state_payload(
    settings: &alert::AlertSettings,
    day: &str,
    fired_today: i64,
) -> serde_json::Value {
    let name = if !settings.enabled {
        "disabled"
    } else if fired_today >= settings.max_fires_per_day {
        "cap_hit"
    } else {
        "armed"
    };
    serde_json::json!({
        "state": name,
        "day": day,
        "firedToday": fired_today,
        "cap": settings.max_fires_per_day,
    })
}

/// 推送当日告警额度状态；只在内容与上次推出的一模一样时跳过，
/// 这样「今天又多投一次」能推出去，而每 10 分钟一次的无变化刷新不会重复推。
fn emit_alert_state_event(
    app: &tauri::AppHandle,
    state: &AppState,
    settings: &alert::AlertSettings,
    day: &str,
    fired_today: i64,
) {
    let payload = alert_state_payload(settings, day, fired_today);
    let Ok(mut last) = state.alert.lock() else {
        log::warn!("告警状态锁不可用，跳过本轮状态事件");
        return;
    };
    if last.last_payload.as_ref() == Some(&payload) {
        return;
    }
    last.last_payload = Some(payload.clone());
    drop(last);
    if let Err(error) = app.emit("alert://status", payload) {
        log::warn!("推送告警状态失败: {error}");
    }
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

/// 后台刷新：成功与否返回给调用方，供自动刷新排程决定下次间隔。
/// 第二个参数是「本轮是否评估告警」：目前只有后台周期刷新传 true，
/// 前端的 `fetch_dashboard` 与启动兜底刷新都不评估（沿用旧版低余额通知的触发范围）。
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
                alert: Mutex::new(AlertState::default()),
                alerts_in_flight: Mutex::new(std::collections::HashSet::new()),
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
            get_alert_settings,
            set_alert_settings,
            get_alert_channels,
            set_alert_channels,
            test_alert_channel,
            report_user_activity,
            check_for_updates_now,
            apply_update_now,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

#[cfg(test)]
mod tests {
    use super::alert;
    use super::alert_state_payload;
    use super::error::ErrorCode;
    use super::validate_date_range;
    use chrono::{Duration, Local};

    fn settings() -> alert::AlertSettings {
        alert::AlertSettings::default()
    }

    #[test]
    fn alert_state_event_reports_disabled_but_keeps_counts() {
        let disabled = alert::AlertSettings {
            enabled: false,
            ..settings()
        };
        let payload = alert_state_payload(&disabled, "2026-09-22", 5);
        assert_eq!(payload["state"], "disabled");
        assert_eq!(payload["firedToday"], 5, "关闭状态也如实报当日次数");
    }

    #[test]
    fn alert_state_event_marks_cap_and_resets_with_new_day() {
        // 默认上限 2 次
        assert_eq!(
            alert_state_payload(&settings(), "2026-09-22", 0)["state"],
            "armed"
        );
        assert_eq!(
            alert_state_payload(&settings(), "2026-09-22", 1)["state"],
            "armed"
        );
        assert_eq!(
            alert_state_payload(&settings(), "2026-09-22", 2)["state"],
            "cap_hit"
        );
        assert_eq!(
            alert_state_payload(&settings(), "2026-09-22", 3)["state"],
            "cap_hit"
        );

        let next_day = alert_state_payload(&settings(), "2026-09-23", 0);
        assert_eq!(next_day["state"], "armed", "次日次数归零后应重新可用");
        assert_eq!(
            next_day["day"], "2026-09-23",
            "负载带 day，前端据此复位展示"
        );
    }

    #[test]
    fn alert_state_payload_fields_are_state_independent() {
        // 三种状态的字段集合必须一致：前端按固定字段渲染，不需要按 state 分支取值
        let variants = [
            alert_state_payload(&settings(), "2026-09-22", 0),
            alert_state_payload(&settings(), "2026-09-22", 9),
            alert_state_payload(
                &alert::AlertSettings {
                    enabled: false,
                    ..settings()
                },
                "2026-09-22",
                0,
            ),
        ];
        for payload in variants {
            let object = payload.as_object().expect("负载应是对象");
            // 排序后比较：不依赖 serde_json 的 Map 是否开了 preserve_order
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["cap", "day", "firedToday", "state"]);
        }
    }

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
