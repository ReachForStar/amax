//! SQLite 持久化模块 — cookie / api_key 加密存储 + 过期检测

use crate::crypto;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

const ENCRYPTED_PREFIX: &str = "dpapi:v1:";

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
        let encrypted = raw.strip_prefix(ENCRYPTED_PREFIX)?;
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
            .is_none_or(|saved_at| Utc::now() - Duration::hours(24) > saved_at)
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
