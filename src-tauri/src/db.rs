//! SQLite 持久化模块 — cookie / api_key 加密存储

use crate::crypto;
use crate::error::AppError;
use chrono::{Local, NaiveDate};
use rusqlite::{Connection, params};

const ENCRYPTED_PREFIX: &str = "dpapi:v1:";
/// 服务端下发的 Cookie 到期时间（本地时区 RFC3339）。非机密，明文存；
/// 仅用于展示，不参与任何失效判定（失效由服务端返回 401/403 决定）
const KEY_COOKIE_EXPIRES_AT: &str = "cookie_expires_at";

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
                ON dashboard_snapshot(saved_at);",
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
    use super::{DailySnapshot, Db, derive_daily_requests};

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
}
