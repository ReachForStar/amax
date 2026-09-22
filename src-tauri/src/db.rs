//! SQLite 持久化模块 — cookie / api_key 加密存储 + 看板快照 + 告警状态与投递记录

use crate::crypto;
use crate::error::AppError;
use chrono::{Local, NaiveDate};
use rusqlite::{Connection, params};

const ENCRYPTED_PREFIX: &str = "dpapi:v1:";
/// 服务端下发的 Cookie 到期时间（本地时区 RFC3339）。非机密，明文存；
/// 仅用于展示，不参与任何失效判定（失效由服务端返回 401/403 决定）
const KEY_COOKIE_EXPIRES_AT: &str = "cookie_expires_at";

/// 告警参数（明文字段）：非机密，且要能直接查库排障——沿用 `cookie_expires_at` 的判例
const KEY_ALERT_ENABLED: &str = "alert_enabled";
const KEY_ALERT_QUOTA_PERCENT: &str = "alert_quota_percent";
const KEY_ALERT_RUNOUT_DAYS: &str = "alert_runout_days";
const KEY_ALERT_SPIKE_MULTIPLIER: &str = "alert_spike_multiplier";
const KEY_ALERT_MAX_FIRES_PER_DAY: &str = "alert_max_fires_per_day";
/// 逐规则开关的键前缀，完整键形如 `alert_rule_quota_low`
const KEY_ALERT_RULE_PREFIX: &str = "alert_rule_";

/// 投递渠道开关（明文）
const KEY_ALERT_CH_NOTIFICATION: &str = "alert_ch_notification";
const KEY_ALERT_CH_MEOW: &str = "alert_ch_meow";
const KEY_ALERT_CH_MAIL: &str = "alert_ch_mail";
/// MeoW 昵称即凭据——知道昵称就能往对方手机推消息，与 SMTP 授权码一样必须加密；
/// 主机/端口/账号/收件地址是明文（同 `smtp_user` 只对本机可见的口径）
const KEY_MEOW_NICKNAME: &str = "meow_nickname";
const KEY_SMTP_HOST: &str = "smtp_host";
const KEY_SMTP_PORT: &str = "smtp_port";
const KEY_SMTP_TLS: &str = "smtp_tls";
const KEY_SMTP_USER: &str = "smtp_user";
const KEY_SMTP_AUTH_CODE: &str = "smtp_auth_code";
const KEY_SMTP_TO: &str = "smtp_to";

/// 读取加密配置的结果。必须区分「没配过」与「配过但本机解不开」（换机器或换 Windows 用户），
/// 后者要让用户看到需要重填，不能静默当成没配。
pub enum Secret {
    Absent,
    Value(String),
    Undecryptable,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &std::path::Path) -> Result<Self, AppError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS config (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dashboard_snapshot (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                saved_at      TEXT NOT NULL,
                day           TEXT,
                today_yuan    REAL NOT NULL,
                total_tokens  INTEGER NOT NULL,
                remaining     REAL NOT NULL,
                used          REAL NOT NULL,
                total         REAL NOT NULL,
                request_count INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_dashboard_snapshot_saved_at
                ON dashboard_snapshot(saved_at);
            CREATE TABLE IF NOT EXISTS alert_state (
                rule_key      TEXT NOT NULL,
                day           TEXT NOT NULL,
                count         INTEGER NOT NULL,
                last_fired_at TEXT NOT NULL,
                PRIMARY KEY (rule_key, day)
            );
            CREATE TABLE IF NOT EXISTS alert_delivery (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                rule_key    TEXT NOT NULL,
                day         TEXT NOT NULL,
                channel     TEXT NOT NULL,
                ok          INTEGER NOT NULL,
                error       TEXT,
                delivered_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_alert_delivery_day
                ON alert_delivery(day);",
        )?;
        Self::migrate_schema(&conn)?;
        Ok(Self { conn })
    }

    /// 旧库迁移：补齐 request_count / day 列并回填；幂等，新旧表均可安全执行。
    /// 说明：day 为写入时的本地日期（YYYY-MM-DD），由 save_snapshot 计算。
    /// 不用 date(saved_at,'localtime') 表达式索引——SQLite 视 localtime 修饰符为非确定性函数，
    /// 禁止出现在索引中；存储列 + 普通索引才能让统计查询按日走索引范围扫描。
    pub fn migrate_schema(conn: &Connection) -> Result<(), AppError> {
        if !Self::has_column(conn, "dashboard_snapshot", "request_count")? {
            conn.execute(
                "ALTER TABLE dashboard_snapshot ADD COLUMN request_count INTEGER",
                [],
            )?;
        }
        if !Self::has_column(conn, "dashboard_snapshot", "day")? {
            conn.execute("ALTER TABLE dashboard_snapshot ADD COLUMN day TEXT", [])?;
        }
        // 一次性回填历史行（按旧口径的本地日期换算）；已回填后为无操作
        conn.execute(
            "UPDATE dashboard_snapshot SET day = date(saved_at, 'localtime') WHERE day IS NULL",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_snapshot_day ON dashboard_snapshot(day)",
            [],
        )?;
        Ok(())
    }

    fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, rusqlite::Error> {
        let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut names = statement.query_map([], |row| row.get::<_, String>(1))?;
        Ok(names.any(|name| name.as_deref() == Ok(column)))
    }

    fn get_raw(&self, key: &str) -> Result<Option<String>, rusqlite::Error> {
        let mut statement = self.conn.prepare("SELECT value FROM config WHERE key=?1")?;
        let mut rows = statement.query(params![key])?;
        rows.next()?.map(|row| row.get(0)).transpose()
    }

    fn encrypt_secret(value: &str) -> Result<String, AppError> {
        crypto::encrypt(value.as_bytes())
            .map(|encrypted| format!("{ENCRYPTED_PREFIX}{encrypted}"))
            .map_err(|error| AppError::storage(format!("凭据加密失败: {error}")))
    }

    fn decrypt_secret(raw: String) -> Option<String> {
        let Some(encrypted) = raw.strip_prefix(ENCRYPTED_PREFIX) else {
            return Some(raw);
        };
        crypto::decrypt(encrypted)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    /// `expires_at` 与新 Cookie 同生命周期：换发凭据时若没有到期信息必须清掉旧值，
    /// 否则上一次登录下发的日期会被当成当前凭据的有效期
    pub fn save_config(
        &mut self,
        cookie: &str,
        api_key: &str,
        expires_at: Option<&str>,
    ) -> Result<(), AppError> {
        let encrypted_cookie = (!cookie.is_empty())
            .then(|| Self::encrypt_secret(cookie))
            .transpose()?;
        let encrypted_api_key = (!api_key.is_empty())
            .then(|| Self::encrypt_secret(api_key))
            .transpose()?;

        if encrypted_cookie.is_none() && encrypted_api_key.is_none() {
            return Ok(());
        }

        let transaction = self.conn.transaction()?;
        if let Some(value) = encrypted_cookie {
            transaction.execute(
                "INSERT INTO config(key, value) VALUES('cookie', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![value],
            )?;
            match expires_at {
                Some(value) => transaction.execute(
                    "INSERT INTO config(key, value) VALUES(?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![KEY_COOKIE_EXPIRES_AT, value],
                )?,
                None => transaction.execute(
                    "DELETE FROM config WHERE key=?1",
                    params![KEY_COOKIE_EXPIRES_AT],
                )?,
            };
        }
        if let Some(value) = encrypted_api_key {
            transaction.execute(
                "INSERT INTO config(key, value) VALUES('api_key', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![value],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_cookie(&self) -> Option<String> {
        Self::decrypt_secret(self.get_raw("cookie").ok()??)
    }

    pub fn has_cookie(&self) -> bool {
        self.get_cookie().is_some_and(|cookie| !cookie.is_empty())
    }

    pub fn get_cookie_expires_at(&self) -> Option<String> {
        self.get_raw(KEY_COOKIE_EXPIRES_AT)
            .ok()
            .flatten()
            .filter(|value| !value.is_empty())
    }

    pub fn get_api_key(&self) -> Option<String> {
        Self::decrypt_secret(self.get_raw("api_key").ok()??)
    }

    pub fn has_api_key(&self) -> bool {
        self.get_api_key().is_some_and(|key| !key.is_empty())
    }

    pub fn save_snapshot(
        &self,
        today_yuan: f64,
        total_tokens: i64,
        remaining: f64,
        used: f64,
        total: f64,
        request_count: i64,
    ) -> Result<(), AppError> {
        // saved_at 以本地时区 RFC3339 写入，查询侧统一按存储的 day 列（写入时的本地日期）取每日末条
        // （旧口径用 date(saved_at, 'localtime') 在查询时换算，依赖查询时刻的时区；day 列固定写入时刻归属）
        // 快照永久保留，供统计页按日聚合，不再做定期清理
        let now = Local::now();
        self.conn.execute(
            "INSERT INTO dashboard_snapshot
                 (saved_at, day, today_yuan, total_tokens, remaining, used, total, request_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now.to_rfc3339(),
                now.format("%Y-%m-%d").to_string(),
                today_yuan,
                total_tokens,
                remaining,
                used,
                total,
                request_count
            ],
        )?;
        Ok(())
    }

    /// 按日取区间内每天的末条快照
    pub fn get_daily_snapshots(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<DailySnapshot>, AppError> {
        let mut statement = self.conn.prepare(
            "SELECT day, today_yuan, total_tokens, remaining, request_count
             FROM dashboard_snapshot
             WHERE id IN (SELECT MAX(id) FROM dashboard_snapshot
                          WHERE day BETWEEN ?1 AND ?2
                          GROUP BY day)
             ORDER BY day",
        )?;
        let rows = statement.query_map(params![start_date, end_date], |row| {
            Ok(DailySnapshot {
                date: row.get(0)?,
                yuan: row.get(1)?,
                tokens: row.get(2)?,
                remaining: row.get(3)?,
                request_count: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn get_config_text(&self, key: &str) -> Result<Option<String>, AppError> {
        let mut statement = self.conn.prepare("SELECT value FROM config WHERE key=?1")?;
        let value = statement
            .query_row(params![key], |row| row.get::<_, String>(0))
            .ok();
        Ok(value)
    }

    fn set_config_text(&self, key: &str, value: &str) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT INTO config(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    fn get_config_number<T>(&self, key: &str, default: T) -> Result<T, AppError>
    where
        T: std::str::FromStr,
    {
        let raw = self.get_config_text(key)?;
        Ok(match raw.as_deref().map(str::trim) {
            Some(text) => text
                .parse::<T>()
                .map_err(|_| AppError::storage(format!("告警设置 {key} 不是合法数值: {text}")))?,
            None => default,
        })
    }

    /// 读取四个可调参数 + 总开关；缺失取默认，脏值报错，调用方决定如何降级（见 `alert::load_settings`）
    pub fn get_alert_settings(&self) -> Result<crate::alert::AlertSettings, AppError> {
        let defaults = crate::alert::AlertSettings::default();
        Ok(crate::alert::AlertSettings {
            enabled: match self.get_config_text(KEY_ALERT_ENABLED)?.as_deref() {
                Some(value) => value == "1",
                None => defaults.enabled,
            },
            quota_percent: self
                .get_config_number(KEY_ALERT_QUOTA_PERCENT, defaults.quota_percent)?,
            runout_days: self.get_config_number(KEY_ALERT_RUNOUT_DAYS, defaults.runout_days)?,
            spike_multiplier: self
                .get_config_number(KEY_ALERT_SPIKE_MULTIPLIER, defaults.spike_multiplier)?,
            max_fires_per_day: self
                .get_config_number(KEY_ALERT_MAX_FIRES_PER_DAY, defaults.max_fires_per_day)?,
        })
    }

    /// 写入前一律 `sanitized()`：宁可把越界值夹进合法区间，也不让非法参数进到判定里
    pub fn set_alert_settings(
        &self,
        settings: crate::alert::AlertSettings,
    ) -> Result<crate::alert::AlertSettings, AppError> {
        let settings = settings.sanitized();
        self.set_config_text(KEY_ALERT_ENABLED, if settings.enabled { "1" } else { "0" })?;
        self.set_config_text(KEY_ALERT_QUOTA_PERCENT, &settings.quota_percent.to_string())?;
        self.set_config_text(KEY_ALERT_RUNOUT_DAYS, &settings.runout_days.to_string())?;
        self.set_config_text(
            KEY_ALERT_SPIKE_MULTIPLIER,
            &settings.spike_multiplier.to_string(),
        )?;
        self.set_config_text(
            KEY_ALERT_MAX_FIRES_PER_DAY,
            &settings.max_fires_per_day.to_string(),
        )?;
        Ok(settings)
    }

    /// 启用的规则 key 列表，顺序按传入的 `known_rules`（即规则声明顺序）
    pub fn get_alert_rules_enabled(&self, known_rules: &[&str]) -> Result<Vec<String>, AppError> {
        let mut enabled = Vec::new();
        for rule in known_rules {
            let key = format!("{KEY_ALERT_RULE_PREFIX}{rule}");
            match self.get_config_text(&key)? {
                // 未落库即默认开启，四参数之外不再额外要求用户先动过设置
                None => enabled.push((*rule).to_string()),
                Some(value) => {
                    if value.trim() == "1" {
                        enabled.push((*rule).to_string());
                    }
                }
            }
        }
        Ok(enabled)
    }

    pub fn set_alert_rules_enabled(
        &self,
        rules: &[String],
        known_rules: &[&str],
    ) -> Result<(), AppError> {
        for rule in known_rules {
            let key = format!("{KEY_ALERT_RULE_PREFIX}{rule}");
            let on = rules.iter().any(|enabled| enabled == *rule);
            self.set_config_text(&key, if on { "1" } else { "0" })?;
        }
        Ok(())
    }

    /// 当日各规则的已投次数（未知规则的旧行由调用方忽略）
    pub fn get_alert_day_counts(&self, day: &str) -> Result<Vec<(String, i64)>, AppError> {
        let mut statement = self
            .conn
            .prepare("SELECT rule_key, count FROM alert_state WHERE day=?1 ORDER BY rule_key")?;
        let rows = statement.query_map(params![day], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 记一次投递：同 (规则, 日) 累加次数并刷新时间，因此同日重复调用就是去重依据
    pub fn record_alert_fire(
        &self,
        rule_key: &str,
        day: &str,
        fired_at: &str,
    ) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT INTO alert_state(rule_key, day, count, last_fired_at) VALUES(?1, ?2, 1, ?3)
             ON CONFLICT(rule_key, day)
             DO UPDATE SET count=count+1, last_fired_at=excluded.last_fired_at",
            params![rule_key, day, fired_at],
        )?;
        Ok(())
    }

    /// 渠道投递结果。只增不改（append-only）；`error` 由调用方保证不含凭据明文，
    /// 且该表是本地表、不在任何同步通道里。
    pub fn record_alert_delivery(
        &self,
        rule_key: &str,
        day: &str,
        channel: &str,
        ok: bool,
        error: Option<&str>,
        delivered_at: &str,
    ) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT INTO alert_delivery(rule_key, day, channel, ok, error, delivered_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![rule_key, day, channel, ok as i64, error, delivered_at],
        )?;
        Ok(())
    }

    /// 当日某规则已失败几次，用于给「可重试的投递失败」封顶，避免每 10 分钟撞同一堵墙
    pub fn count_alert_failures(&self, rule_key: &str, day: &str) -> Result<i64, AppError> {
        let mut statement = self
            .conn
            .prepare("SELECT COUNT(*) FROM alert_delivery WHERE rule_key=?1 AND day=?2 AND ok=0")?;
        let count = statement.query_row(params![rule_key, day], |row| row.get::<_, i64>(0))?;
        Ok(count)
    }

    /// 该表唯一的读取入口：只给「投递复盘」与失败封顶判定用，按时间倒序取最近几条
    pub fn recent_alert_deliveries(
        &self,
        day: &str,
        limit: i64,
    ) -> Result<Vec<crate::deliver::DeliveryLog>, AppError> {
        let mut statement = self.conn.prepare(
            "SELECT rule_key, channel, ok, error, delivered_at
             FROM alert_delivery WHERE day=?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![day, limit], |row| {
            Ok(crate::deliver::DeliveryLog {
                rule: row.get(0)?,
                channel: row.get(1)?,
                ok: row.get::<_, i64>(2)? != 0,
                error: row.get(3)?,
                at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn get_secret(&self, key: &str) -> Result<Secret, AppError> {
        let Some(raw) = self.get_config_text(key)? else {
            return Ok(Secret::Absent);
        };
        if raw.is_empty() {
            return Ok(Secret::Absent);
        }
        if !raw.starts_with(ENCRYPTED_PREFIX) {
            // 旧明文记录（同 cookie/api_key 的兼容口径）
            return Ok(Secret::Value(raw));
        }
        Ok(match Self::decrypt_secret(raw.clone()) {
            Some(value) if value.is_empty() => Secret::Absent,
            Some(value) => Secret::Value(value),
            None => Secret::Undecryptable,
        })
    }

    fn set_secret(&self, key: &str, value: &str) -> Result<(), AppError> {
        self.set_config_text(key, &Self::encrypt_secret(value)?)
    }

    fn clear_config(&self, key: &str) -> Result<(), AppError> {
        self.conn
            .execute("DELETE FROM config WHERE key=?1", params![key])?;
        Ok(())
    }

    fn flag(&self, key: &str, default: bool) -> Result<bool, AppError> {
        Ok(match self.get_config_text(key)?.as_deref() {
            Some(value) => value.trim() == "1",
            None => default,
        })
    }

    /// 投递渠道配置。单个字段脏（例如手改库把端口写成非数字）只记进 `problems` 并退回默认，
    /// 不让整份读取失败——额度告警不该被一封配坏的邮件拖掉。
    pub fn get_delivery_config(&self) -> Result<crate::deliver::DeliveryConfig, AppError> {
        use crate::deliver::{DeliveryConfig, TlsMode};
        let mut config = DeliveryConfig {
            // 系统通知默认开：老用户一直收得到，不该因为没写过这个键而变成关
            notification: self.flag(KEY_ALERT_CH_NOTIFICATION, true)?,
            meow: self.flag(KEY_ALERT_CH_MEOW, false)?,
            mail: self.flag(KEY_ALERT_CH_MAIL, false)?,
            ..DeliveryConfig::default()
        };

        // 昵称是明文配置，但旧库里可能还留着它当凭据时写下的 `dpapi:v1:` 密文：`get_secret`
        // 两种都读得出来，下次保存自然落成明文。解不开就按没填处理——它不是凭据，不值一条提示。
        if let Secret::Value(value) = self.get_secret(KEY_MEOW_NICKNAME)? {
            config.meow_nickname = Some(value);
        }
        match self.get_secret(KEY_SMTP_AUTH_CODE)? {
            Secret::Value(value) => config.mail_auth_code = Some(value),
            Secret::Undecryptable => config.undecryptable.push("smtp_auth_code".into()),
            Secret::Absent => {}
        }

        let text = |key: &str| -> Result<String, AppError> {
            Ok(self.get_config_text(key)?.unwrap_or_default())
        };
        config.smtp.host = text(KEY_SMTP_HOST)?;
        config.smtp.user = text(KEY_SMTP_USER)?;
        config.smtp.to = text(KEY_SMTP_TO)?;
        if let Some(value) = self.get_config_text(KEY_SMTP_TLS)? {
            match TlsMode::parse(&value) {
                Some(mode) => config.smtp.tls = mode,
                None => config
                    .problems
                    .push(format!("SMTP 加密方式无法识别: {value}")),
            }
        }
        if let Some(value) = self.get_config_text(KEY_SMTP_PORT)? {
            match value.trim().parse::<u16>() {
                Ok(0) => config.problems.push(format!("SMTP 端口非法: {value}")),
                Ok(port) => config.smtp.port = port,
                Err(_) => config.problems.push(format!("SMTP 端口无法识别: {value}")),
            }
        }
        Ok(config)
    }

    /// 写渠道设置。规则：开关、MeoW 昵称与 SMTP 明文字段整体覆盖；只有授权码是凭据，
    /// 仅在 `Some` 时改动（`None`=保持原值、`Some("")`=删除），因为视图不回显它的原值。
    pub fn set_delivery_channels(
        &self,
        input: &crate::deliver::ChannelInput,
    ) -> Result<(), AppError> {
        self.set_config_text(
            KEY_ALERT_CH_NOTIFICATION,
            if input.notification { "1" } else { "0" },
        )?;
        self.set_config_text(KEY_ALERT_CH_MEOW, if input.meow { "1" } else { "0" })?;
        self.set_config_text(KEY_ALERT_CH_MAIL, if input.mail { "1" } else { "0" })?;

        let nickname = input.meow_nickname.trim();
        if nickname.is_empty() {
            self.clear_config(KEY_MEOW_NICKNAME)?;
        } else {
            self.set_config_text(KEY_MEOW_NICKNAME, nickname)?;
        }

        let Some(smtp) = &input.smtp else {
            return Ok(());
        };
        self.set_config_text(KEY_SMTP_HOST, smtp.host.trim())?;
        self.set_config_text(KEY_SMTP_PORT, &smtp.port.to_string())?;
        self.set_config_text(KEY_SMTP_TLS, smtp.tls.as_str())?;
        self.set_config_text(KEY_SMTP_USER, smtp.user.trim())?;
        self.set_config_text(KEY_SMTP_TO, smtp.to.trim())?;
        if let Some(auth_code) = &smtp.auth_code {
            let auth_code = auth_code.trim();
            if auth_code.is_empty() {
                self.clear_config(KEY_SMTP_AUTH_CODE)?;
            } else {
                self.set_secret(KEY_SMTP_AUTH_CODE, auth_code)?;
            }
        }
        Ok(())
    }
}

/// 每日快照行 — 取当日最后一条
#[derive(Debug, Clone)]
pub struct DailySnapshot {
    pub date: String,
    pub yuan: f64,
    pub tokens: i64,
    pub remaining: f64,
    pub request_count: Option<i64>,
}

/// 每日新增请求数：仅相邻两日且双方累计值非 NULL 时产出，其余留空
pub fn derive_daily_requests(snapshots: &[DailySnapshot]) -> Vec<Option<i64>> {
    snapshots
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let prev = snapshots.get(index.checked_sub(1)?)?;
            let current = row.request_count?;
            let previous = prev.request_count?;
            let day = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d").ok()?;
            let prev_day = NaiveDate::parse_from_str(&prev.date, "%Y-%m-%d").ok()?;
            (day - prev_day == chrono::Duration::days(1)).then_some(current - previous)
        })
        .collect()
}

/// 本地汇总派生：(累计费用, 日均费用, 峰值费用, 峰值日期, 累计 Token)
pub fn summarize_local(snapshots: &[DailySnapshot]) -> (f64, f64, f64, String, i64) {
    let total_yuan: f64 = snapshots.iter().map(|row| row.yuan).sum();
    let total_tokens: i64 = snapshots.iter().map(|row| row.tokens).sum();
    let days = snapshots.len();
    let avg_yuan = if days > 0 {
        total_yuan / days as f64
    } else {
        0.0
    };
    let mut peak_yuan = 0.0_f64;
    let mut peak_date = String::new();
    for row in snapshots {
        if row.yuan > peak_yuan {
            peak_yuan = row.yuan;
            peak_date = row.date.clone();
        }
    }
    (total_yuan, avg_yuan, peak_yuan, peak_date, total_tokens)
}

#[cfg(test)]
mod tests {
    use super::{
        DailySnapshot, Db, ENCRYPTED_PREFIX, KEY_MEOW_NICKNAME, KEY_SMTP_AUTH_CODE, KEY_SMTP_PORT,
        KEY_SMTP_TLS, derive_daily_requests,
    };

    fn snapshot(date: &str, request_count: Option<i64>) -> DailySnapshot {
        DailySnapshot {
            date: date.to_string(),
            yuan: 0.0,
            tokens: 0,
            remaining: 0.0,
            request_count,
        }
    }

    #[test]
    fn daily_requests_diff_adjacent_days() {
        let rows = vec![
            snapshot("2026-08-01", Some(100)),
            snapshot("2026-08-02", Some(130)),
            snapshot("2026-08-03", Some(200)),
        ];
        assert_eq!(derive_daily_requests(&rows), vec![None, Some(30), Some(70)]);
    }

    #[test]
    fn daily_requests_gap_and_null_yield_none() {
        let rows = vec![
            snapshot("2026-08-01", Some(100)),
            snapshot("2026-08-03", Some(300)), // 缺 8-02，不可差分
            snapshot("2026-08-04", None),      // 老数据 NULL
            snapshot("2026-08-05", Some(400)), // 前行为 NULL，不可差分
        ];
        assert_eq!(derive_daily_requests(&rows), vec![None, None, None, None]);
    }

    #[test]
    fn daily_requests_empty_input() {
        assert!(derive_daily_requests(&[]).is_empty());
    }

    fn local_rfc3339(date_time: &str) -> String {
        // 给无偏移的本地时间串追加当前机器时区偏移，模拟 save_snapshot 的写入形态
        format!("{date_time}{}", chrono::Local::now().format("%:z"))
    }

    fn db_with_snapshots() -> Db {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        // (saved_at, today_yuan, total_tokens, remaining, request_count)
        let rows = [
            (
                local_rfc3339("2026-08-01T08:00:00"),
                5.0,
                100,
                90.0,
                Some(10),
            ),
            (
                local_rfc3339("2026-08-01T20:00:00"),
                12.0,
                300,
                83.0,
                Some(25),
            ), // 8-01 末条
            (local_rfc3339("2026-08-02T10:00:00"), 3.0, 80, 80.0, None), // 老数据模拟 NULL
        ];
        for (saved_at, yuan, tokens, remaining, req) in rows {
            db.conn
                .execute(
                    "INSERT INTO dashboard_snapshot
                         (saved_at, day, today_yuan, total_tokens, remaining, used, total, request_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, 0, 100, ?6)",
                    rusqlite::params![
                        saved_at,
                        &saved_at[..10], // 本地日期即 saved_at 的日期部分
                        yuan,
                        tokens,
                        remaining,
                        req
                    ],
                )
                .expect("插入快照应成功");
        }
        db
    }

    #[test]
    fn daily_snapshots_take_last_per_day_and_filter_range() {
        let db = db_with_snapshots();
        let rows = db
            .get_daily_snapshots("2026-08-01", "2026-08-02")
            .expect("查询应成功");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].date, "2026-08-01");
        assert_eq!(rows[0].yuan, 12.0); // 日末条，非早 8 点那条
        assert_eq!(rows[0].tokens, 300);
        assert_eq!(rows[0].remaining, 83.0);
        assert_eq!(rows[0].request_count, Some(25));
        assert_eq!(rows[1].date, "2026-08-02");
        assert_eq!(rows[1].request_count, None);

        // 边界：仅取区间内日期（含端点）
        let only_first = db
            .get_daily_snapshots("2026-08-01", "2026-08-01")
            .expect("查询应成功");
        assert_eq!(only_first.len(), 1);
        assert_eq!(only_first[0].date, "2026-08-01");
    }

    #[test]
    fn daily_snapshots_attribute_early_morning_to_local_date() {
        // 防回归：凌晨快照按写入时的本地日期（day 列）归属当日，不因 UTC 归一偏移到前一天
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.conn
            .execute(
                "INSERT INTO dashboard_snapshot
                     (saved_at, day, today_yuan, total_tokens, remaining, used, total, request_count)
                 VALUES (?1, '2026-08-03', 1.0, 10, 99.0, 0, 100, 1)",
                rusqlite::params![local_rfc3339("2026-08-03T06:00:00")],
            )
            .expect("插入快照应成功");

        let rows = db
            .get_daily_snapshots("2026-08-03", "2026-08-03")
            .expect("查询应成功");
        assert_eq!(rows.len(), 1, "凌晨 6 点快照应归属本地当日");
        assert_eq!(rows[0].date, "2026-08-03");
    }

    #[test]
    fn daily_snapshots_empty_table_returns_empty() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let rows = db
            .get_daily_snapshots("2026-08-01", "2026-08-02")
            .expect("查询应成功");
        assert!(rows.is_empty());
    }

    #[test]
    fn daily_snapshots_range_filtered_inside_subquery_and_indexed() {
        // 防回归：区间过滤须下推到子查询内（依赖 day 列索引），
        // 区间外的日期不得参与分组，也不得出现在结果中
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let insert = |saved_at: &str, req: i64| {
            db.conn
                .execute(
                    "INSERT INTO dashboard_snapshot
                         (saved_at, day, today_yuan, total_tokens, remaining, used, total, request_count)
                     VALUES (?1, ?2, 1.0, 10, 99.0, 0, 100, ?3)",
                    rusqlite::params![local_rfc3339(saved_at), &saved_at[..10], req],
                )
                .expect("插入快照应成功");
        };
        // 区间外（前后各一天）+ 区间内两天各两条，验证日末条取 MAX(id)
        insert("2026-07-31T10:00:00", 1);
        insert("2026-08-01T08:00:00", 10);
        insert("2026-08-01T20:00:00", 25);
        insert("2026-08-02T09:00:00", 30);
        insert("2026-08-02T21:00:00", 55);
        insert("2026-08-03T10:00:00", 99);

        let rows = db
            .get_daily_snapshots("2026-08-01", "2026-08-02")
            .expect("查询应成功");
        assert_eq!(rows.len(), 2, "区间外日期不应出现在结果中");
        assert_eq!(rows[0].date, "2026-08-01");
        assert_eq!(rows[0].request_count, Some(25), "应取当日末条");
        assert_eq!(rows[1].date, "2026-08-02");
        assert_eq!(rows[1].request_count, Some(55), "应取当日末条");

        // day 列索引应已建好，查询可走索引范围扫描
        let mut statement = db
            .conn
            .prepare("SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_snapshot_day'")
            .expect("准备应成功");
        let exists = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .expect("查询应成功")
            .next()
            .is_some();
        assert!(exists, "day 列索引应存在");
    }

    #[test]
    fn decrypt_secret_reads_legacy_plaintext() {
        let value = "session=legacy-cookie".to_string();
        assert_eq!(Db::decrypt_secret(value.clone()), Some(value));
    }

    #[test]
    fn encrypt_secret_round_trips() {
        let value = "session=encrypted-cookie";
        let encrypted = Db::encrypt_secret(value).expect("加密应成功");
        assert_eq!(Db::decrypt_secret(encrypted).as_deref(), Some(value));
    }

    #[test]
    fn decrypt_secret_rejects_invalid_encrypted_value() {
        assert_eq!(Db::decrypt_secret("dpapi:v1:invalid".to_string()), None);
    }

    fn db_with_config(cookie: &str, api_key: &str, expires: Option<&str>) -> Db {
        let mut db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.save_config(cookie, api_key, expires)
            .expect("保存配置应成功");
        db
    }

    #[test]
    fn save_config_stores_expires_alongside_cookie() {
        let db = db_with_config("session=abcdef", "", Some("2026-10-01T08:00:00+08:00"));
        assert_eq!(
            db.get_cookie_expires_at().as_deref(),
            Some("2026-10-01T08:00:00+08:00")
        );
    }

    #[test]
    fn save_config_clears_stale_expires_when_new_cookie_has_none() {
        // 防回归：换发凭据但没有到期信息时必须清掉旧值，不能把上次的日期当成当前有效期
        let mut db = db_with_config("session=first", "", Some("2026-10-01T08:00:00+08:00"));
        db.save_config("session=second", "", None)
            .expect("更新凭据应成功");
        assert_eq!(db.get_cookie_expires_at(), None);
        assert_eq!(db.get_cookie().as_deref(), Some("session=second"));
    }

    #[test]
    fn save_config_without_cookie_keeps_existing_expires() {
        // 只更新 API Key 时凭据未变，到期时间须原样保留
        let mut db = db_with_config("session=first", "", Some("2026-10-01T08:00:00+08:00"));
        db.save_config("", "sk-test", None)
            .expect("仅保存 API Key 应成功");
        assert_eq!(
            db.get_cookie_expires_at().as_deref(),
            Some("2026-10-01T08:00:00+08:00")
        );
    }

    #[test]
    fn get_cookie_expires_at_none_when_never_saved() {
        let db = db_with_config("session=abcdef", "", None);
        assert_eq!(db.get_cookie_expires_at(), None);
    }

    #[test]
    fn migrate_schema_is_idempotent_and_upgrades_legacy() {
        let conn = rusqlite::Connection::open_in_memory().expect("内存数据库应打开成功");
        // 构造旧 schema（无 request_count / day 列），并插入一条历史行
        // （用机器当前偏移构造 saved_at，保证任意时区测试机回填的本地日期一致）
        let legacy_saved_at = local_rfc3339("2026-08-01T08:00:00");
        conn.execute_batch(&format!(
            "CREATE TABLE dashboard_snapshot (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                saved_at TEXT NOT NULL,
                today_yuan REAL NOT NULL,
                total_tokens INTEGER NOT NULL,
                remaining REAL NOT NULL,
                used REAL NOT NULL,
                total REAL NOT NULL
            );
            INSERT INTO dashboard_snapshot
                (saved_at, today_yuan, total_tokens, remaining, used, total)
            VALUES ('{legacy_saved_at}', 1.0, 10, 99.0, 0, 100);"
        ))
        .expect("建旧表应成功");

        super::Db::migrate_schema(&conn).expect("首次迁移应成功");
        super::Db::migrate_schema(&conn).expect("重复迁移应幂等成功");

        let mut statement = conn
            .prepare("PRAGMA table_info(dashboard_snapshot)")
            .expect("准备应成功");
        let names: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("查询应成功")
            .collect::<Result<_, _>>()
            .expect("收集应成功");
        assert!(names.iter().any(|name| name == "request_count"));
        assert!(names.iter().any(|name| name == "day"));

        // 历史行应按本地日期回填 day 列
        let day: String = conn
            .query_row("SELECT day FROM dashboard_snapshot LIMIT 1", [], |row| {
                row.get(0)
            })
            .expect("回填后的 day 应可读");
        assert_eq!(day, "2026-08-01");

        // day 索引应已建好
        let index_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_snapshot_day'",
                [],
                |row| row.get(0),
            )
            .expect("索引查询应成功");
        assert_eq!(index_exists, 1);
    }

    // ---------- 告警状态与参数 ----------

    fn alert_day_counts(db: &Db, day: &str) -> Vec<(String, i64)> {
        db.get_alert_day_counts(day)
            .expect("读取当日告警次数应成功")
    }

    #[test]
    fn alert_fire_counts_once_per_rule_and_day() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.record_alert_fire("quota_low", "2026-09-22", "2026-09-22T10:00:00+08:00")
            .expect("首次记录应成功");
        db.record_alert_fire("quota_low", "2026-09-22", "2026-09-22T11:00:00+08:00")
            .expect("同日重复记录应成功累加");
        db.record_alert_fire("spike", "2026-09-22", "2026-09-22T11:05:00+08:00")
            .expect("另一规则应可同日记录");
        db.record_alert_fire("quota_low", "2026-09-23", "2026-09-23T09:00:00+08:00")
            .expect("跨日应另起一行");

        assert_eq!(
            alert_day_counts(&db, "2026-09-22"),
            vec![("quota_low".to_string(), 2), ("spike".to_string(), 1)]
        );
        assert_eq!(
            alert_day_counts(&db, "2026-09-23"),
            vec![("quota_low".to_string(), 1)]
        );
        assert!(
            alert_day_counts(&db, "2026-09-21").is_empty(),
            "未投递的过去日应为空"
        );

        // 去重依据是 count>0，last_fired_at 只留最新一次
        let last: String = db
            .conn
            .query_row(
                "SELECT last_fired_at FROM alert_state WHERE rule_key='quota_low' AND day='2026-09-22'",
                [],
                |row| row.get(0),
            )
            .expect("读取末次时间应成功");
        assert_eq!(last, "2026-09-22T11:00:00+08:00");
    }

    #[test]
    fn alert_settings_default_when_absent() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        assert_eq!(
            db.get_alert_settings().expect("缺省应取默认值"),
            crate::alert::AlertSettings::default()
        );
    }

    #[test]
    fn alert_settings_round_trip_and_sanitize_on_write() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let saved = db
            .set_alert_settings(crate::alert::AlertSettings {
                enabled: false,
                quota_percent: 500.0, // 越界，写入时夹到 99
                runout_days: 3,
                spike_multiplier: 2.5,
                max_fires_per_day: 0,
            })
            .expect("保存应成功");
        assert_eq!(saved.quota_percent, 99.0, "保存前应已夹取");
        assert_eq!(
            db.get_alert_settings().expect("读取应成功"),
            crate::alert::AlertSettings {
                enabled: false,
                quota_percent: 99.0,
                runout_days: 3,
                spike_multiplier: 2.5,
                max_fires_per_day: 0,
            }
        );
    }

    #[test]
    fn alert_settings_typed_as_number_are_still_readable() {
        // 手工 `UPDATE config SET value=15 WHERE key='alert_quota_percent'` 存成 SQLite INTEGER 后，
        // rusqlite 走 f64 转换的路径必须仍然可读（数字参数一律以 REAL 写入，不能只测 TEXT）
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.set_alert_settings(crate::alert::AlertSettings {
            quota_percent: 20.0,
            ..crate::alert::AlertSettings::default()
        })
        .expect("预置应成功");
        db.conn
            .execute(
                "UPDATE config SET value=CAST(15 AS INTEGER) WHERE key='alert_quota_percent'",
                [],
            )
            .expect("改存为数值类型应成功");
        assert_eq!(
            db.get_alert_settings()
                .expect("数值存储应可读")
                .quota_percent,
            15.0
        );
    }

    #[test]
    fn dirty_alert_settings_report_error_instead_of_silently_defaulting() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.conn
            .execute(
                "INSERT INTO config(key, value) VALUES('alert_runout_days', 'abc')",
                [],
            )
            .expect("塞入脏值应成功");
        let error = db
            .get_alert_settings()
            .expect_err("脏值应报错，由调用方决定降级");
        assert_eq!(error.code, crate::error::ErrorCode::Storage);
        assert!(error.message.contains("alert_runout_days"));
    }

    #[test]
    fn alert_rules_default_to_enabled_and_unknown_keys_switch_nothing() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let known = crate::alert::rule_keys();
        assert_eq!(
            db.get_alert_rules_enabled(&known).expect("未落库应可读"),
            known.iter().map(|key| key.to_string()).collect::<Vec<_>>()
        );

        db.set_alert_rules_enabled(&["spike".to_string()], &known)
            .expect("写入子集应成功");
        assert_eq!(
            db.get_alert_rules_enabled(&known).expect("读取应成功"),
            vec!["spike".to_string()]
        );

        db.set_alert_rules_enabled(&["nope".to_string()], &known)
            .expect("未知 key 应静默忽略");
        assert!(
            db.get_alert_rules_enabled(&known)
                .expect("读取应成功")
                .is_empty(),
            "未知 key 不该把任何规则打开"
        );
    }

    #[test]
    fn alert_delivery_rows_are_append_only() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.record_alert_delivery(
            "quota_low",
            "2026-09-22",
            "notification",
            true,
            None,
            "2026-09-22T10:00:00+08:00",
        )
        .expect("成功记录应写入");
        db.record_alert_delivery(
            "quota_low",
            "2026-09-22",
            "mail",
            false,
            Some("550 relay not permitted"),
            "2026-09-22T10:00:01+08:00",
        )
        .expect("失败记录应写入");

        let rows: Vec<(String, i64, Option<String>)> = {
            let mut statement = db
                .conn
                .prepare("SELECT channel, ok, error FROM alert_delivery ORDER BY id")
                .expect("准备应成功");
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .expect("查询应成功")
                .collect::<Result<_, _>>()
                .expect("收集应成功")
        };
        assert_eq!(
            rows,
            vec![
                ("notification".to_string(), 1, None),
                (
                    "mail".to_string(),
                    0,
                    Some("550 relay not permitted".to_string())
                ),
            ]
        );
    }

    // 投递渠道配置：凭据必须加密落库、缺失与解不开要分得开、脏字段不能拖垮整份读取

    fn smtp_input() -> crate::deliver::SmtpInput {
        crate::deliver::SmtpInput {
            host: "smtp.qq.com".to_string(),
            port: 465,
            tls: "implicit".to_string(),
            user: "bot@qq.com".to_string(),
            auth_code: Some("qq-auth-code".to_string()),
            to: "me@qq.com".to_string(),
        }
    }

    fn channel_input(meow: bool, mail: bool, nickname: &str) -> crate::deliver::ChannelInput {
        crate::deliver::ChannelInput {
            notification: true,
            meow,
            mail,
            meow_nickname: nickname.to_string(),
            smtp: Some(smtp_input()),
        }
    }

    #[test]
    fn mail_auth_code_stays_encrypted_while_the_nickname_is_plain_config() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.set_delivery_channels(&channel_input(true, true, "  小明  "))
            .expect("写渠道配置应成功");

        let config = db.get_delivery_config().expect("读渠道配置应成功");
        assert_eq!(config.meow_nickname.as_deref(), Some("小明"));
        assert_eq!(config.mail_auth_code.as_deref(), Some("qq-auth-code"));
        assert_eq!(config.smtp.user, "bot@qq.com");
        assert_eq!(config.smtp.to, "me@qq.com");
        assert!(config.undecryptable.is_empty());
        assert!(config.problems.is_empty());

        // 授权码：明文只允许出现在解密后的返回值里，库里必须是密文
        let raw = db
            .get_config_text(KEY_SMTP_AUTH_CODE)
            .expect("读原始配置应成功")
            .unwrap_or_default();
        assert!(raw.starts_with(ENCRYPTED_PREFIX), "授权码未加密落库");
        assert!(!raw.contains("qq-auth-code"), "库里含授权码明文");

        // 昵称：普通明文配置，直接查库就该看得见，否则排障要先把机器搬过去
        assert_eq!(
            db.get_config_text(KEY_MEOW_NICKNAME)
                .expect("读原始配置应成功")
                .as_deref(),
            Some("小明")
        );
    }

    #[test]
    fn legacy_encrypted_nickname_still_reads_and_lands_plain_on_next_save() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.set_secret(KEY_MEOW_NICKNAME, "小明")
            .expect("按旧凭据口径写入应成功");

        assert_eq!(
            db.get_delivery_config()
                .expect("旧密文昵称不该让读取失败")
                .meow_nickname
                .as_deref(),
            Some("小明")
        );

        db.set_delivery_channels(&channel_input(true, false, "小明"))
            .expect("再保存一次应成功");
        assert_eq!(
            db.get_config_text(KEY_MEOW_NICKNAME)
                .expect("读原始配置应成功")
                .as_deref(),
            Some("小明"),
            "保存一次后不该还留着密文"
        );
    }

    #[test]
    fn omitting_the_auth_code_keeps_it_while_empty_nickname_clears_it() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.set_delivery_channels(&channel_input(true, true, "pika"))
            .expect("首次写入应成功");

        // 视图不回显授权码，所以它提交 None 表示「保持原值」；昵称每次整体覆盖
        let mut keep = channel_input(true, true, "pika");
        keep.smtp.as_mut().expect("应有 smtp").host = "smtp.163.com".into();
        keep.smtp.as_mut().expect("应有 smtp").auth_code = None;
        db.set_delivery_channels(&keep)
            .expect("保持授权码的写入应成功");
        let config = db.get_delivery_config().expect("读渠道配置应成功");
        assert_eq!(config.meow_nickname.as_deref(), Some("pika"));
        assert_eq!(config.mail_auth_code.as_deref(), Some("qq-auth-code"));
        assert_eq!(config.smtp.host, "smtp.163.com");

        let mut cleared = channel_input(false, false, "");
        cleared.smtp.as_mut().expect("应有 smtp").auth_code = Some("".into());
        db.set_delivery_channels(&cleared).expect("清空应成功");
        let config = db.get_delivery_config().expect("读渠道配置应成功");
        assert_eq!(config.meow_nickname, None);
        assert_eq!(config.mail_auth_code, None);
        assert!(
            db.get_config_text(KEY_MEOW_NICKNAME)
                .expect("读配置应成功")
                .is_none()
        );
        assert!(
            db.get_config_text(KEY_SMTP_AUTH_CODE)
                .expect("读配置应成功")
                .is_none()
        );
    }

    #[test]
    fn undecryptable_secret_is_reported_instead_of_looking_unconfigured() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.set_delivery_channels(&channel_input(false, true, ""))
            .expect("先按完整邮件配置写入应成功");
        db.set_config_text(KEY_SMTP_AUTH_CODE, "dpapi:v1:not-from-this-machine")
            .expect("写入外来密文应成功");

        let config = db
            .get_delivery_config()
            .expect("单个解不开的凭据不该让读取失败");
        assert_eq!(config.mail_auth_code, None);
        assert_eq!(config.undecryptable, vec![KEY_SMTP_AUTH_CODE.to_string()]);
        assert_eq!(
            config.availability(crate::deliver::Channel::Mail),
            crate::deliver::Availability::Incomplete
        );
        // 提示要说「重新填写」，不能让用户以为填过的东西凭空消失
        assert!(
            config
                .notice(crate::deliver::Channel::Mail)
                .unwrap_or_default()
                .contains("重新填写")
        );
    }

    #[test]
    fn dirty_smtp_fields_degrade_to_defaults_and_only_record_problems() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.set_config_text(KEY_SMTP_TLS, "plaintext")
            .expect("写脏 tls 应成功");
        db.set_config_text(KEY_SMTP_PORT, "465a")
            .expect("写脏 port 应成功");

        let config = db.get_delivery_config().expect("脏字段不该让读取失败");
        assert_eq!(config.smtp.tls, crate::deliver::TlsMode::Implicit);
        assert_eq!(config.smtp.port, 465);
        assert_eq!(config.problems.len(), 2, "{:?}", config.problems);
        // 一封配坏的邮件不能把系统通知一起拖掉
        assert!(config.notification);
        assert_eq!(
            db.get_alert_settings()
                .expect("告警参数仍可读")
                .quota_percent,
            10.0
        );
    }

    #[test]
    fn notification_channel_defaults_on_and_flags_round_trip() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let config = db.get_delivery_config().expect("空库应给出默认渠道");
        assert!(
            config.notification,
            "老用户不该因为没写过这个键就收不到通知"
        );
        assert!(!config.meow);
        assert!(!config.mail);

        db.set_delivery_channels(&channel_input(false, false, ""))
            .expect("整体覆盖应成功");
        let config = db.get_delivery_config().expect("读渠道配置应成功");
        assert!(config.notification);
        assert!(!config.usable().contains(&crate::deliver::Channel::Mail));
    }

    #[test]
    fn failure_count_is_scoped_to_one_rule_and_one_day() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let record = |db: &Db, rule: &str, day: &str, ok: bool| {
            db.record_alert_delivery(rule, day, "mail", ok, None, "2026-09-22T10:00:00+08:00")
                .expect("写投递记录应成功")
        };
        for _ in 0..3 {
            record(&db, "quota_low", "2026-09-22", false);
        }
        record(&db, "quota_low", "2026-09-22", true);
        record(&db, "spike", "2026-09-22", false);
        record(&db, "quota_low", "2026-09-21", false);

        assert_eq!(
            db.count_alert_failures("quota_low", "2026-09-22")
                .expect("计数应成功"),
            3
        );
        assert_eq!(
            db.count_alert_failures("spike", "2026-09-22")
                .expect("计数应成功"),
            1
        );
        assert_eq!(
            db.count_alert_failures("quota_low", "2026-09-21")
                .expect("计数应成功"),
            1
        );
        assert_eq!(
            db.count_alert_failures("runout_soon", "2026-09-22")
                .expect("无记录应返回 0"),
            0
        );
    }

    #[test]
    fn recent_deliveries_are_newest_first_and_limited_to_one_day() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let record = |db: &Db, rule: &str, channel: &str, day: &str, at: &str, ok: bool| {
            db.record_alert_delivery(rule, day, channel, ok, Some("boom"), at)
                .expect("写投递记录应成功")
        };
        record(
            &db,
            "quota_low",
            "notification",
            "2026-09-21",
            "2026-09-21T09:00:00+08:00",
            false,
        );
        record(
            &db,
            "quota_low",
            "notification",
            "2026-09-22",
            "2026-09-22T09:00:00+08:00",
            true,
        );
        record(
            &db,
            "spike",
            "mail",
            "2026-09-22",
            "2026-09-22T10:00:00+08:00",
            false,
        );

        let rows = db
            .recent_alert_deliveries("2026-09-22", 5)
            .expect("查投递记录应成功");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].rule, "spike");
        assert_eq!(rows[0].channel, "mail");
        assert!(!rows[0].ok);
        assert_eq!(rows[0].error.as_deref(), Some("boom"));
        assert_eq!(rows[1].channel, "notification");

        assert_eq!(
            db.recent_alert_deliveries("2026-09-22", 1)
                .expect("limit 应生效")
                .len(),
            1
        );
        assert!(
            db.recent_alert_deliveries("2026-09-20", 5)
                .expect("空日应返回空表")
                .is_empty()
        );
    }

    // 告警接线里「从库里取基准/次数」这两步的真实数据通路（被测函数在 crate 根，
    // 放这里是因为只有本模块能直接塞带 day 列的历史行）

    #[test]
    fn alert_baseline_takes_day_end_row_and_excludes_today() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let seed = |db: &Db, day: &str, yuan: f64| {
            db.conn
                .execute(
                    "INSERT INTO dashboard_snapshot
                         (saved_at, day, today_yuan, total_tokens, remaining, used, total, request_count)
                     VALUES (?1, ?2, ?3, 0, 0, 0, 100, 0)",
                    rusqlite::params![format!("{day}T12:00:00+08:00"), day, yuan],
                )
                .expect("插入快照应成功");
        };
        seed(&db, "2026-09-20", 3.0);
        seed(&db, "2026-09-20", 9.0); // 同日晚些的末条，基准应取这条
        seed(&db, "2026-09-21", 99.0); // 当日，不参与基准

        let baseline = crate::alert_baseline(&db, "2026-09-21").expect("基准应可计算");
        assert_eq!(baseline.sample_days, 1, "当日不得进入基准");
        assert_eq!(baseline.avg_yuan, Some(9.0), "同日多行取末条而非首条");
    }

    #[test]
    fn alert_baseline_window_reaches_seven_days_back() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let seed = |db: &Db, day: &str| {
            db.conn
                .execute(
                    "INSERT INTO dashboard_snapshot
                         (saved_at, day, today_yuan, total_tokens, remaining, used, total, request_count)
                     VALUES (?1, ?2, 5.0, 0, 0, 0, 100, 0)",
                    rusqlite::params![format!("{day}T12:00:00+08:00"), day],
                )
                .expect("插入快照应成功");
        };
        seed(&db, "2026-09-14"); // today 前第 7 天，在窗口内
        seed(&db, "2026-09-13"); // 第 8 天，应在窗口外
        seed(&db, "2026-09-19");
        seed(&db, "2026-09-20");
        seed(&db, "2026-09-21"); // 当日

        let baseline = crate::alert_baseline(&db, "2026-09-21").expect("基准应可计算");
        assert_eq!(
            baseline.sample_days, 3,
            "窗口是 today-7 ..= today-1（09-14/09-19/09-20），09-13 与当日都不参与"
        );
    }

    #[test]
    fn alert_day_state_counts_every_rule_fired_today() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.record_alert_fire("quota_low", "2026-09-22", "2026-09-22T10:00:00+08:00")
            .expect("记录应成功");
        db.record_alert_fire("quota_low", "2026-09-22", "2026-09-22T10:10:00+08:00")
            .expect("记录应成功");
        db.record_alert_fire("spike", "2026-09-21", "2026-09-21T10:00:00+08:00")
            .expect("记录应成功");

        let state = crate::alert_day_state(&db, "2026-09-22").expect("读取应成功");
        assert_eq!(state.total, 2, "只算当日的次数");
        assert_eq!(
            state,
            crate::alert::DayState::from_counts(&[("quota_low".to_string(), 2)])
        );
    }

    /// 把 `run_alerts` 的数据通路（读设置/读次数/读基准/记账）串起来跑两轮，
    /// 只去掉系统通知与事件推送这两件必须由 AppHandle 做的事。
    #[test]
    fn alert_pipeline_stops_refiring_within_the_same_day() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        for (day, yuan) in [
            ("2026-09-19", 1.0),
            ("2026-09-20", 2.0),
            ("2026-09-21", 3.0),
            ("2026-09-22", 100.0),
        ] {
            db.conn
                .execute(
                    "INSERT INTO dashboard_snapshot
                         (saved_at, day, today_yuan, total_tokens, remaining, used, total, request_count)
                     VALUES (?1, ?2, ?3, 0, 0, 0, 100, 0)",
                    rusqlite::params![format!("{day}T12:00:00+08:00"), day, yuan],
                )
                .expect("插入快照应成功");
        }

        let day = "2026-09-22";
        let settings = crate::alert::AlertSettings::default(); // 水位 10% / 燃尽 5 天 / 3 倍 / 每日 2 次
        let current = crate::alert::Current {
            percent: 5.0,
            remaining: 5.0,
            total: 100.0,
            today_yuan: 100.0,
        };
        let evaluate = |db: &Db| -> Vec<String> {
            let state = crate::alert_day_state(db, day).expect("次数应可读");
            let baseline = crate::alert_baseline(db, day).expect("基准应可算");
            let rules = crate::alert::load_rules_enabled(db);
            crate::alert::detect(&current, &baseline, &settings, &rules, &state)
                .into_iter()
                .map(|finding| finding.rule.to_string())
                .collect()
        };

        assert_eq!(
            evaluate(&db),
            vec!["quota_low", "runout_soon"],
            "三条都命中，但每日上限 2 → 按声明顺序取前两条"
        );
        for rule in ["quota_low", "runout_soon"] {
            db.record_alert_fire(rule, day, "2026-09-22T12:00:00+08:00")
                .expect("记账应成功");
        }
        assert!(evaluate(&db).is_empty(), "同日第二轮应被每日上限拦住");

        // 次日：alert_state 按 day 分行，次数归零后重新可投
        let next_day = "2026-09-23";
        let state = crate::alert_day_state(&db, next_day).expect("次数应可读");
        assert_eq!(state.total, 0, "次日应从零计数");
        let findings = {
            let baseline = crate::alert_baseline(&db, next_day).expect("基准应可算");
            crate::alert::detect(
                &current,
                &baseline,
                &settings,
                &crate::alert::load_rules_enabled(&db),
                &state,
            )
        };
        assert_eq!(
            findings.len(),
            2,
            "次日仍应能报（当日花费仍是基准的很多倍）"
        );
    }

    /// 线上 0.2.4 的库正是这个形态：有 config 与旧版 dashboard_snapshot，但没有任何告警表。
    /// 落盘跑一遍「旧库 → Db::open」才是真迁移，内存库里的建表语句已经新过了，测不到这条路径。
    #[test]
    fn legacy_database_on_disk_upgrades_and_gets_alert_tables() {
        let path = std::env::temp_dir().join(format!(
            "amax-alert-migration-{}-{}.db",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let legacy_saved_at = local_rfc3339("2026-08-01T08:00:00");
        {
            let conn = rusqlite::Connection::open(&path).expect("建旧库应成功");
            conn.execute_batch(&format!(
                "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO config(key, value) VALUES('cookie', 'session=legacy');
                 CREATE TABLE dashboard_snapshot (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     saved_at TEXT NOT NULL,
                     today_yuan REAL NOT NULL,
                     total_tokens INTEGER NOT NULL,
                     remaining REAL NOT NULL,
                     used REAL NOT NULL,
                     total REAL NOT NULL
                 );
                 INSERT INTO dashboard_snapshot
                     (saved_at, today_yuan, total_tokens, remaining, used, total)
                 VALUES ('{legacy_saved_at}', 7.0, 10, 93.0, 7.0, 100.0);"
            ))
            .expect("写旧 schema 应成功");
        }

        let upgraded = Db::open(&path).expect("旧库应能按新 schema 打开");
        assert_eq!(
            upgraded.get_cookie().as_deref(),
            Some("session=legacy"),
            "旧凭据必须原样可读"
        );
        let rows = upgraded
            .get_daily_snapshots("2026-08-01", "2026-08-01")
            .expect("历史快照应可查");
        assert_eq!(rows.len(), 1, "缺 day 列的历史行应被回填并纳入按日查询");
        assert_eq!(rows[0].date, "2026-08-01");
        assert_eq!(rows[0].yuan, 7.0);

        assert!(
            upgraded
                .get_alert_day_counts("2026-08-01")
                .expect("告警表应已建好")
                .is_empty(),
            "旧库升级后当日没有投递记录"
        );
        upgraded
            .record_alert_fire("quota_low", "2026-08-01", "2026-08-01T09:00:00+08:00")
            .expect("升级后应可直接记账");
        assert_eq!(
            upgraded
                .get_alert_day_counts("2026-08-01")
                .expect("读取应成功")
                .len(),
            1
        );
        // 参数缺省即默认值，不因为旧库没有这些键而报错
        assert_eq!(
            upgraded.get_alert_settings().expect("缺省参数应可读"),
            crate::alert::AlertSettings::default()
        );

        // 再次打开必须仍是 no-op（幂等）
        drop(upgraded);
        let reopened = Db::open(&path).expect("重复打开应幂等");
        assert_eq!(
            reopened
                .get_alert_day_counts("2026-08-01")
                .expect("记账应跨进程保留")
                .len(),
            1
        );
        // Windows 上连接未关闭就删临时库会被文件锁挡住，必须先 drop 再清理
        drop(reopened);
        std::fs::remove_file(&path).expect("清理临时库应成功");
    }
}
