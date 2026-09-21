//! 统一错误类型 — 前端按 `code` 分支，`message` 只用于展示
//!
//! 分级语义：`auth` 表示凭据问题（重试无用，必须重新登录）；`network` 表示网络或官网
//! 暂时不可用（可重试）；`data` 表示响应解析与接口契约异常；`input` 表示入参非法；
//! `storage` 表示本机故障（数据库、加解密、窗口）。
//! 命令返回值序列化为 `{ code, message }` 作为 `invoke()` 的 reject 载荷，
//! 前端不再用文案子串（`includes('认证失败')` / `includes('401')`）判定错误类别。

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorCode {
    Auth,
    Network,
    Data,
    Input,
    Storage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn auth(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Auth, message)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Network, message)
    }

    pub fn data(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Data, message)
    }

    pub fn input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Input, message)
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Storage, message)
    }

    pub fn is_auth(&self) -> bool {
        self.code == ErrorCode::Auth
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        Self::storage(format!("本地数据库操作失败: {error}"))
    }
}

/// 后台任务/日志侧把锁中毒转成可读错误（`Mutex` 中毒是进程内状态损坏，归 `storage`）
pub fn poisoned(what: &str) -> AppError {
    AppError::storage(format!("{what}状态已损坏，请重启应用"))
}

#[cfg(test)]
mod tests {
    use super::{AppError, ErrorCode};

    #[test]
    fn serializes_code_and_message() {
        let value = serde_json::to_value(AppError::auth("Cookie 无效")).expect("应可序列化");
        assert_eq!(value["code"], "auth");
        assert_eq!(value["message"], "Cookie 无效");
    }

    #[test]
    fn codes_serialize_to_lowercase_strings() {
        for (error, expected) in [
            (AppError::network("a"), "network"),
            (AppError::data("b"), "data"),
            (AppError::input("c"), "input"),
            (AppError::storage("d"), "storage"),
        ] {
            assert_eq!(
                serde_json::to_value(&error).expect("应可序列化")["code"],
                expected
            );
        }
    }

    #[test]
    fn display_shows_only_message() {
        let error = AppError::input("日期格式错误");
        assert_eq!(error.to_string(), "日期格式错误");
    }

    #[test]
    fn only_auth_is_auth() {
        assert!(AppError::auth("x").is_auth());
        assert!(!AppError::data("x").is_auth());
        assert!(!AppError::network("x").is_auth());
    }

    #[test]
    fn rusqlite_error_maps_to_storage() {
        let error = AppError::from(rusqlite::Error::QueryReturnedNoRows);
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(error.message.contains("本地数据库操作失败"));
    }
}
