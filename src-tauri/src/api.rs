//! HTTP API 模块 — 调用 AMAX 后端接口

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

const BASE_URL: &str = "https://ai.amaxsmp.com";
pub const QUOTA_PER_YUAN: f64 = 500_000.0;

#[derive(Deserialize)]
struct ApiEnvelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct UserInfo {
    id: i64,
    quota: i64,
    used_quota: i64,
    request_count: i64,
    display_name: String,
}

#[derive(Debug, Default, Deserialize)]
struct UsageSummary {
    #[serde(default)]
    quota: f64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

/// by-model 响应中的模型维度明细（原始字段）
#[derive(Debug, Default, Deserialize)]
struct ModelUsageRaw {
    model: String,
    #[serde(default)]
    request_count: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    quota: f64,
}

/// by-model 响应中的按日明细（原始字段）
#[derive(Debug, Default, Deserialize)]
struct DailyUsageRaw {
    /// 本地时区当日 0 点的 Unix 时间戳
    date: i64,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    quota: f64,
}

#[derive(Debug, Default, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    summary: UsageSummary,
    #[serde(default)]
    models: Vec<ModelUsageRaw>,
    #[serde(default)]
    daily: Vec<DailyUsageRaw>,
}

/// 统计页每日消耗（官方口径，已补零）
#[derive(Debug, Clone, Serialize)]
pub struct UsageDaily {
    pub date: String,
    pub yuan: f64,
    pub tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// 统计页模型分布项
#[derive(Debug, Clone, Serialize)]
pub struct UsageModel {
    pub model: String,
    pub yuan: f64,
    pub tokens: i64,
    pub request_count: i64,
    pub percent: f64,
}

/// 统计页区间汇总
#[derive(Debug, Clone, Serialize)]
pub struct UsageSummaryStats {
    pub total_yuan: f64,
    pub avg_yuan: f64,
    pub peak_yuan: f64,
    pub peak_date: String,
    pub total_tokens: i64,
    pub request_count: i64,
    pub days_with_usage: usize,
}

/// 统计页官方数据聚合结果
#[derive(Debug, Clone, Serialize)]
pub struct UsageStats {
    pub daily: Vec<UsageDaily>,
    pub models: Vec<UsageModel>,
    pub summary: UsageSummaryStats,
    pub percent_basis: String,
}

/// 看板汇总数据 (可序列化, 返回给前端)
#[derive(Debug, Clone, Serialize)]
pub struct DashboardData {
    pub display_name: String,
    pub request_count: i64,
    pub today_yuan: f64,
    pub today_tokens: i64,
    pub today_input: i64,
    pub today_output: i64,
    pub log_count: Option<usize>,
    pub remaining: f64,
    pub used: f64,
    pub total: f64,
    pub percent: f64,
    pub logs_available: bool,
}

pub fn build_client() -> Result<reqwest::Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    headers.insert("x-company", HeaderValue::from_static("AMAX"));

    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| format!("HTTP 客户端初始化失败: {error}"))
}

fn date_to_string(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// 聚合官网 by-model 响应：按日跨模型求和、补零、派生汇总与占比
fn aggregate_usage(response: UsageResponse, start: NaiveDate, end: NaiveDate) -> UsageStats {
    // 按日聚合（BTreeMap 天然有序）
    let mut by_day: BTreeMap<NaiveDate, UsageDaily> = BTreeMap::new();
    let mut days_with_usage = 0_usize;
    let mut seen: std::collections::BTreeSet<NaiveDate> = std::collections::BTreeSet::new();
    for raw in &response.daily {
        let Some(date) = Local
            .timestamp_opt(raw.date, 0)
            .single()
            .map(|dt| dt.date_naive())
        else {
            continue;
        };
        if seen.insert(date) {
            days_with_usage += 1;
        }
        let entry = by_day.entry(date).or_insert_with(|| UsageDaily {
            date: date_to_string(date),
            yuan: 0.0,
            tokens: 0,
            input_tokens: 0,
            output_tokens: 0,
        });
        entry.yuan += raw.quota / QUOTA_PER_YUAN;
        entry.tokens += raw.total_tokens;
        entry.input_tokens += raw.input_tokens;
        entry.output_tokens += raw.output_tokens;
    }

    // 逐日补齐区间，无使用日填 0
    let mut daily = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        let row = by_day.remove(&cursor).unwrap_or_else(|| UsageDaily {
            date: date_to_string(cursor),
            yuan: 0.0,
            tokens: 0,
            input_tokens: 0,
            output_tokens: 0,
        });
        daily.push(row);
        cursor += chrono::Duration::days(1);
    }

    // 汇总派生
    let total_yuan: f64 = daily.iter().map(|d| d.yuan).sum();
    let total_tokens: i64 = daily.iter().map(|d| d.tokens).sum();
    let avg_yuan = if days_with_usage > 0 {
        total_yuan / days_with_usage as f64
    } else {
        0.0
    };
    let mut peak_yuan = 0.0_f64;
    let mut peak_found = false;
    let mut peak_date = date_to_string(end); // 全 0 回退 end_date
    for row in &daily {
        if !peak_found || row.yuan > peak_yuan {
            peak_yuan = row.yuan;
            peak_found = true;
            if row.yuan > 0.0 {
                peak_date = row.date.clone();
            }
        }
    }
    let request_count: i64 = response.models.iter().map(|m| m.request_count).sum();

    // 一致性校验：补零后累计应等于 summary 上报值
    if (total_tokens - response.summary.total_tokens).abs() > 0 {
        log::warn!(
            "官方用量聚合 Token 累计({total_tokens})与 summary({})不一致，保留按日聚合值",
            response.summary.total_tokens
        );
    }

    // 模型占比：quota 全 0 时切换 tokens 口径
    let total_quota: f64 = response.models.iter().map(|m| m.quota).sum();
    let use_tokens_basis = total_quota <= 0.0;
    let percent_basis = if use_tokens_basis { "tokens" } else { "quota" };
    let basis_total: f64 = response
        .models
        .iter()
        .map(|m| {
            if use_tokens_basis {
                m.total_tokens as f64
            } else {
                m.quota
            }
        })
        .sum();
    let mut models: Vec<UsageModel> = response
        .models
        .iter()
        .map(|m| {
            let basis_value = if use_tokens_basis {
                m.total_tokens as f64
            } else {
                m.quota
            };
            let percent = if basis_total > 0.0 {
                (basis_value / basis_total * 1000.0).round() / 10.0
            } else {
                0.0
            };
            UsageModel {
                model: m.model.clone(),
                yuan: m.quota / QUOTA_PER_YUAN,
                tokens: m.total_tokens,
                request_count: m.request_count,
                percent,
            }
        })
        .collect();
    // 按主指标降序
    models.sort_by(|a, b| {
        let (av, bv) = if use_tokens_basis {
            (a.tokens as f64, b.tokens as f64)
        } else {
            (a.yuan, b.yuan)
        };
        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
    });

    UsageStats {
        daily,
        models,
        summary: UsageSummaryStats {
            total_yuan,
            avg_yuan,
            peak_yuan,
            peak_date,
            total_tokens,
            request_count,
            days_with_usage,
        },
        percent_basis: percent_basis.to_string(),
    }
}

/// 获取账户信息（含认证错误分级），供看板与统计共用
async fn fetch_user_info(client: &reqwest::Client, cookie: &str) -> Result<UserInfo, String> {
    let response = client
        .get(format!("{BASE_URL}/api/user/self"))
        .header("Cookie", cookie)
        .send()
        .await
        .map_err(|error| format!("网络连接失败, 请检查网络或代理设置: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let detail = match status.as_u16() {
            401 | 403 => "Cookie 无效或已过期, 请从浏览器重新获取".to_string(),
            429 => "请求过于频繁, 请稍后重试".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("认证失败: {detail}"));
    }

    let envelope: ApiEnvelope<UserInfo> = response
        .json()
        .await
        .map_err(|error| format!("数据解析失败: {error}"))?;
    Ok(envelope.data)
}

/// 当日用量查询结果（by-model 失败时保留账户额度数据，不致命）
#[derive(Default)]
struct TodayUsage {
    summary: UsageSummary,
    log_count: Option<usize>,
    logs_available: bool,
}

/// 查询当日用量汇总；失败仅记录日志并返回空值，不向上抛错（与账户额度解耦）
async fn fetch_today_usage(client: &reqwest::Client, cookie: &str, user_id: i64) -> TodayUsage {
    let now = Local::now();
    let Some(start_time) = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .map(|dt| dt.timestamp())
    else {
        log::error!("无法计算本地日期起点，跳过当日用量查询");
        return TodayUsage::default();
    };
    let Some(end_time) = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 23, 59, 59)
        .single()
        .map(|dt| dt.timestamp())
    else {
        log::error!("无法计算本地日期终点，跳过当日用量查询");
        return TodayUsage::default();
    };

    let mut summary = UsageSummary::default();
    let mut logs_available = false;
    let mut log_count: Option<usize> = None;
    let response = client
        .post(format!("{BASE_URL}/v1/logs/token-usage/by-model"))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "start_time": start_time,
            "end_time": end_time,
            "user_id": user_id.to_string(),
            "status": "success",
        }))
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => {
            let status = response.status();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("未知")
                .to_string();
            match response.bytes().await {
                Ok(bytes) => match serde_json::from_slice::<UsageResponse>(&bytes) {
                    Ok(payload) => {
                        summary = payload.summary;
                        // 当日成功请求数 = 各模型 request_count 之和（与统计页口径一致）
                        log_count = Some(
                            payload
                                .models
                                .iter()
                                .map(|m| m.request_count)
                                .sum::<i64>()
                                .max(0) as usize,
                        );
                        logs_available = true;
                    }
                    Err(error) => {
                        let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
                        log::error!(
                            "日志汇总 JSON 解析失败，保留账户额度数据: status={status}, content-type={content_type}, body={preview:?}, error={error}"
                        );
                    }
                },
                Err(error) => log::error!(
                    "读取日志汇总响应失败，保留账户额度数据: status={status}, content-type={content_type}, error={error}"
                ),
            }
        }
        Ok(response) => {
            let status = response.status();
            let detail = response
                .text()
                .await
                .unwrap_or_else(|error| format!("无法读取错误响应: {error}"));
            let preview: String = detail.chars().take(512).collect();
            log::error!(
                "日志汇总查询返回异常状态，保留账户额度数据: status={status}, body={preview:?}"
            );
        }
        Err(error) => log::error!("日志汇总查询失败，保留账户额度数据: {error}"),
    }

    TodayUsage {
        summary,
        log_count,
        logs_available,
    }
}

/// 由账户信息与当日用量组装看板数据（纯函数，便于测试）
fn assemble_dashboard(user: UserInfo, usage: TodayUsage) -> DashboardData {
    let total_quota = user.quota.saturating_add(user.used_quota);
    let remaining = user.quota as f64 / QUOTA_PER_YUAN;
    let used = user.used_quota as f64 / QUOTA_PER_YUAN;
    let total = total_quota as f64 / QUOTA_PER_YUAN;
    let percent = if total_quota > 0 {
        (user.quota as f64 / total_quota as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    DashboardData {
        display_name: user.display_name,
        request_count: user.request_count,
        today_yuan: usage.summary.quota / QUOTA_PER_YUAN,
        today_tokens: usage.summary.total_tokens,
        today_input: usage.summary.input_tokens,
        today_output: usage.summary.output_tokens,
        log_count: usage.log_count,
        remaining,
        used,
        total,
        percent,
        logs_available: usage.logs_available,
    }
}

/// 拉取看板数据；返回 (数据, 当前 user_id) 供调用方缓存。
/// cached_user_id 有效时并行发账户信息与当日用量请求，省一个 RTT；
/// 账户 id 与缓存不一致（更换 Cookie）时按新 id 重查用量，保证数据正确。
pub async fn fetch_dashboard(
    client: &reqwest::Client,
    cookie: &str,
    cached_user_id: Option<i64>,
) -> Result<(DashboardData, i64), String> {
    if cookie.is_empty() {
        return Err("认证失败: 未配置 Cookie".into());
    }

    let Some(cached_id) = cached_user_id else {
        // 无缓存：串行取账户信息后查询用量
        let user = fetch_user_info(client, cookie).await?;
        let user_id = user.id;
        let usage = fetch_today_usage(client, cookie, user_id).await;
        return Ok((assemble_dashboard(user, usage), user_id));
    };

    // 有缓存：并行发两个请求，账户信息校验 id 一致性
    let (user_result, usage) = tokio::join!(
        fetch_user_info(client, cookie),
        fetch_today_usage(client, cookie, cached_id),
    );
    let user = user_result?;
    let user_id = user.id;
    let usage = if user_id == cached_id {
        usage
    } else {
        // 账户已切换：按新 id 重查当日用量
        log::warn!(
            "user_id 缓存失效（{cached_id} → {}），按新 id 重查用量",
            user_id
        );
        fetch_today_usage(client, cookie, user_id).await
    };
    Ok((assemble_dashboard(user, usage), user_id))
}

/// 拉取区间用量并聚合为统计页数据
pub async fn fetch_usage_stats(
    client: &reqwest::Client,
    cookie: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<UsageStats, String> {
    if cookie.is_empty() {
        return Err("认证失败: 未配置 Cookie".into());
    }
    let user = fetch_user_info(client, cookie).await?;

    let start_time = start
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).single())
        .map(|dt| dt.timestamp())
        .ok_or_else(|| "无法计算区间起点时间戳".to_string())?;
    let end_time = end
        .and_hms_opt(23, 59, 59)
        .and_then(|naive| Local.from_local_datetime(&naive).single())
        .map(|dt| dt.timestamp())
        .ok_or_else(|| "无法计算区间终点时间戳".to_string())?;

    let response = client
        .post(format!("{BASE_URL}/v1/logs/token-usage/by-model"))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "start_time": start_time,
            "end_time": end_time,
            "user_id": user.id.to_string(),
            "status": "success",
        }))
        .send()
        .await
        .map_err(|error| format!("网络连接失败, 请检查网络或代理设置: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let detail = match status.as_u16() {
            401 | 403 => "Cookie 无效或已过期, 请从浏览器重新获取".to_string(),
            429 => "请求过于频繁, 请稍后重试".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(format!("用量查询失败: {detail}"));
    }

    let payload: UsageResponse = response
        .json()
        .await
        .map_err(|error| format!("用量数据解析失败: {error}"))?;
    Ok(aggregate_usage(payload, start, end))
}

#[cfg(test)]
mod tests {
    use super::{QUOTA_PER_YUAN, UsageResponse};
    use chrono::NaiveDate;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
    }

    fn raw_quota_model(name: &str, quota: f64, tokens: i64, requests: i64) -> super::ModelUsageRaw {
        super::ModelUsageRaw {
            model: name.into(),
            request_count: requests,
            total_tokens: tokens,
            quota,
        }
    }

    fn raw_daily(date: NaiveDate, quota: f64, tokens: i64) -> super::DailyUsageRaw {
        // 固定以本地时区当日 0 点构造时间戳，aggregate 内部用 Local 换算，
        // 保证任意时区的测试机都能往返一致
        super::DailyUsageRaw {
            date: date
                .and_hms_opt(0, 0, 0)
                .expect("合法时间")
                .and_local_timezone(chrono::Local)
                .single()
                .expect("本地时区换算应成功")
                .timestamp(),
            input_tokens: tokens / 2,
            output_tokens: tokens - tokens / 2,
            total_tokens: tokens,
            quota,
        }
    }

    #[test]
    fn aggregate_fills_missing_days_and_merges_models() {
        let d1 = day(2026, 8, 1);
        let d3 = day(2026, 8, 3);
        let response = super::UsageResponse {
            summary: super::UsageSummary::default(),
            models: vec![
                raw_quota_model("model-a", 3.0, 300, 10),
                raw_quota_model("model-b", 1.0, 100, 4),
            ],
            // 8-01 两条跨模型记录，8-02 无记录，8-03 一条
            // quota 为官方单位（QUOTA_PER_YUAN quota = 1 元），聚合后换算为元
            daily: vec![
                raw_daily(d1, 2.0 * QUOTA_PER_YUAN, 200),
                {
                    let mut r = raw_daily(d1, 1.0 * QUOTA_PER_YUAN, 100);
                    r.input_tokens = 50;
                    r.output_tokens = 50;
                    r
                },
                raw_daily(d3, 1.0 * QUOTA_PER_YUAN, 100),
            ],
        };

        let stats = super::aggregate_usage(response, d1, d3);

        // 三日连续，8-02 补 0
        assert_eq!(stats.daily.len(), 3);
        assert_eq!(stats.daily[0].date, "2026-08-01");
        assert!((stats.daily[0].yuan - 3.0).abs() < 1e-9); // 跨模型求和
        assert_eq!(stats.daily[0].tokens, 300);
        assert_eq!(stats.daily[1].date, "2026-08-02");
        assert_eq!(stats.daily[1].tokens, 0);
        assert_eq!(stats.daily[2].date, "2026-08-03");

        // 汇总：累计 4 元 / 2 个有使用日 / 日均 2 元 / 峰值 3 元在 8-01
        assert!((stats.summary.total_yuan - 4.0).abs() < 1e-9);
        assert_eq!(stats.summary.days_with_usage, 2);
        assert!((stats.summary.avg_yuan - 2.0).abs() < 1e-9);
        assert!((stats.summary.peak_yuan - 3.0).abs() < 1e-9);
        assert_eq!(stats.summary.peak_date, "2026-08-01");
        assert_eq!(stats.summary.total_tokens, 400);
        assert_eq!(stats.summary.request_count, 14);

        // quota 非 0：按 quota 占比，model-a 75%，降序在前
        assert_eq!(stats.percent_basis, "quota");
        assert_eq!(stats.models[0].model, "model-a");
        assert!((stats.models[0].percent - 75.0).abs() < 1e-9);
        assert!((stats.models[1].percent - 25.0).abs() < 1e-9);
    }

    #[test]
    fn aggregate_switches_to_tokens_basis_when_quota_zero() {
        let d1 = day(2026, 8, 1);
        let response = super::UsageResponse {
            summary: super::UsageSummary::default(),
            models: vec![
                raw_quota_model("model-a", 0.0, 300, 10),
                raw_quota_model("model-b", 0.0, 100, 4),
            ],
            daily: vec![raw_daily(d1, 0.0, 400)],
        };

        let stats = super::aggregate_usage(response, d1, d1);

        assert_eq!(stats.percent_basis, "tokens");
        assert_eq!(stats.models[0].model, "model-a");
        assert!((stats.models[0].percent - 75.0).abs() < 1e-9);
        assert!((stats.models[1].percent - 25.0).abs() < 1e-9);
        // 全 0 时峰值日期回退 end_date
        assert_eq!(stats.summary.peak_date, "2026-08-01");
    }

    #[test]
    fn aggregate_empty_response_covers_full_range_with_zero() {
        let start = day(2026, 8, 1);
        let end = day(2026, 8, 3);
        let stats = super::aggregate_usage(super::UsageResponse::default(), start, end);

        assert_eq!(stats.daily.len(), 3);
        assert!(stats.daily.iter().all(|d| d.tokens == 0 && d.yuan == 0.0));
        assert!(stats.models.is_empty());
        assert_eq!(stats.summary.days_with_usage, 0);
        assert_eq!(stats.summary.avg_yuan, 0.0);
        assert_eq!(stats.summary.peak_date, "2026-08-03"); // 全 0 回退 end_date
    }

    #[test]
    fn parses_usage_summary() {
        let payload: UsageResponse = serde_json::from_str(
            r#"{
                "summary": {
                    "input_tokens": 25989178,
                    "output_tokens": 83375,
                    "completion_tokens": 83375,
                    "total_tokens": 26072553,
                    "quota": 7006999.0
                }
            }"#,
        )
        .expect("应解析官网日志汇总响应");

        assert_eq!(payload.summary.input_tokens, 25_989_178);
        assert_eq!(payload.summary.output_tokens, 83_375);
        assert_eq!(payload.summary.total_tokens, 26_072_553);
        assert!((payload.summary.quota / QUOTA_PER_YUAN - 14.013998).abs() < 1e-9);
    }

    #[test]
    fn parses_usage_response_with_models_and_daily() {
        let payload: super::UsageResponse = serde_json::from_str(
            r#"{
                "start_time": 1785081600,
                "end_time": 1785686399,
                "status": "success",
                "summary": {
                    "input_tokens": 88062962,
                    "output_tokens": 1873797,
                    "completion_tokens": 1873797,
                    "total_tokens": 89936759,
                    "quota": 0.0
                },
                "models": [
                    {
                        "model": "qwen3.8-max-preview",
                        "request_count": 2003,
                        "input_tokens": 88062962,
                        "output_tokens": 1873797,
                        "completion_tokens": 1873797,
                        "total_tokens": 89936759,
                        "quota": 0.0
                    }
                ],
                "daily": [
                    {
                        "date": 1785081600,
                        "model": "qwen3.8-max-preview",
                        "input_tokens": 5276286,
                        "output_tokens": 358085,
                        "completion_tokens": 358085,
                        "total_tokens": 5634371,
                        "quota": 0.0
                    }
                ]
            }"#,
        )
        .expect("应解析含 models 与 daily 的实测响应");

        assert_eq!(payload.models.len(), 1);
        assert_eq!(payload.models[0].model, "qwen3.8-max-preview");
        assert_eq!(payload.models[0].request_count, 2003);
        assert_eq!(payload.daily.len(), 1);
        assert_eq!(payload.daily[0].date, 1785081600);
        assert_eq!(payload.daily[0].total_tokens, 5634371);
    }

    #[test]
    fn parses_usage_response_without_models_or_daily() {
        // 接口契约变更仅返回 summary 时，models / daily 应为空而非报错
        let payload: super::UsageResponse =
            serde_json::from_str(r#"{ "summary": { "total_tokens": 1, "quota": 1.0 } }"#)
                .expect("应兼容缺失明细字段的响应");

        assert!(payload.models.is_empty());
        assert!(payload.daily.is_empty());
    }

    #[test]
    fn assemble_dashboard_computes_quota_and_usage() {
        let user = super::UserInfo {
            id: 42,
            quota: 75_000_000,
            used_quota: 25_000_000,
            request_count: 1234,
            display_name: "Tester".into(),
        };
        let usage = super::TodayUsage {
            summary: super::UsageSummary {
                quota: 2.0 * QUOTA_PER_YUAN,
                total_tokens: 3000,
                input_tokens: 1000,
                output_tokens: 2000,
            },
            log_count: Some(57),
            logs_available: true,
        };

        let data = super::assemble_dashboard(user, usage);

        assert_eq!(data.display_name, "Tester");
        assert_eq!(data.request_count, 1234);
        assert_eq!(data.log_count, Some(57));
        assert!(data.logs_available);
        // 75M 剩余 / (75M+25M) 总量 = 75%
        assert!((data.percent - 75.0).abs() < 1e-9);
        assert!((data.remaining - 150.0).abs() < 1e-9);
        assert!((data.used - 50.0).abs() < 1e-9);
        assert!((data.total - 200.0).abs() < 1e-9);
        assert!((data.today_yuan - 2.0).abs() < 1e-9);
        assert_eq!(data.today_tokens, 3000);
        assert_eq!(data.today_input, 1000);
        assert_eq!(data.today_output, 2000);
    }

    #[test]
    fn assemble_dashboard_handles_zero_quota() {
        let user = super::UserInfo {
            id: 1,
            quota: 0,
            used_quota: 0,
            request_count: 0,
            display_name: "Empty".into(),
        };
        let data = super::assemble_dashboard(user, super::TodayUsage::default());
        assert_eq!(data.percent, 0.0);
        assert!((data.remaining - 0.0).abs() < 1e-9);
        assert!(!data.logs_available);
        assert_eq!(data.log_count, None);
    }
}
