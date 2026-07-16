# Cookie 固定 15 天有效期 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 AMAX Dashboard 的 Cookie 本地有效期从保存后 24 小时调整为固定 15×24 小时。

**Architecture:** 只修改 `Db::is_cookie_expired` 所属的 SQLite 持久化模块。用模块级命名常量表达 15 天业务规则，保留时间戳缺失、读取失败或格式无效时视为过期的现有安全行为；不改变数据库、加密格式、IPC 或前端。

**Tech Stack:** Rust、chrono、rusqlite、Cargo test/clippy/fmt

## Global Constraints

- “半个月”固定解释为 15×24 小时，不随自然月长度变化。
- 已有 Cookie 自动采用新阈值，不执行数据迁移。
- 不修改数据库结构、既有时间戳、Cookie/API Key 加密格式、IPC command、前端接口或网络请求。
- 时间戳缺失、数据库读取失败或格式无效时仍视为 Cookie 已过期。
- 测试不得依赖精确瞬时时钟边界，使用明显位于 15 天阈值两侧的时间值。

---

## 文件结构

- 修改：`src-tauri/src/db.rs`——定义 Cookie 有效期常量、执行过期判断，并在同文件测试模块中覆盖有效、过期、缺失和无效时间戳。
- 不创建生产代码文件；该行为属于现有 `Db` 持久化职责，拆分文件会扩大范围且没有独立接口收益。

### Task 1: Cookie 固定 15 天过期判定

**Files:**
- Modify: `src-tauri/src/db.rs:7-8`
- Modify: `src-tauri/src/db.rs:104-110`
- Test: `src-tauri/src/db.rs:149-170`

**Interfaces:**
- Consumes: `config.cookie_saved_at`，值为 RFC 3339 时间字符串；`chrono::{DateTime, Duration, Utc}`。
- Produces: `Db::is_cookie_expired(&self) -> bool`，在保存时间超过固定 15 天、时间戳缺失或无效时返回 `true`。

- [ ] **Step 1: 写入会失败的 15 天阈值测试**

在 `src-tauri/src/db.rs` 的 `tests` 模块中将导入扩展为：

```rust
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
```

追加测试：

```rust
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
```

- [ ] **Step 2: 运行阈值测试并确认先失败**

Run:

```bash
cargo test --workspace cookie_saved_less_than_fifteen_days_ago_is_not_expired -- --exact --nocapture
```

Expected: FAIL；`cookie_saved_less_than_fifteen_days_ago_is_not_expired` 在现有 24 小时阈值下触发 `assert!(!db.is_cookie_expired())` 失败。

- [ ] **Step 3: 用命名常量实现固定 15 天有效期**

在 `ENCRYPTED_PREFIX` 后添加：

```rust
const COOKIE_VALID_DAYS: i64 = 15;
```

将 `Db::is_cookie_expired` 改为：

```rust
pub fn is_cookie_expired(&self) -> bool {
    self.get_raw("cookie_saved_at")
        .ok()
        .flatten()
        .and_then(|timestamp| DateTime::parse_from_rfc3339(&timestamp).ok())
        .is_none_or(|saved_at| Utc::now() - Duration::days(COOKIE_VALID_DAYS) > saved_at)
}
```

- [ ] **Step 4: 运行全部 db 模块测试并确认通过**

Run:

```bash
cargo test --workspace db::tests -- --nocapture
```

Expected: PASS；新增四个过期测试与已有加密兼容测试全部通过。

- [ ] **Step 5: 运行完整静态检查和测试**

Run:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node --check dist/app.js
```

Expected: 所有命令退出码均为 0。该变更不涉及窗口、托盘、前端交互或数据展示，无需启动真实应用验证。

- [ ] **Step 6: 审查精确差异**

Run:

```bash
git diff --check
git diff -- src-tauri/src/db.rs
git status --short
```

Expected: 无空白错误；生产代码仅新增 `COOKIE_VALID_DAYS` 并将 24 小时阈值替换为 15 天；其余差异仅为同文件单元测试。

- [ ] **Step 7: 调用 Rust 代码审查并处理阻塞问题**

调用 `ecc:rust-reviewer` 审查 `src-tauri/src/db.rs` 的改动，重点确认：

- 时间比较方向正确。
- 固定 15×24 小时语义明确。
- 缺失或无效时间戳仍安全地视为过期。
- 测试不会因精确边界或执行耗时而不稳定。

若发现阻塞问题，修复后重新执行 Step 5。

- [ ] **Step 8: 仅提交并推送本任务文件**

Run:

```bash
git add src-tauri/src/db.rs
git commit -m "自动提交：延长 Cookie 有效期至十五天"
git push
```

Expected: 提交只包含 `src-tauri/src/db.rs`，并成功推送当前分支。
