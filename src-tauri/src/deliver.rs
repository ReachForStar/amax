//! 告警投递渠道 — 系统通知 / MeoW 推送 / 邮件（SMTP）
//!
//! 本模块只负责「把一条消息发出去，并回报成功 / 永久失败 / 可重试」。当日额度与去重
//! 一律由 `lib.rs::run_alerts` 记账，所以增减渠道不会动到判定逻辑，理由同 `alert.rs`
//! 把规则做成纯函数：能单独测的那部分才守得住。
//!
//! 凭据口径：只有 SMTP 授权码算凭据（`db.rs` 里以 `dpapi:v1:` 加密存取，视图只给「是否已配」）；
//! MeoW 昵称按明文配置对待，与 SMTP 主机/账号同级，界面原样回显。本模块拿到的永远是明文，因此
//! - 授权码相关的错误串必须先过 `redact` 再回前端或落日志；
//! - `Debug` 手写而不是 derive，避免一次 `{:?}` 就把授权码打进日志。

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 推送端点按昵称路由、无鉴权：`POST /{nickname}/{title}`
pub const MEOW_BASE_URL: &str = "https://api.chuckfang.com";
/// 单规则单日的失败投递行上限（开了几个渠道就一次计几行）。可重试的失败不消耗当日额度，
/// 所以必须有上限，否则限流或断网会让每 10 分钟一轮的刷新重投一整天（144 次骚扰 + 144 行写库）。
pub const MAX_FAILED_DELIVERIES_PER_RULE: i64 = 3;
/// 邮件走同步 SMTP：卡住的是投递任务而不是看板刷新，但仍要有界
const MAIL_TIMEOUT: Duration = Duration::from_secs(20);

/// 一条消息在某个渠道上的结果。
/// 区分「永久失败」与「可重试」是为了让当日额度只在成功或确定无望时消耗。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    Failed,
    Retryable,
}

impl Outcome {
    pub fn is_success(self) -> bool {
        self == Outcome::Sent
    }
}

#[derive(Debug, Clone)]
pub struct Receipt {
    pub channel: Channel,
    pub outcome: Outcome,
    /// 已脱敏的失败原因，成功时为 `None`
    pub error: Option<String>,
}

impl Receipt {
    pub fn sent(channel: Channel) -> Self {
        Self {
            channel,
            outcome: Outcome::Sent,
            error: None,
        }
    }

    pub fn failed(channel: Channel, error: impl Into<String>) -> Self {
        Self {
            channel,
            outcome: Outcome::Failed,
            error: Some(error.into()),
        }
    }

    pub fn retryable(channel: Channel, error: impl Into<String>) -> Self {
        Self {
            channel,
            outcome: Outcome::Retryable,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Notification,
    Meow,
    Mail,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::Notification, Channel::Meow, Channel::Mail];

    /// 落库用的稳定标识（`alert_delivery.channel`），改名等于丢历史，勿动
    pub fn key(self) -> &'static str {
        match self {
            Channel::Notification => "notification",
            Channel::Meow => "meow",
            Channel::Mail => "mail",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Notification => "系统通知",
            Channel::Meow => "MeoW 推送",
            Channel::Mail => "邮件",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|channel| channel.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// 465：连接即 TLS（lettre 的 `relay`）
    #[default]
    Implicit,
    /// 587：明文连接后 STARTTLS 升级（lettre 的 `starttls_relay`）
    StartTls,
}

impl TlsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TlsMode::Implicit => "implicit",
            TlsMode::StartTls => "starttls",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "implicit" | "tls" | "smtps" => Some(TlsMode::Implicit),
            "starttls" => Some(TlsMode::StartTls),
            _ => None,
        }
    }
}

/// 齐备可用的邮件投递目标（由 `DeliveryConfig::mail_target` 派生，不单独存库）
#[derive(Clone, PartialEq, Eq)]
pub struct MailTarget {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub user: String,
    pub auth_code: String,
    pub to: String,
}

/// 手写 Debug：derive 会把授权码一起打出来，日志里一次 `{:?}` 就是泄密
impl std::fmt::Debug for MailTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("user", &self.user)
            .field("to", &self.to)
            .field("auth_code", &"[redacted]")
            .finish()
    }
}

/// 已存的 SMTP 明文字段。单独存着而不是只存组装后的 `MailTarget`：配到一半（比如还没填
/// 收件地址）时，界面仍要回显用户已经填过的主机与端口，否则一次提交就把它们清空了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpStored {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub user: String,
    pub to: String,
}

impl Default for SmtpStored {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 465,
            tls: TlsMode::Implicit,
            user: String::new(),
            to: String::new(),
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct DeliveryConfig {
    pub notification: bool,
    pub meow: bool,
    /// MeoW 昵称按普通明文配置存（同 SMTP 主机/账号）：它只是收件路由名，本机随时可读可改，
    /// 界面上原样回显。`Option` 只用来区分「没填」和「填了空串以外的值」。
    pub meow_nickname: Option<String>,
    pub mail: bool,
    pub smtp: SmtpStored,
    /// 已解密可用的授权码；解不开时为 None 且键名进 `undecryptable`
    pub mail_auth_code: Option<String>,
    /// 存过但本机解不开（换机器或换 Windows 用户后的旧密文），配置键名
    pub undecryptable: Vec<String>,
    /// 配了但读不出来的脏字段，只用于展示与日志
    pub problems: Vec<String>,
}

/// 同上：derive 会把授权码原样打进日志
impl std::fmt::Debug for DeliveryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryConfig")
            .field("notification", &self.notification)
            .field("meow", &self.meow)
            .field("meow_nickname", &self.meow_nickname)
            .field("mail", &self.mail)
            .field("smtp", &self.smtp)
            .field(
                "mail_auth_code",
                &self.mail_auth_code.as_ref().map(|_| "[redacted]"),
            )
            .field("undecryptable", &self.undecryptable)
            .field("problems", &self.problems)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// 开关关着
    Off,
    /// 开关开着但缺凭据、或凭据解不开
    Incomplete,
    Ready,
}

impl DeliveryConfig {
    pub fn availability(&self, channel: Channel) -> Availability {
        let enabled = match channel {
            Channel::Notification => self.notification,
            Channel::Meow => self.meow,
            Channel::Mail => self.mail,
        };
        if !enabled {
            return Availability::Off;
        }
        match channel {
            // 系统通知不需要凭据，开了就能用
            Channel::Notification => Availability::Ready,
            Channel::Meow => match &self.meow_nickname {
                Some(_) => Availability::Ready,
                None => Availability::Incomplete,
            },
            Channel::Mail => match self.mail_target() {
                Some(_) => Availability::Ready,
                None => Availability::Incomplete,
            },
        }
    }

    /// 明文字段齐全且授权码可用才算配好
    pub fn mail_target(&self) -> Option<MailTarget> {
        let auth_code = self.mail_auth_code.as_ref()?;
        let smtp = &self.smtp;
        if smtp.host.is_empty() || smtp.user.is_empty() || smtp.to.is_empty() || smtp.port == 0 {
            return None;
        }
        Some(MailTarget {
            host: smtp.host.clone(),
            port: smtp.port,
            tls: smtp.tls,
            user: smtp.user.clone(),
            auth_code: auth_code.clone(),
            to: smtp.to.clone(),
        })
    }

    pub fn usable(&self) -> Vec<Channel> {
        Channel::ALL
            .into_iter()
            .filter(|channel| self.availability(*channel) == Availability::Ready)
            .collect()
    }

    /// 渠道开着却用不了时给用户的解释；返回 `None` 表示无需提示
    pub fn notice(&self, channel: Channel) -> Option<String> {
        if self.availability(channel) != Availability::Incomplete {
            return None;
        }
        match channel {
            Channel::Meow => Some("尚未填写 MeoW 昵称".to_string()),
            // 授权码「解不开」与「从没填过」是两件事：前者要重填，后者是漏填
            Channel::Mail => Some(
                if self
                    .undecryptable
                    .iter()
                    .any(|stored| stored == "smtp_auth_code")
                {
                    "凭据无法解密（换过机器或 Windows 用户），请重新填写".to_string()
                } else {
                    "邮件配置不完整（主机、账号、授权码、收件地址）".to_string()
                },
            ),
            // 系统通知不需要凭据，开不了就是 Off
            Channel::Notification => None,
        }
    }
}

/// 投递账本的一行（复盘界面用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryLog {
    pub rule: String,
    pub channel: String,
    pub ok: bool,
    pub error: Option<String>,
    pub at: String,
}

/// 给设置页的视图：只有真正的凭据（SMTP 授权码）不给原值，界面拿不到它也就无法原样提交回来。
pub fn view(config: &DeliveryConfig) -> serde_json::Value {
    serde_json::json!({
        "notification": {
            "enabled": config.notification,
            "available": availability_key(config.availability(Channel::Notification)),
        },
        "meow": {
            "enabled": config.meow,
            "available": availability_key(config.availability(Channel::Meow)),
            "nickname": config.meow_nickname,
            "notice": config.notice(Channel::Meow),
        },
        "mail": {
            "enabled": config.mail,
            "available": availability_key(config.availability(Channel::Mail)),
            "host": config.smtp.host,
            "port": config.smtp.port,
            "tls": config.smtp.tls.as_str(),
            "user": config.smtp.user,
            "to": config.smtp.to,
            "authCodeConfigured": config.mail_auth_code.is_some()
                || config.undecryptable.iter().any(|key| key == "smtp_auth_code"),
            "notice": config.notice(Channel::Mail),
        },
        "problems": config.problems,
    })
}

fn availability_key(availability: Availability) -> &'static str {
    match availability {
        Availability::Off => "off",
        Availability::Incomplete => "incomplete",
        Availability::Ready => "ready",
    }
}

/// 前端提交的渠道设置。
///
/// 开关、MeoW 昵称与 SMTP 明文字段每次整体覆盖（界面原样回显这些值，提交回来的就是当前内容）；
/// 只有 SMTP 授权码是例外——它是唯一真凭据，视图不给原值，所以 `None` 表示保持原值、
/// `Some("")` 才删除。若把界面上的占位文本当成授权码提交，真凭据就被覆盖掉了。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelInput {
    pub notification: bool,
    pub meow: bool,
    pub mail: bool,
    #[serde(default)]
    pub meow_nickname: String,
    #[serde(default)]
    pub smtp: Option<SmtpInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SmtpInput {
    pub host: String,
    pub port: u16,
    pub tls: String,
    pub user: String,
    #[serde(default)]
    pub auth_code: Option<String>,
    pub to: String,
}

impl ChannelInput {
    /// 校验入参。`existing` 提供本次**不提交**的凭据原值（授权码），用来判断「开启邮件」是否真有料可发。
    pub fn validate(&self, existing: &DeliveryConfig) -> Result<(), AppError> {
        if self.meow {
            let nickname = self.meow_nickname.trim();
            if nickname.is_empty() {
                return Err(AppError::input("开启 MeoW 推送需要先填写昵称"));
            }
            if nickname.chars().count() > 100 {
                return Err(AppError::input("MeoW 昵称过长"));
            }
            if nickname.chars().any(char::is_control) {
                return Err(AppError::input("MeoW 昵称含控制字符"));
            }
        }

        // 关着也校验格式：存一份填错的配置等于把问题留到下次开启
        if let Some(smtp) = &self.smtp {
            validate_smtp(smtp)?;
            if self.mail && !auth_code_available(smtp, existing) {
                return Err(AppError::input("开启邮件通知需要先填写授权码"));
            }
        } else if self.mail {
            return Err(AppError::input("开启邮件通知需要提交 SMTP 配置"));
        }
        Ok(())
    }
}

fn validate_smtp(smtp: &SmtpInput) -> Result<(), AppError> {
    let host = smtp.host.trim();
    if host.is_empty() {
        return Err(AppError::input("SMTP 主机不能为空"));
    }
    if host.chars().any(char::is_whitespace) {
        return Err(AppError::input("SMTP 主机不能含空格"));
    }
    if smtp.port == 0 {
        return Err(AppError::input("SMTP 端口非法"));
    }
    if TlsMode::parse(&smtp.tls).is_none() {
        return Err(AppError::input(
            "SMTP 加密方式只能是 implicit(465) 或 starttls(587)",
        ));
    }
    validate_address(&smtp.user, "发件账号")?;
    validate_address(&smtp.to, "收件地址")
}

/// 换主机就要重填授权码：QQ / 163 的授权码按服务商签发，旧主机的码对新主机没有意义
fn auth_code_available(smtp: &SmtpInput, existing: &DeliveryConfig) -> bool {
    if smtp
        .auth_code
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }
    existing.mail_auth_code.is_some() && existing.smtp.host.trim() == smtp.host.trim()
}

fn validate_address(value: &str, label: &str) -> Result<(), AppError> {
    value
        .trim()
        .parse::<lettre::Address>()
        .map(|_| ())
        .map_err(|_| AppError::input(format!("{label}不是合法邮箱地址")))
}

/// 把可能含凭据的服务端文案换成 `***`。落库与写日志前都要过这里。
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut redacted = text.to_string();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        redacted = redacted.replace(secret, "***");
    }
    redacted
}

/// 渠道整体结论：任一成功即成功；否则只要有一个还能重试就判可重试（交给下一轮，
/// 由 `MAX_FAILED_DELIVERIES_PER_RULE` 收口）；全都不行才算永久失败。
pub fn summarize(receipts: &[Receipt]) -> Outcome {
    if receipts.iter().any(|receipt| receipt.outcome.is_success()) {
        Outcome::Sent
    } else if receipts
        .iter()
        .any(|receipt| receipt.outcome == Outcome::Retryable)
    {
        Outcome::Retryable
    } else {
        Outcome::Failed
    }
}

/// 系统通知。失败几乎都是「未授权 / 通知服务不可用」，重试无意义，所以记永久失败。
pub fn send_notification(app: &tauri::AppHandle, title: &str, body: &str) -> Receipt {
    use tauri_plugin_notification::NotificationExt;
    let channel = Channel::Notification;
    match app.notification().builder().title(title).body(body).show() {
        Ok(()) => Receipt::sent(channel),
        Err(error) => Receipt::failed(channel, format!("系统通知发送失败: {error}")),
    }
}

/// MeoW 端点：昵称与标题都百分号编码（中文昵称、含 `/` 的昵称都会改写路径段）
pub fn meow_endpoint(nickname: &str, title: &str) -> Result<url::Url, AppError> {
    let mut url = url::Url::parse(MEOW_BASE_URL)
        .map_err(|error| AppError::storage(format!("MeoW 基址配置异常: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| AppError::storage("MeoW 基址无法拼接路径"))?
        .push(nickname)
        .push(title);
    url.query_pairs_mut().append_pair("msgType", "markdown");
    Ok(url)
}

/// 响应分类：`status == 200` 或 `data == true` 算成功；429 与 5xx 可重试；其余永久失败
pub fn classify_meow(status: u16, body: &str) -> Outcome {
    if status == 429 || status >= 500 {
        return Outcome::Retryable;
    }
    if !(200..300).contains(&status) {
        return Outcome::Failed;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        // 2xx 但不是 JSON：不猜业务失败，按 HTTP 状态算成功，原文仍能落库复盘
        return Outcome::Sent;
    };
    let accepted = value.get("status").and_then(serde_json::Value::as_i64) == Some(200)
        || value.get("data").and_then(serde_json::Value::as_bool) == Some(true);
    if accepted {
        Outcome::Sent
    } else {
        Outcome::Failed
    }
}

pub async fn send_meow(
    client: &reqwest::Client,
    nickname: &str,
    title: &str,
    body: &str,
) -> Receipt {
    let channel = Channel::Meow;
    let endpoint = match meow_endpoint(nickname, title) {
        Ok(endpoint) => endpoint,
        Err(error) => return Receipt::failed(channel, error.message),
    };
    let payload = serde_json::json!({ "msg": body, "title": title });
    let response = client.post(endpoint.as_str()).json(&payload).send().await;
    match response {
        Err(error) => Receipt::retryable(channel, format!("MeoW 请求失败: {error}")),
        Ok(response) => {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            let outcome = classify_meow(status, &text);
            if outcome.is_success() {
                return Receipt::sent(channel);
            }
            let brief: String = text.chars().take(200).collect();
            let detail = format!("MeoW 响应 {status}: {brief}");
            match outcome {
                Outcome::Retryable => Receipt::retryable(channel, detail),
                _ => Receipt::failed(channel, detail),
            }
        }
    }
}

/// 同步发邮件，调用方必须放进 `spawn_blocking`（lettre 的阻塞传输会占住线程）。
pub fn send_mail(target: &MailTarget, title: &str, body: &str) -> Receipt {
    use lettre::Transport;
    match build_mail(target, title, body) {
        Err(receipt) => receipt,
        Ok((message, transport)) => match transport.send(&message) {
            Ok(_) => Receipt::sent(Channel::Mail),
            Err(error) => {
                let outcome = if error.is_transient() || error.is_timeout() {
                    Outcome::Retryable
                } else {
                    Outcome::Failed
                };
                let detail = redact(
                    &format!("邮件发送失败: {error}"),
                    &[target.auth_code.as_str()],
                );
                match outcome {
                    Outcome::Retryable => Receipt::retryable(Channel::Mail, detail),
                    _ => Receipt::failed(Channel::Mail, detail),
                }
            }
        },
    }
}

/// 组装待发邮件与传输器。拆开是为了能离线单测（不连服务端也要验证地址与主题正确）
fn build_mail(
    target: &MailTarget,
    title: &str,
    body: &str,
) -> Result<(lettre::Message, lettre::SmtpTransport), Receipt> {
    use lettre::SmtpTransport;
    use lettre::message::{Mailbox, MessageBuilder};
    use lettre::transport::smtp::authentication::Credentials;

    let from = Mailbox::try_from(("AMAX Dashboard", target.user.as_str()))
        .map_err(|error| setup_failure(target, &error.to_string()))?;
    let to = Mailbox::try_from((target.to.as_str(), target.to.as_str()))
        .map_err(|error| setup_failure(target, &error.to_string()))?;
    let message = MessageBuilder::new()
        .from(from)
        .to(to)
        .subject(title)
        .body(body.to_string())
        .map_err(|error| setup_failure(target, &error.to_string()))?;

    let builder = match target.tls {
        TlsMode::Implicit => SmtpTransport::relay(&target.host),
        TlsMode::StartTls => SmtpTransport::starttls_relay(&target.host),
    }
    .map_err(|error| setup_failure(target, &error.to_string()))?;
    let transport = builder
        .port(target.port)
        .credentials(Credentials::new(
            target.user.clone(),
            target.auth_code.clone(),
        ))
        .timeout(Some(MAIL_TIMEOUT))
        .build();
    Ok((message, transport))
}

fn setup_failure(target: &MailTarget, text: &str) -> Receipt {
    Receipt::failed(
        Channel::Mail,
        redact(
            &format!("邮件配置不可用: {text}"),
            &[target.auth_code.as_str()],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        Availability, Channel, ChannelInput, DeliveryConfig, MailTarget, Outcome, SmtpInput,
        SmtpStored, TlsMode, build_mail, classify_meow, meow_endpoint, redact, summarize, view,
    };
    use crate::deliver::Receipt;

    fn target(host: &str, auth_code: &str) -> MailTarget {
        MailTarget {
            host: host.to_string(),
            port: 465,
            tls: TlsMode::Implicit,
            user: "bot@qq.com".to_string(),
            auth_code: auth_code.to_string(),
            to: "me@qq.com".to_string(),
        }
    }

    fn config_with_mail(auth_code: Option<&str>, host: &str) -> DeliveryConfig {
        DeliveryConfig {
            mail: true,
            smtp: SmtpStored {
                host: host.to_string(),
                port: 465,
                tls: TlsMode::Implicit,
                user: "bot@qq.com".to_string(),
                to: "me@qq.com".to_string(),
            },
            mail_auth_code: auth_code.map(str::to_string),
            ..DeliveryConfig::default()
        }
    }

    #[test]
    fn channel_keys_are_stable_identifiers() {
        assert_eq!(
            Channel::ALL.map(|channel| channel.key()),
            ["notification", "meow", "mail"]
        );
        for channel in Channel::ALL {
            assert_eq!(Channel::from_key(channel.key()), Some(channel));
        }
        assert_eq!(Channel::from_key("sms"), None);
    }

    #[test]
    fn tls_mode_round_trips() {
        for (text, mode) in [
            ("implicit", TlsMode::Implicit),
            ("SMTPS", TlsMode::Implicit),
            (" starttls ", TlsMode::StartTls),
        ] {
            assert_eq!(TlsMode::parse(text), Some(mode), "{text}");
            assert_eq!(TlsMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(TlsMode::parse("none"), None);
        assert_eq!(TlsMode::default(), TlsMode::Implicit);
    }

    #[test]
    fn notification_needs_no_credential_but_network_channels_do() {
        let config = DeliveryConfig {
            notification: true,
            meow: true,
            mail: true,
            ..DeliveryConfig::default()
        };
        assert_eq!(
            config.availability(Channel::Notification),
            Availability::Ready
        );
        assert_eq!(config.availability(Channel::Meow), Availability::Incomplete);
        assert_eq!(config.availability(Channel::Mail), Availability::Incomplete);
        assert_eq!(config.usable(), vec![Channel::Notification]);
        assert_eq!(
            config.notice(Channel::Meow).as_deref(),
            Some("尚未填写 MeoW 昵称")
        );
    }

    #[test]
    fn disabled_channel_is_off_regardless_of_credential() {
        let config = DeliveryConfig {
            meow: false,
            meow_nickname: Some("xyx".into()),
            ..DeliveryConfig::default()
        };
        assert_eq!(config.availability(Channel::Meow), Availability::Off);
        assert_eq!(config.notice(Channel::Meow), None);
        assert!(config.usable().is_empty());
    }

    #[test]
    fn incomplete_mail_lists_which_piece_is_missing() {
        // 有授权码但没填收件地址：仍不可用
        let mut config = config_with_mail(Some("secret"), "smtp.qq.com");
        config.smtp.to.clear();
        assert_eq!(config.availability(Channel::Mail), Availability::Incomplete);
        assert!(
            config
                .notice(Channel::Mail)
                .unwrap_or_default()
                .contains("不完整")
        );
        assert!(config.mail_target().is_none());
    }

    #[test]
    fn undecryptable_auth_code_says_reenter_not_missing() {
        let config = DeliveryConfig {
            mail: true,
            smtp: SmtpStored {
                host: "smtp.qq.com".into(),
                port: 465,
                tls: TlsMode::Implicit,
                user: "bot@qq.com".into(),
                to: "me@qq.com".into(),
            },
            undecryptable: vec!["smtp_auth_code".into()],
            ..DeliveryConfig::default()
        };
        assert_eq!(config.availability(Channel::Mail), Availability::Incomplete);
        assert!(
            config
                .notice(Channel::Mail)
                .unwrap_or_default()
                .contains("重新填写")
        );
    }

    #[test]
    fn summarize_prefers_success_then_retryable() {
        let receipt = |channel, outcome| Receipt {
            channel,
            outcome,
            error: None,
        };
        assert_eq!(
            summarize(&[
                receipt(Channel::Mail, Outcome::Failed),
                receipt(Channel::Meow, Outcome::Sent)
            ]),
            Outcome::Sent
        );
        assert_eq!(
            summarize(&[
                receipt(Channel::Mail, Outcome::Failed),
                receipt(Channel::Meow, Outcome::Retryable)
            ]),
            Outcome::Retryable
        );
        assert_eq!(
            summarize(&[receipt(Channel::Mail, Outcome::Failed)]),
            Outcome::Failed
        );
        assert_eq!(summarize(&[]), Outcome::Failed);
    }

    #[test]
    fn meow_endpoint_percent_encodes_both_segments() {
        let url = meow_endpoint("张三/pika", "⚠️ AMAX 额度不足").expect("应可构造");
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("api.chuckfang.com"));
        assert!(!url.path().contains(' '), "路径不应含裸空格: {}", url);
        assert_eq!(url.query(), Some("msgType=markdown"));
        // `/` 必须被编码，否则请求会打到别的昵称上
        assert_eq!(url.path_segments().unwrap().count(), 2);
        assert!(url.path().contains("%2F"));
    }

    #[test]
    fn meow_response_classification() {
        assert_eq!(
            classify_meow(200, r#"{"status":200,"msg":"ok"}"#),
            Outcome::Sent
        );
        assert_eq!(classify_meow(200, r#"{"data":true}"#), Outcome::Sent);
        assert_eq!(
            classify_meow(200, r#"{"status":500,"msg":"no such user"}"#),
            Outcome::Failed
        );
        assert_eq!(classify_meow(429, "slow down"), Outcome::Retryable);
        assert_eq!(classify_meow(502, "bad gateway"), Outcome::Retryable);
        assert_eq!(classify_meow(404, "not found"), Outcome::Failed);
        assert_eq!(classify_meow(200, "not json"), Outcome::Sent);
    }

    #[test]
    fn auth_code_survives_neither_debug_nor_view() {
        let mut config = config_with_mail(Some("abcdefghijkl"), "smtp.qq.com");
        config.meow = true;
        config.meow_nickname = Some("pikachu".into());
        let debug = format!("{config:?}");
        assert!(!debug.contains("abcdefghijkl"), "Debug 泄漏授权码");
        // 昵称不是凭据，Debug 里原样出现才有排障价值
        assert!(debug.contains("pikachu"), "昵称按普通配置打印");

        let shown = view(&config);
        let text = shown.to_string();
        assert!(!text.contains("abcdefghijkl"));
        assert_eq!(shown["meow"]["nickname"], "pikachu");
        assert_eq!(shown["mail"]["user"], "bot@qq.com");
        assert_eq!(shown["mail"]["authCodeConfigured"], true);
        assert_eq!(shown["mail"]["port"], 465);
        assert_eq!(shown["mail"]["available"], "ready");
    }

    #[test]
    fn view_keeps_plain_fields_of_a_half_filled_config() {
        let config = config_with_mail(None, "smtp.163.com");
        let shown = view(&config);
        assert_eq!(shown["mail"]["available"], "incomplete");
        assert_eq!(shown["mail"]["host"], "smtp.163.com");
        assert_eq!(shown["mail"]["authCodeConfigured"], false);
    }

    #[test]
    fn redact_replaces_every_credential_occurrence() {
        let text = "auth failed for abcdefghijkl, retry with abdefghijkl";
        assert_eq!(
            redact(text, &["abcdefghijkl", "abdefghijkl"]),
            "auth failed for ***, retry with ***"
        );
        // 空凭据不能参与替换，否则会把整段文案吃掉
        assert_eq!(redact("keep me", &[""]), "keep me");
    }

    fn input(meow: bool, mail: bool) -> ChannelInput {
        ChannelInput {
            notification: true,
            meow,
            mail,
            meow_nickname: "pika".into(),
            smtp: Some(SmtpInput {
                host: "smtp.qq.com".into(),
                port: 465,
                tls: "implicit".into(),
                user: "bot@qq.com".into(),
                auth_code: Some("secret".into()),
                to: "me@qq.com".into(),
            }),
        }
    }

    #[test]
    fn enabling_a_channel_without_its_credential_is_rejected() {
        let mut bare = input(true, false);
        bare.meow_nickname = String::new();
        let error = bare
            .validate(&DeliveryConfig::default())
            .expect_err("缺昵称应拒绝");
        assert!(error.message.contains("昵称"), "{error}");

        // 昵称每次整体覆盖，所以「本机存过」不能替代本次提交：界面会原样回显它，
        // 提交空串就是用户自己清空的
        let stored = DeliveryConfig {
            meow_nickname: Some("stored".into()),
            ..DeliveryConfig::default()
        };
        let error = bare
            .validate(&stored)
            .expect_err("开启 MeoW 必须随本次提交带上昵称");
        assert!(error.message.contains("昵称"), "{error}");
    }

    #[test]
    fn mail_format_is_validated_even_when_mail_is_off() {
        let mut bad = input(false, false);
        bad.smtp.as_mut().expect("应有 smtp").host = "smtp qq".into();
        let error = bad
            .validate(&DeliveryConfig::default())
            .expect_err("主机含空格应拒绝");
        assert!(error.message.contains("空格"), "{error}");

        for broken in ["port", "tls", "user"] {
            let mut variant = input(false, false);
            let smtp = variant.smtp.as_mut().expect("应有 smtp");
            match broken {
                "port" => smtp.port = 0,
                "tls" => smtp.tls = "plaintext".into(),
                _ => smtp.user = "not-an-address".into(),
            }
            let error = variant
                .validate(&DeliveryConfig::default())
                .expect_err("{broken} 非法应拒绝");
            assert_eq!(error.code, crate::error::ErrorCode::Input, "{broken}");
        }
    }

    #[test]
    fn stored_auth_code_satisfies_validation_only_for_the_same_host() {
        let existing = config_with_mail(Some("secret"), "smtp.qq.com");
        let mut variant = input(false, true);
        variant.smtp.as_mut().expect("应有 smtp").auth_code = None;
        variant
            .validate(&existing)
            .expect("同主机已有授权码，不必重新提交");

        let other = config_with_mail(Some("secret"), "smtp.163.com");
        let error = variant
            .validate(&other)
            .expect_err("换主机后旧授权码不算数");
        assert!(error.message.contains("授权码"), "{error}");

        assert!(
            variant.validate(&DeliveryConfig::default()).is_err(),
            "全新的邮件配置必须带授权码"
        );
    }

    #[test]
    fn clearing_the_nickname_is_allowed_while_the_channel_is_off() {
        let mut variant = input(false, false);
        variant.meow_nickname = String::new();
        variant
            .validate(&DeliveryConfig::default())
            .expect("清空昵称但不是要开启渠道");
    }

    #[test]
    fn mail_message_carries_configured_addresses_and_subject() {
        let (message, _transport) = build_mail(
            &target("smtp.qq.com", "secret"),
            "⚠️ 额度不足",
            "剩余 ¥1.00",
        )
        .expect("应可组装");
        let formatted = message.formatted();
        let source = String::from_utf8_lossy(&formatted);
        assert!(source.contains("bot@qq.com"), "发件地址应在文里: {source}");
        assert!(source.contains("me@qq.com"), "收件地址应在文里: {source}");
        assert!(source.contains("Subject:"), "主题头应存在: {source}");
        assert!(
            !source.contains("secret"),
            "授权码不该出现在邮件正文里: {source}"
        );
    }

    #[test]
    fn mail_target_is_derived_only_from_a_complete_config() {
        let config = config_with_mail(Some("secret"), "smtp.qq.com");
        let derived = config.mail_target().expect("齐全应可用");
        assert_eq!(derived.port, 465);
        assert_eq!(derived.auth_code, "secret");
        assert_eq!(config.mail_target(), Some(derived));
    }
}
