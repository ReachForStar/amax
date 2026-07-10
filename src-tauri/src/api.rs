//! HTTP API 模块 — 调用 AMAX 后端接口

use chrono::{Datelike, Local, TimeZone};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
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
    quota: f64,
    total_tokens: i64,
    input_tokens: i64,
    output_tokens: i64,
}

#[derive(Deserialize)]
struct UsageResponse {
    summary: UsageSummary,
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

pub async fn fetch_dashboard(
    client: &reqwest::Client,
    cookie: &str,
) -> Result<DashboardData, String> {
    if cookie.is_empty() {
        return Err("认证失败: 未配置 Cookie".into());
    }

    let now = Local::now();
    let start_time = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .ok_or_else(|| "无法计算本地日期起点".to_string())?
        .timestamp();
    let end_time = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 23, 59, 59)
        .single()
        .ok_or_else(|| "无法计算本地日期终点".to_string())?
        .timestamp();
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

    let mut summary = UsageSummary::default();
    let mut logs_available = false;
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
                        logs_available = true;
                    }
                    Err(error) => {
                        let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
                        eprintln!(
                            "日志汇总 JSON 解析失败，保留账户额度数据: status={status}, content-type={content_type}, body={preview:?}, error={error}"
                        );
                    }
                },
                Err(error) => eprintln!(
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
            eprintln!(
                "日志汇总查询返回异常状态，保留账户额度数据: status={status}, body={preview:?}"
            );
        }
        Err(error) => eprintln!("日志汇总查询失败，保留账户额度数据: {error}"),
    }

    Ok(DashboardData {
        display_name: user.display_name,
        request_count: user.request_count,
        today_yuan: summary.quota / QUOTA_PER_YUAN,
        today_tokens: summary.total_tokens,
        today_input: summary.input_tokens,
        today_output: summary.output_tokens,
        log_count: None,
        remaining,
        used,
        total,
        percent,
        logs_available,
    })
}

#[cfg(test)]
mod tests {
    use super::{QUOTA_PER_YUAN, UsageResponse};

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
}
