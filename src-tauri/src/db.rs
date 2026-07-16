//! SQLite 持久化模块 — cookie / api_key 加密存储 + 过期检测

use crate::crypto;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

const ENCRYPTED_PREFIX: &str = "dpapi:v1:";
const COOKIE_VALID_DAYS: i64 = 15;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &std::path::Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS config (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dashboard_snapshot (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                saved_at     TEXT NOT NULL,
                today_yuan   REAL NOT NULL,
                total_tokens INTEGER NOT NULL,
                remaining    REAL NOT NULL,
                used         REAL NOT NULL,
                total        REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_dashboard_snapshot_saved_at
                ON dashboard_snapshot(saved_at);",
        )?;
        Ok(Self { conn })
    }

    fn get_raw(&self, key: &str) -> Result<Option<String>, rusqlite::Error> {
        let mut statement = self.conn.prepare("SELECT value FROM config WHERE key=?1")?;
        let mut rows = statement.query(params![key])?;
        rows.next()?.map(|row| row.get(0)).transpose()
    }

    fn encrypt_secret(value: &str) -> Result<String, String> {
        crypto::encrypt(value.as_bytes()).map(|encrypted| format!("{ENCRYPTED_PREFIX}{encrypted}"))
    }

    fn decrypt_secret(raw: String) -> Option<String> {
        let Some(encrypted) = raw.strip_prefix(ENCRYPTED_PREFIX) else {
            return Some(raw);
        };
        crypto::decrypt(encrypted)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn save_config(&mut self, cookie: &str, api_key: &str) -> Result<(), String> {
        let encrypted_cookie = (!cookie.is_empty())
            .then(|| Self::encrypt_secret(cookie))
            .transpose()?;
        let encrypted_api_key = (!api_key.is_empty())
            .then(|| Self::encrypt_secret(api_key))
            .transpose()?;

        if encrypted_cookie.is_none() && encrypted_api_key.is_none() {
            return Ok(());
        }

        let transaction = self.conn.transaction().map_err(|error| error.to_string())?;
        if let Some(value) = encrypted_cookie {
            transaction
                .execute(
                    "INSERT INTO config(key, value) VALUES('cookie', ?1)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![value],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "INSERT INTO config(key, value) VALUES('cookie_saved_at', ?1)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![Utc::now().to_rfc3339()],
                )
                .map_err(|error| error.to_string())?;
        }
        if let Some(value) = encrypted_api_key {
            transaction
                .execute(
                    "INSERT INTO config(key, value) VALUES('api_key', ?1)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![value],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn get_cookie(&self) -> Option<String> {
        Self::decrypt_secret(self.get_raw("cookie").ok()??)
    }

    pub fn has_cookie(&self) -> bool {
        self.get_cookie().is_some_and(|cookie| !cookie.is_empty())
    }

    pub fn is_cookie_expired(&self) -> bool {
        self.get_raw("cookie_saved_at")
            .ok()
            .flatten()
            .and_then(|timestamp| DateTime::parse_from_rfc3339(&timestamp).ok())
            .is_none_or(|saved_at| Utc::now() - Duration::days(COOKIE_VALID_DAYS) > saved_at)
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
    ) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO dashboard_snapshot (saved_at, today_yuan, total_tokens, remaining, used, total)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                Utc::now().to_rfc3339(),
                today_yuan,
                total_tokens,
                remaining,
                used,
                total
            ],
        )?;
        self.conn.execute(
            "DELETE FROM dashboard_snapshot
             WHERE datetime(saved_at) < datetime('now', '-90 days')",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Db;
    use chrono::{Duration, Utc};
    use rusqlite::params;

    fn db_with_cookie_saved_at(saved_at: &str) -> Db {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        db.conn
            .execute(
                "INSERT INTO config(key, value) VALUES('cookie_saved_at', ?1)",
                params![saved_at],
            )
            .expect("保存 Cookie 时间应成功");
        db
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

    #[test]
    fn cookie_saved_less_than_fifteen_days_ago_is_not_expired() {
        let saved_at = (Utc::now() - Duration::days(14)).to_rfc3339();
        let db = db_with_cookie_saved_at(&saved_at);

        assert!(!db.is_cookie_expired());
    }

    #[test]
    fn cookie_saved_more_than_fifteen_days_ago_is_expired() {
        let saved_at = (Utc::now() - Duration::days(16)).to_rfc3339();
        let db = db_with_cookie_saved_at(&saved_at);

        assert!(db.is_cookie_expired());
    }

    #[test]
    fn missing_cookie_saved_at_is_expired() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");

        assert!(db.is_cookie_expired());
    }

    #[test]
    fn invalid_cookie_saved_at_is_expired() {
        let db = db_with_cookie_saved_at("invalid-timestamp");

        assert!(db.is_cookie_expired());
    }
}
