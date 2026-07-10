//! HTTP API 模块 — 调用 AMAX 后端接口

use chrono::Local;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const BASE_URL: &str = "https://ai.amaxsmp.com";
const MAX_LOG_PAGES: i64 = 100;
pub const QUOTA_PER_YUAN: f64 = 500_000.0;

#[derive(Deserialize)]
struct ApiEnvelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct UserInfo {
    quota: i64,
    used_quota: i64,
    request_count: i64,
    display_name: String,
}

#[derive(Deserialize)]
struct LogEntry {
    quota: f64,
    total_tokens: i64,
    input_tokens: i64,
    output_tokens: i64,
}

#[derive(Deserialize)]
struct LogsResponse {
    total_pages: i64,
    data: Vec<LogEntry>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LogsPayload {
    Direct(LogsResponse),
    Enveloped(ApiEnvelope<LogsResponse>),
}

impl LogsPayload {
    fn into_response(self) -> LogsResponse {
        match self {
            Self::Direct(response) => response,
            Self::Enveloped(envelope) => envelope.data,
        }
    }
}

#[derive(Default)]
struct LogSummary {
    quota: f64,
    total_tokens: i64,
    input_tokens: i64,
    output_tokens: i64,
    count: usize,
}

impl LogSummary {
    fn add(&mut self, entry: LogEntry) {
        self.quota += entry.quota;
        self.total_tokens += entry.total_tokens;
        self.input_tokens += entry.input_tokens;
        self.output_tokens += entry.output_tokens;
        self.count += 1;
    }
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
    pub log_count: usize,
    pub remaining: f64,
    pub used: f64,
    pub total: f64,
    pub percent: f64,
    pub logs_available: bool,
}

pub fn build_client() -> Result<reqwest::Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0"));
    headers.insert("x-company", HeaderValue::from_static("AMAX"));

    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| format!("HTTP 客户端初始化失败: {error}"))
}

pub async fn fetch_dashboard(
    client: &reqwest::Client,
    cookie: &str,
    api_key: Option<&str>,
) -> Result<DashboardData, String> {
    if cookie.is_empty() {
        return Err("认证失败: 未配置 Cookie".into());
    }

    let today = Local::now().format("%Y-%m-%d").to_string();
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
    let user = envelope.data;
    let total_quota = user.quota.saturating_add(user.used_quota);
    let remaining = user.quota as f64 / QUOTA_PER_YUAN;
    let used = user.used_quota as f64 / QUOTA_PER_YUAN;
    let total = total_quota as f64 / QUOTA_PER_YUAN;
    let percent = if total_quota > 0 {
        (user.quota as f64 / total_quota as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let mut summary = LogSummary::default();
    let mut logs_available = true;
    let mut page = 1;
    loop {
        let url = format!(
            "{BASE_URL}/v1/logs?page={page}&page_size=1000&start_time={today}&end_time={today}"
        );
        let body = match api_key {
            Some(key) => serde_json::json!({ "api_keys": [key] }),
            None => serde_json::json!({}),
        };
        let response = match client
            .post(url)
            .header("Cookie", cookie)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                eprintln!("日志查询失败，保留账户额度数据: {error}");
                logs_available = false;
                break;
            }
        };

        if !response.status().is_success() {
            eprintln!(
                "日志查询返回异常状态，保留账户额度数据: {}",
                response.status()
            );
            logs_available = false;
            break;
        }

        let logs_response = match response.json::<LogsPayload>().await {
            Ok(payload) => payload.into_response(),
            Err(error) => {
                eprintln!("日志响应解析失败，保留账户额度数据: {error}");
                logs_available = false;
                break;
            }
        };
        if !(0..=MAX_LOG_PAGES).contains(&logs_response.total_pages) {
            eprintln!(
                "日志分页数据异常，保留账户额度数据: {}",
                logs_response.total_pages
            );
            logs_available = false;
            break;
        }
        for entry in logs_response.data {
            summary.add(entry);
        }
        if page >= logs_response.total_pages || logs_response.total_pages == 0 {
            break;
        }
        page += 1;
    }
    Ok(DashboardData {
        display_name: user.display_name,
        request_count: user.request_count,
        today_yuan: summary.quota / QUOTA_PER_YUAN,
        today_tokens: summary.total_tokens,
        today_input: summary.input_tokens,
        today_output: summary.output_tokens,
        log_count: summary.count,
        remaining,
        used,
        total,
        percent,
        logs_available,
    })
}

#[cfg(test)]
mod tests {
    use super::{LogsPayload, LogsResponse};

    fn assert_empty_response(response: LogsResponse) {
        assert_eq!(response.total_pages, 0);
        assert!(response.data.is_empty());
    }

    #[test]
    fn parses_direct_logs_response() {
        let payload: LogsPayload =
            serde_json::from_str(r#"{"total_pages":0,"data":[]}"#).expect("应解析直接日志响应");
        assert_empty_response(payload.into_response());
    }

    #[test]
    fn parses_enveloped_logs_response() {
        let payload: LogsPayload = serde_json::from_str(r#"{"data":{"total_pages":0,"data":[]}}"#)
            .expect("应解析带信封的日志响应");
        assert_empty_response(payload.into_response());
    }
}
