# 数据统计分析功能实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 AMAX Dashboard 新增独立统计页，提供消耗/请求数/余额趋势、模型分布、区间汇总与三格式导出，官方数据为主、本地快照对比与降级。

**Architecture:** Rust 后端新增两个 IPC command（`fetch_usage_stats` 走官网 by-model 接口聚合，`get_local_stats` 纯 SQLite 查询 + 差分）；前端新增第三屏统计页，Chart.js 渲染四块图表，纯前端生成 CSV/JSON/XLSX 导出。

**Tech Stack:** Rust (Tauri 2 / rusqlite / chrono / reqwest / serde)、原生 JS、Chart.js 4 UMD、SheetJS 0.20.3 UMD。

## Global Constraints

- 金额口径不变：`yuan = quota / QUOTA_PER_YUAN`，`QUOTA_PER_YUAN = 500_000.0`。
- Token 统计以官方 API 为主基准；官方 `quota` 全 0 时模型占比自动切换 `total_tokens` 口径（`percent_basis`）。
- 日期参数格式 `YYYY-MM-DD`；`start <= end <= 今天`；跨度上限 1096 天。
- 快照永久保留，废除 90 天清理；`dashboard_snapshot` 新增 `request_count` 列，幂等迁移。
- 前端无构建步骤，vendor 库以单文件 UMD 放入 `dist/vendor/`；CSP `script-src 'self'` 不可改。
- 提交信息中文祈使句，主题 ≤50 字符；`git add` 仅具体文件，禁止 `git add .`。
- 注释使用简体中文，与现有代码库一致。
- 涉及前端交互的任务完成后必须 `cargo tauri dev` 真实验证（Task 9 统一执行）。

---

### Task 1: db.rs — 快照迁移、永久保留与每日查询

**Files:**
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/lib.rs`（`save_snapshot` 调用点传参同步）

**Interfaces:**
- Produces（后续任务依赖）：
  - `Db::save_snapshot(&self, today_yuan: f64, total_tokens: i64, remaining: f64, used: f64, total: f64, request_count: i64) -> Result<(), rusqlite::Error>`
  - `Db::get_daily_snapshots(&self, start_date: &str, end_date: &str) -> Result<Vec<DailySnapshot>, rusqlite::Error>`
  - `Db::migrate_schema(conn: &Connection) -> Result<(), rusqlite::Error>`
  - `pub struct DailySnapshot { pub date: String, pub yuan: f64, pub tokens: i64, pub remaining: f64, pub request_count: Option<i64> }`
  - `pub fn derive_daily_requests(snapshots: &[DailySnapshot]) -> Vec<Option<i64>>`
  - `pub fn summarize_local(snapshots: &[DailySnapshot]) -> (f64, f64, f64, String, i64)` → `(total_yuan, avg_yuan, peak_yuan, peak_date, total_tokens)`

- [ ] **Step 1: 写失败测试 — 每日查询取日末条、边界与空表**

在 `db.rs` 的 `#[cfg(test)] mod tests` 内，先扩展辅助函数并新增测试：

```rust
    fn db_with_snapshots() -> Db {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        // (saved_at, today_yuan, total_tokens, remaining, request_count)
        let rows = [
            ("2026-08-01 08:00:00", 5.0, 100, 90.0, Some(10)),
            ("2026-08-01 20:00:00", 12.0, 300, 83.0, Some(25)), // 8-01 末条
            ("2026-08-02 10:00:00", 3.0, 80, 80.0, None),       // 老数据模拟 NULL
        ];
        for (saved_at, yuan, tokens, remaining, req) in rows {
            db.conn
                .execute(
                    "INSERT INTO dashboard_snapshot
                         (saved_at, today_yuan, total_tokens, remaining, used, total, request_count)
                     VALUES (datetime(?1), ?2, ?3, ?4, 0, 100, ?5)",
                    rusqlite::params![saved_at, yuan, tokens, remaining, req],
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
    fn daily_snapshots_empty_table_returns_empty() {
        let db = Db::open(std::path::Path::new(":memory:")).expect("内存数据库应打开成功");
        let rows = db
            .get_daily_snapshots("2026-08-01", "2026-08-02")
            .expect("查询应成功");
        assert!(rows.is_empty());
    }
```

时区口径（全任务统一）：`saved_at` 改用**本地时区** RFC3339 写入（`Local::now().to_rfc3339()`），查询侧按 `date(saved_at)` 分组截取即本地日期，不带 `'localtime'` 修饰，避免运行机时区与测试断言耦合。测试数据以 `"YYYY-MM-DD HH:MM:SS"` 字符串经 `datetime(?1)` 规范化写入，`date()` 截取结果与字面日期一致。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --workspace daily_snapshots -- --nocapture`
Expected: FAIL（`get_daily_snapshots` 未定义，编译错误）

- [ ] **Step 3: 实现迁移、save_snapshot 改造与 get_daily_snapshots**

`db.rs` 顶部导入调整：

```rust
use chrono::Local;
use rusqlite::{Connection, params};
```

（移除不再使用的 `Utc`；`Duration`/`DateTime` 上一轮已删。）

`Db::open` 尾部调用迁移：

```rust
    pub fn open(path: &std::path::Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS config (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dashboard_snapshot (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                saved_at      TEXT NOT NULL,
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

    /// 旧库迁移：补齐 request_count 列；幂等，新旧表均可安全执行
    pub fn migrate_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
        let has_column = {
            let mut statement = conn.prepare("PRAGMA table_info(dashboard_snapshot)")?;
            let names = statement.query_map([], |row| row.get::<_, String>(1))?;
            names.any(|name| name.as_deref() == Some("request_count"))
        };
        if !has_column {
            conn.execute(
                "ALTER TABLE dashboard_snapshot ADD COLUMN request_count INTEGER",
                [],
            )?;
        }
        Ok(())
    }
```

`save_snapshot` 增参并移除 90 天清理、改用本地时区写入：

```rust
    pub fn save_snapshot(
        &self,
        today_yuan: f64,
        total_tokens: i64,
        remaining: f64,
        used: f64,
        total: f64,
        request_count: i64,
    ) -> Result<(), rusqlite::Error> {
        // 本地时区 RFC3339：date(saved_at) 截取即本地日期，与按日分组口径一致
        self.conn.execute(
            "INSERT INTO dashboard_snapshot
                 (saved_at, today_yuan, total_tokens, remaining, used, total, request_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                Local::now().to_rfc3339(),
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
```

查询与数据结构：

```rust
/// 每日快照行 — 取当日最后一条
#[derive(Debug, Clone)]
pub struct DailySnapshot {
    pub date: String,
    pub yuan: f64,
    pub tokens: i64,
    pub remaining: f64,
    pub request_count: Option<i64>,
}

// impl Db 内：
    pub fn get_daily_snapshots(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<DailySnapshot>, rusqlite::Error> {
        let mut statement = self.conn.prepare(
            "SELECT date(saved_at) AS day, today_yuan, total_tokens, remaining, request_count
             FROM dashboard_snapshot
             WHERE id IN (SELECT MAX(id) FROM dashboard_snapshot GROUP BY date(saved_at))
               AND day BETWEEN ?1 AND ?2
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
        rows.collect()
    }
```

同步修改 `lib.rs` 中 `save_snapshot` 调用点（`refresh_dashboard` 内）：

```rust
        db.save_snapshot(
            data.today_yuan,
            data.today_tokens,
            data.remaining,
            data.used,
            data.total,
            data.request_count,
        )
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --workspace daily_snapshots -- --nocapture`
Expected: PASS（2 个测试）

- [ ] **Step 5: 写失败测试 — 请求数差分**

```rust
    use super::{DailySnapshot, derive_daily_requests};

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
        assert_eq!(
            derive_daily_requests(&rows),
            vec![None, None, None, None]
        );
    }

    #[test]
    fn daily_requests_empty_input() {
        assert!(derive_daily_requests(&[]).is_empty());
    }
```

- [ ] **Step 6: 运行测试确认失败**

Run: `cargo test --workspace daily_requests -- --nocapture`
Expected: FAIL（`derive_daily_requests` 未定义）

- [ ] **Step 7: 实现差分与汇总纯函数**

```rust
use chrono::{Local, NaiveDate};

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
    let avg_yuan = if days > 0 { total_yuan / days as f64 } else { 0.0 };
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
```

- [ ] **Step 8: 运行全部 db 测试**

Run: `cargo test --workspace db:: -- --nocapture`
Expected: PASS（含既有加解密测试 + 5 个新测试）

- [ ] **Step 9: 写迁移幂等测试**

```rust
    #[test]
    fn migrate_schema_is_idempotent_and_upgrades_legacy() {
        let conn = rusqlite::Connection::open_in_memory().expect("内存数据库应打开成功");
        // 构造旧 schema（无 request_count 列）
        conn.execute_batch(
            "CREATE TABLE dashboard_snapshot (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                saved_at TEXT NOT NULL,
                today_yuan REAL NOT NULL,
                total_tokens INTEGER NOT NULL,
                remaining REAL NOT NULL,
                used REAL NOT NULL,
                total REAL NOT NULL
            );",
        )
        .expect("建旧表应成功");

        super::Db::migrate_schema(&conn).expect("首次迁移应成功");
        super::Db::migrate_schema(&conn).expect("重复迁移应幂等成功");

        let mut statement = conn.prepare("PRAGMA table_info(dashboard_snapshot)").expect("准备应成功");
        let names: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("查询应成功")
            .collect::<Result<_, _>>()
            .expect("收集应成功");
        assert!(names.iter().any(|name| name == "request_count"));
    }
```

- [ ] **Step 10: 运行测试确认通过**

Run: `cargo test --workspace migrate_schema -- --nocapture`
Expected: PASS

- [ ] **Step 11: 静态检查**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 无警告、无错误

- [ ] **Step 12: 提交**

```bash
git add src-tauri/src/db.rs src-tauri/src/lib.rs
git commit -m "feat: 快照新增请求数字段与每日查询"
```

---

### Task 2: api.rs — 官方用量解析与聚合

**Files:**
- Modify: `src-tauri/src/api.rs`

**Interfaces:**
- Consumes：Task 1 无直接依赖（模块独立）。
- Produces：
  - `pub async fn fetch_usage_stats(client: &reqwest::Client, cookie: &str, start: NaiveDate, end: NaiveDate) -> Result<UsageStats, String>`
  - `pub struct UsageStats { pub daily: Vec<UsageDaily>, pub models: Vec<UsageModel>, pub summary: UsageSummaryStats, pub percent_basis: String }`（全部 `Serialize + Clone`）
  - `UsageDaily { date: String, yuan: f64, tokens: i64, input_tokens: i64, output_tokens: i64 }`
  - `UsageModel { model: String, yuan: f64, tokens: i64, request_count: i64, percent: f64 }`
  - `UsageSummaryStats { total_yuan: f64, avg_yuan: f64, peak_yuan: f64, peak_date: String, total_tokens: i64, request_count: i64, days_with_usage: usize }`
  - 模块内私有纯函数 `aggregate_usage(response: UsageResponse, start: NaiveDate, end: NaiveDate) -> UsageStats`（同模块测试可直接调用）
  - 重构提取：`async fn fetch_user_info(client, cookie) -> Result<UserInfo, String>`（`fetch_dashboard` 与 `fetch_usage_stats` 共用，消除 user/self 重复逻辑）

- [ ] **Step 1: 写失败测试 — 实测响应样本解析**

`api.rs` 测试模块内新增（样本取自 2026-08-02 实测响应，字段精简）：

```rust
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
        let payload: super::UsageResponse = serde_json::from_str(
            r#"{ "summary": { "total_tokens": 1, "quota": 1.0 } }"#,
        )
        .expect("应兼容缺失明细字段的响应");

        assert!(payload.models.is_empty());
        assert!(payload.daily.is_empty());
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --workspace parses_usage_response -- --nocapture`
Expected: FAIL（`UsageResponse` 无 `models` / `daily` 字段，编译或断言错误）

- [ ] **Step 3: 扩展响应结构并实现解析**

`api.rs` 中替换 `UsageResponse` 并新增原始结构（保留现有 `UsageSummary`，补 `#[serde(default)]`）：

```rust
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
```

`UsageSummary` 同样补 `#[serde(default)]`（结构体与字段两层），保证缺字段时默认 0。

- [ ] **Step 4: 写失败测试 — 聚合规则**

```rust
    use chrono::NaiveDate;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
    }

    fn raw_quota_model(name: &str, quota: f64, tokens: i64, requests: i64) -> super::ModelUsageRaw {
        super::ModelUsageRaw { model: name.into(), request_count: requests, total_tokens: tokens, quota }
    }

    fn raw_daily(date: NaiveDate, quota: f64, tokens: i64) -> super::DailyUsageRaw {
        // 固定以 UTC+8 当日 0 点构造时间戳，aggregate 内部用 Local 换算，
        // 测试机时区非 UTC+8 时改用 date.and_hms_opt(0,0,0) 的本地时间戳
        super::DailyUsageRaw {
            date: date.and_hms_opt(0, 0, 0).expect("合法时间").and_local_timezone(chrono::Local).single().expect("本地时区换算应成功").timestamp(),
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
            daily: vec![
                raw_daily(d1, 2.0, 200),
                { let mut r = raw_daily(d1, 1.0, 100); r.input_tokens = 50; r.output_tokens = 50; r },
                raw_daily(d3, 1.0, 100),
            ],
        };

        let stats = super::aggregate_usage(response, d1, d3);

        // 三日连续，8-02 补 0
        assert_eq!(stats.daily.len(), 3);
        assert_eq!(stats.daily[0].date, "2026-08-01");
        assert!((stats.daily[0].yuan - 3.0).abs() < 1e-9);   // 跨模型求和
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
```

- [ ] **Step 5: 运行测试确认失败**

Run: `cargo test --workspace aggregate -- --nocapture`
Expected: FAIL（`aggregate_usage` 未定义）

- [ ] **Step 6: 实现输出结构与聚合纯函数**

`api.rs` 顶部导入补充：`use chrono::{Datelike, Local, NaiveDate, TimeZone}; use std::collections::BTreeMap;`

输出结构（`Serialize` 供 command 直接返回）：

```rust
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
```

聚合实现：

```rust
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
        else { continue };
        if seen.insert(date) { days_with_usage += 1; }
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
    let avg_yuan = if days_with_usage > 0 { total_yuan / days_with_usage as f64 } else { 0.0 };
    let mut peak_yuan = 0.0_f64;
    let mut peak_found = false;
    let mut peak_date = date_to_string(end); // 全 0 回退 end_date
    for row in &daily {
        if !peak_found || row.yuan > peak_yuan {
            peak_yuan = row.yuan;
            peak_found = true;
            if row.yuan > 0.0 { peak_date = row.date.clone(); }
        }
    }
    let request_count: i64 = response.models.iter().map(|m| m.request_count).sum();

    // 一致性校验：补零后累计应等于 summary 上报值
    if (total_tokens - response.summary.total_tokens).abs() > 0 {
        eprintln!(
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
        .map(|m| if use_tokens_basis { m.total_tokens as f64 } else { m.quota })
        .sum();
    let mut models: Vec<UsageModel> = response
        .models
        .iter()
        .map(|m| {
            let basis_value = if use_tokens_basis { m.total_tokens as f64 } else { m.quota };
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
        let (av, bv) = if use_tokens_basis { (a.tokens as f64, b.tokens as f64) } else { (a.yuan, b.yuan) };
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
```

- [ ] **Step 7: 运行聚合测试确认通过**

Run: `cargo test --workspace aggregate -- --nocapture`
Expected: PASS（3 个测试）

- [ ] **Step 8: 提取 fetch_user_info 并实现 fetch_usage_stats**

将 `fetch_dashboard` 中 `/api/user/self` 请求段（含 401/403/429 错误分级）提取为私有函数，`fetch_dashboard` 改为调用它：

```rust
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
```

`fetch_usage_stats`（统计 command 专用，by-model 失败直接 Err 触发前端降级）：

```rust
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
```

`fetch_dashboard` 内原有 by-model 请求段保持不变（容错哲学不同：看板要求日志失败仍返回额度），仅 user/self 段替换为 `fetch_user_info` 调用。

- [ ] **Step 9: 静态检查与全量测试**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: 无警告，全部测试通过（含既有 `parses_usage_summary`、`fetch_dashboard` 行为不变）

- [ ] **Step 10: 提交**

```bash
git add src-tauri/src/api.rs
git commit -m "feat: 新增官网用量区间聚合接口"
```

---

### Task 3: lib.rs — 日期校验与两个统计 command

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes：
  - Task 1：`Db::get_daily_snapshots`、`db::derive_daily_requests`、`db::summarize_local`
  - Task 2：`api::fetch_usage_stats(client, cookie, start, end) -> Result<api::UsageStats, String>`
- Produces：
  - IPC command `get_local_stats(start_date, end_date)` → `LocalStats { daily: Vec<LocalDaily>, summary: LocalSummary }`
  - IPC command `fetch_usage_stats(start_date, end_date)` → `api::UsageStats`
  - `fn validate_date_range(&str, &str) -> Result<(NaiveDate, NaiveDate), String>`

- [ ] **Step 1: 写失败测试 — 日期校验**

`lib.rs` 末尾新增测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::validate_date_range;
    use chrono::{Duration, Local};

    fn today() -> chrono::NaiveDate {
        Local::now().date_naive()
    }

    #[test]
    fn valid_range_passes() {
        let end = today();
        let start = end - Duration::days(6);
        let result = validate_date_range(
            &start.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        );
        assert!(result.is_ok());
        let (s, e) = result.expect("合法区间应通过");
        assert_eq!((e - s).num_days(), 6);
    }

    #[test]
    fn start_after_end_rejected() {
        let end = today();
        let start = end + Duration::days(0);
        let error = validate_date_range(
            &(start + Duration::days(1)).format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .expect_err("start 晚于 end 应拒绝");
        assert!(error.contains("不能晚于"));
    }

    #[test]
    fn future_end_rejected() {
        let end = today() + Duration::days(1);
        let error = validate_date_range(
            &today().format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .expect_err("未来日期应拒绝");
        assert!(error.contains("不能超过今天"));
    }

    #[test]
    fn span_over_1096_days_rejected() {
        let end = today();
        let start = end - Duration::days(1097);
        let error = validate_date_range(
            &start.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .expect_err("超 1096 天应拒绝");
        assert!(error.contains("1096"));

        // 恰好 1096 天应通过
        let start_ok = end - Duration::days(1096);
        assert!(validate_date_range(
            &start_ok.format("%Y-%m-%d").to_string(),
            &end.format("%Y-%m-%d").to_string(),
        )
        .is_ok());
    }

    #[test]
    fn invalid_format_rejected() {
        assert!(validate_date_range("2026/08/01", "2026-08-02").is_err());
        assert!(validate_date_range("2026-02-30", "2026-08-02").is_err());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --workspace validate_date_range -- --nocapture`
Expected: FAIL（函数未定义）

- [ ] **Step 3: 实现日期校验**

`lib.rs` 顶部导入补充 `use chrono::{Local, NaiveDate};`，新增：

```rust
/// 校验统计区间：格式、先后、不超今天、跨度上限 1096 天
fn validate_date_range(start_date: &str, end_date: &str) -> Result<(NaiveDate, NaiveDate), String> {
    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .map_err(|_| "起始日期格式无效，应为 YYYY-MM-DD".to_string())?;
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .map_err(|_| "结束日期格式无效，应为 YYYY-MM-DD".to_string())?;
    if start > end {
        return Err("起始日期不能晚于结束日期".into());
    }
    if end > Local::now().date_naive() {
        return Err("结束日期不能超过今天".into());
    }
    if (end - start).num_days() > 1096 {
        return Err("区间跨度不能超过 1096 天".into());
    }
    Ok((start, end))
}
```

- [ ] **Step 4: 运行校验测试确认通过**

Run: `cargo test --workspace validate_date_range -- --nocapture`
Expected: PASS（5 个测试）

- [ ] **Step 5: 实现 command 与响应结构**

`lib.rs` 新增响应 DTO 与两个 command（置于 `get_config` 之后）：

```rust
/// 本地统计每日行（余额 + 降级估算 + 请求数差分）
#[derive(serde::Serialize)]
struct LocalDaily {
    date: String,
    remaining: f64,
    yuan: f64,
    tokens: i64,
    new_requests: Option<i64>,
}

#[derive(serde::Serialize)]
struct LocalSummary {
    total_yuan: f64,
    avg_yuan: f64,
    peak_yuan: f64,
    peak_date: String,
    total_tokens: i64,
    days_with_data: usize,
}

#[derive(serde::Serialize)]
struct LocalStats {
    daily: Vec<LocalDaily>,
    summary: LocalSummary,
}

#[tauri::command]
fn get_local_stats(
    state: tauri::State<AppState>,
    start_date: String,
    end_date: String,
) -> Result<LocalStats, String> {
    validate_date_range(&start_date, &end_date)?;
    let db = state
        .db
        .lock()
        .map_err(|_| "数据库状态锁已损坏".to_string())?;
    let snapshots = db
        .get_daily_snapshots(&start_date, &end_date)
        .map_err(|error| format!("查询本地快照失败: {error}"))?;
    let diffs = db::derive_daily_requests(&snapshots);
    let daily = snapshots
        .iter()
        .zip(diffs)
        .map(|(row, new_requests)| LocalDaily {
            date: row.date.clone(),
            remaining: row.remaining,
            yuan: row.yuan,
            tokens: row.tokens,
            new_requests,
        })
        .collect();
    let (total_yuan, avg_yuan, peak_yuan, peak_date, total_tokens) =
        db::summarize_local(&snapshots);
    Ok(LocalStats {
        daily,
        summary: LocalSummary {
            total_yuan,
            avg_yuan,
            peak_yuan,
            peak_date,
            total_tokens,
            days_with_data: snapshots.len(),
        },
    })
}

#[tauri::command]
async fn fetch_usage_stats(
    state: tauri::State<'_, AppState>,
    start_date: String,
    end_date: String,
) -> Result<api::UsageStats, String> {
    let (start, end) = validate_date_range(&start_date, &end_date)?;
    let cookie = {
        let db = state
            .db
            .lock()
            .map_err(|_| "数据库状态锁已损坏".to_string())?;
        db.get_cookie()
    };
    api::fetch_usage_stats(&state.client, &cookie.unwrap_or_default(), start, end).await
}
```

注册到 handler（`tauri::generate_handler!` 列表追加两项）：

```rust
            get_local_stats,
            fetch_usage_stats,
```

- [ ] **Step 6: 静态检查与全量测试**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: 无警告，全部通过

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: 新增统计 command 与区间校验"
```

---

### Task 4: vendor 依赖引入与统计页 HTML 骨架

**Files:**
- Create: `dist/vendor/chart.umd.js`（Chart.js 4.4.7 UMD，MIT）
- Create: `dist/vendor/xlsx.full.min.js`（SheetJS 0.20.3 UMD，Apache-2.0）
- Modify: `dist/index.html`

**Interfaces:**
- Produces：全局 `Chart`（Chart.js）、`XLSX`（SheetJS）；统计页 DOM 结构（各元素 id 见 HTML，后续 JS 任务按 id 绑定）；dashboard 顶栏 `#stats-btn` 入口。

- [ ] **Step 1: 下载 vendor 库并固定版本**

Run:

```bash
mkdir -p dist/vendor
curl -fL -o dist/vendor/chart.umd.js https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.js
curl -fL -o dist/vendor/xlsx.full.min.js https://cdn.sheetjs.com/xlsx-0.20.3/package/dist/xlsx.full.min.js
```

Expected: 两个文件下载成功，无 curl 错误。

- [ ] **Step 2: 验证依赖完整性**

Run:

```bash
node -e "const c = require('./dist/vendor/chart.umd.js'); console.log('Chart:', typeof c.Chart)"
node -e "const x = require('./dist/vendor/xlsx.full.min.js'); console.log('XLSX:', typeof x.utils)"
```

Expected: 输出 `Chart: function`、`XLSX: object`（任一失败说明下载损坏，重新执行 Step 1）。

- [ ] **Step 3: index.html 引入 vendor 脚本**

将 `<script src="app.js"></script>` 替换为（vendor 必须先于 app.js）：

```html
  <script src="vendor/chart.umd.js"></script>
  <script src="vendor/xlsx.full.min.js"></script>
  <script src="app.js"></script>
```

- [ ] **Step 4: dashboard 顶栏新增统计入口**

在 `#dashboard-screen` 的 `.topbar-actions` 内、`#settings-btn` 之前插入：

```html
        <button id="stats-btn" class="btn icon" title="统计分析" aria-label="统计分析">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 20V10m7 10V4m7 16v-7" /></svg>
        </button>
```

- [ ] **Step 5: 新增 stats-screen 结构**

在 `</section>`（dashboard-screen 结束标签）之后、`<script>` 之前插入：

```html
  <section id="stats-screen" class="screen hidden" aria-labelledby="stats-title" aria-hidden="true">
    <header class="topbar">
      <p id="stats-title" class="product-mark">AMAX / ANALYTICS</p>
      <div class="topbar-actions">
        <button id="export-csv-btn" class="btn tiny" type="button" disabled>CSV</button>
        <button id="export-json-btn" class="btn tiny" type="button" disabled>JSON</button>
        <button id="export-xlsx-btn" class="btn tiny" type="button" disabled>XLSX</button>
        <button id="stats-back-btn" class="btn secondary" type="button">返回看板</button>
      </div>
    </header>

    <main class="content stats-content">
      <div class="range-bar" role="group" aria-label="统计区间">
        <div class="range-presets">
          <button type="button" class="range-preset is-active" data-days="7">7天</button>
          <button type="button" class="range-preset" data-days="14">14天</button>
          <button type="button" class="range-preset" data-days="30">30天</button>
        </div>
        <div class="range-dates">
          <input type="date" id="range-start" aria-label="起始日期" />
          <span class="range-sep">~</span>
          <input type="date" id="range-end" aria-label="结束日期" />
        </div>
      </div>
      <p id="range-error" class="range-error hidden" role="alert"></p>

      <section class="stats-summary" aria-label="区间汇总">
        <h2 id="summary-title" class="telemetry-label">RANGE SUMMARY</h2>
        <div class="summary-grid">
          <div class="summary-card"><p class="telemetry-label">累计费用</p><p id="sum-total-yuan" class="summary-value">--</p></div>
          <div class="summary-card"><p class="telemetry-label">日均费用</p><p id="sum-avg-yuan" class="summary-value">--</p></div>
          <div class="summary-card"><p class="telemetry-label">单日峰值</p><p id="sum-peak" class="summary-value">--</p></div>
          <div class="summary-card"><p class="telemetry-label">有使用天数</p><p id="sum-days" class="summary-value">--</p></div>
          <div class="summary-card"><p class="telemetry-label">区间总请求数</p><p id="sum-requests" class="summary-value">--</p></div>
        </div>
      </section>

      <section class="stats-block" aria-label="消耗趋势">
        <div class="block-head">
          <h2 class="telemetry-label">COST / TOKENS TREND</h2>
          <p id="trend-note" class="metric-note"></p>
        </div>
        <p id="trend-error" class="block-error hidden" role="alert"></p>
        <div class="chart-wrap"><canvas id="trend-chart" aria-label="消耗趋势图" role="img"></canvas></div>
      </section>

      <section class="stats-block" aria-label="请求数趋势">
        <h2 class="telemetry-label">DAILY REQUESTS</h2>
        <div class="chart-wrap chart-wrap-sm"><canvas id="requests-chart" aria-label="每日请求数趋势图" role="img"></canvas></div>
      </section>

      <section class="stats-block" aria-label="模型分布">
        <div class="block-head">
          <h2 class="telemetry-label">MODEL DISTRIBUTION</h2>
          <p id="model-basis-note" class="metric-note"></p>
        </div>
        <p id="model-error" class="block-error hidden" role="alert"></p>
        <div class="model-layout">
          <div class="chart-wrap chart-wrap-sm"><canvas id="model-chart" aria-label="模型分布环图" role="img"></canvas></div>
          <ul id="model-list" class="model-list"></ul>
        </div>
      </section>

      <section id="remaining-block" class="stats-block" aria-label="余额趋势">
        <h2 class="telemetry-label">BALANCE TREND</h2>
        <div class="chart-wrap chart-wrap-sm"><canvas id="remaining-chart" aria-label="余额趋势图" role="img"></canvas></div>
      </section>
    </main>
  </section>
```

- [ ] **Step 6: 验证 HTML 引用与提交**

Run:

```bash
node --check dist/app.js
grep -c "vendor/chart.umd.js\|vendor/xlsx.full.min.js\|stats-screen" dist/index.html
```

Expected: app.js 语法通过；grep 输出 ≥ 3。

```bash
git add dist/vendor/chart.umd.js dist/vendor/xlsx.full.min.js dist/index.html
git commit -m "feat: 引入图表依赖与统计页骨架"
```

---

### Task 5: 统计页样式

**Files:**
- Modify: `dist/style.css`（末尾追加统计页段落）

**Interfaces:**
- Consumes：Task 4 的 HTML 结构与既有设计变量（`--accent` / `--line` / `--surface-raised` / `--mono` 等）。
- Produces：`.range-bar` / `.summary-grid` / `.stats-block` / `.chart-wrap` / `.model-layout` / `.btn.tiny` 等类的完整样式。

- [ ] **Step 1: 追加统计页样式**

在 `style.css` 的 `@media (prefers-reduced-motion)` 段之前追加：

```css
/* ═══ 统计页 ═══ */
.stats-content { max-width: 452px; margin: 0 auto; }
.btn.tiny { min-height: 26px; padding: 0 8px; border: 1px solid var(--line); background: transparent; color: var(--muted); font: 700 10px var(--mono); letter-spacing: .08em; }
.btn.tiny:hover:not(:disabled) { border-color: #91a298; background: var(--surface-pressed); color: var(--ink); }

.range-bar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 20px; }
.range-presets { display: flex; gap: 4px; }
.range-preset { min-height: 28px; padding: 0 10px; border: 1px solid var(--line); border-radius: 4px; background: transparent; color: var(--muted); font: 600 11px var(--sans); cursor: pointer; transition: background-color .16s, color .16s, border-color .16s; }
.range-preset:hover { border-color: #91a298; background: var(--surface-pressed); color: var(--ink); }
.range-preset.is-active { border-color: var(--accent); background: var(--accent-soft); color: var(--accent); }
.range-preset:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.range-dates { display: flex; align-items: center; gap: 6px; }
.range-dates input { min-height: 28px; border: 1px solid var(--line); border-radius: 4px; background: rgba(242, 244, 241, .88); color: var(--ink); padding: 2px 6px; font: 11px var(--mono); }
.range-dates input:focus-visible { border-color: var(--accent); outline: 0; box-shadow: 0 0 0 3px rgba(44, 107, 75, .16); }
.range-sep { color: var(--muted); font: 11px var(--mono); }
.range-error { margin: -12px 0 16px; border-left: 2px solid var(--error); background: var(--error-bg); color: var(--error); padding: 7px 10px; font-size: 12px; }

.stats-summary { margin-bottom: 26px; }
.summary-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(128px, 1fr)); gap: 8px; margin-top: 12px; }
.summary-card { border: 1px solid var(--line); border-radius: 4px; background: var(--surface-raised); padding: 10px 12px; }
.summary-card .telemetry-label { font-size: 9px; }
.summary-value { margin: 6px 0 0; font: 600 16px var(--mono); font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }

.stats-block { margin-bottom: 26px; border-top: 1px solid var(--line); padding-top: 18px; }
.block-head { display: flex; justify-content: space-between; align-items: baseline; gap: 12px; }
.block-head .metric-note { margin: 0; text-align: right; }
.block-error { margin: 10px 0 0; border-left: 2px solid var(--error); background: var(--error-bg); color: var(--error); padding: 7px 10px; font-size: 12px; line-height: 1.5; }
.chart-wrap { position: relative; height: 210px; margin-top: 14px; }
.chart-wrap-sm { height: 150px; }

.model-layout { display: grid; grid-template-columns: 150px 1fr; gap: 14px; align-items: center; margin-top: 14px; }
.model-list { margin: 0; padding: 0; list-style: none; display: grid; gap: 8px; min-width: 0; }
.model-list li { display: flex; justify-content: space-between; gap: 10px; font: 11px var(--mono); min-width: 0; }
.model-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.model-nums { color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
```

- [ ] **Step 2: 提交**

```bash
git add dist/style.css
git commit -m "feat: 新增统计页样式"
```

---

### Task 6: app.js — 三屏切换、区间联动与官方数据渲染

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes：Task 3 的 `fetch_usage_stats` / `get_local_stats` command；Task 4 的 DOM id；全局 `Chart`。
- Produces：`showScreen` 三屏版、`loadStats()`、`statsUsage` / `statsLocal` / `statsSource` / `statsRange` 模块状态、`renderSummary` / `renderTrendChart` / `renderModelBlock`。Task 7/8 依赖这些状态与函数名；本任务末尾提供 `renderRequestsChart` / `renderRemainingChart` / `updateExportButtons` 空实现占位（Task 7/8 替换）。

- [ ] **Step 1: 重写 showScreen 为三屏互斥**

替换现有 `showScreen` 实现（位于工具段，在所有 DOM 常量之后，保留函数名与调用方式）：

```js
const screenRegistry = { config: configScreen, dashboard: dashboardScreen, stats: statsScreen };
function showScreen(screen) {
  for (const [name, el] of Object.entries(screenRegistry)) {
    const hidden = name !== screen;
    el.classList.toggle('hidden', hidden);
    el.setAttribute('aria-hidden', String(hidden));
  }
}
```

注意：`screenRegistry` 为顶层立即执行代码，引用了 `statsScreen`（Step 2 新增的 DOM 常量）。`const` 存在暂时性死区，因此 **Step 2 的 DOM 常量必须先写入文件并位于本段之前**（现有 `showScreen` 在工具段，天然位于 DOM 段之后，原位替换即可满足顺序）。实施顺序：先做 Step 2 的 DOM 常量追加，再做本步替换。

- [ ] **Step 2: DOM 引用、状态与工具函数**

在文件 `// ═══ DOM ═══` 段既有常量之后追加：

```js
const statsScreen = $('#stats-screen');
const statsBtn = $('#stats-btn');
const statsBackBtn = $('#stats-back-btn');
const rangeStartInput = $('#range-start');
const rangeEndInput = $('#range-end');
const rangeErrorEl = $('#range-error');
const summaryTitleEl = $('#summary-title');
const trendNoteEl = $('#trend-note');
const trendErrorEl = $('#trend-error');
const modelErrorEl = $('#model-error');
const modelBasisNoteEl = $('#model-basis-note');
const modelListEl = $('#model-list');
const remainingBlock = $('#remaining-block');
const exportCsvBtn = $('#export-csv-btn');
const exportJsonBtn = $('#export-json-btn');
const exportXlsxBtn = $('#export-xlsx-btn');
```

在 `// ═══ 工具 ═══` 段追加：

```js
function toDateStr(date) {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}
function todayStr() { return toDateStr(new Date()); }
function daysAgoStr(days) {
  const date = new Date();
  date.setDate(date.getDate() - (days - 1)); // 含当日，7 天即今天往前 6 天
  return toDateStr(date);
}
function isAuthErrorMessage(msg) { return msg.includes('认证失败'); }
```

在文件末尾（`init()` 调用之前）新增统计页段落：

```js
// ═══ 统计页 ═══
const statsCharts = { trend: null, requests: null, model: null, remaining: null };
let statsUsage = null;   // fetch_usage_stats 结果
let statsLocal = null;   // get_local_stats 结果
let statsSource = 'official'; // 'official' | 'local_estimate'
let statsRange = null;   // { start_date, end_date }

function chartTheme() {
  return {
    font: { family: '"Cascadia Mono", "SFMono-Regular", Consolas, monospace', size: 10 },
    muted: '#5a6460',
    grid: 'rgba(190, 201, 193, .5)',
    accent: '#2c6b4b',
  };
}

function destroyStatsCharts() {
  for (const key of Object.keys(statsCharts)) {
    if (statsCharts[key]) { statsCharts[key].destroy(); statsCharts[key] = null; }
  }
}

function showRangeError(msg) { rangeErrorEl.textContent = msg; rangeErrorEl.classList.remove('hidden'); }
function hideRangeError() { rangeErrorEl.classList.add('hidden'); }
function showBlockError(el, msg) { el.textContent = msg; el.classList.remove('hidden'); }
function hideBlockError(el) { el.classList.add('hidden'); }
```

- [ ] **Step 3: 入口与区间联动**

紧接 Step 2 段落追加：

```js
function setActivePreset(daysOrNull) {
  document.querySelectorAll('.range-preset').forEach((btn) => {
    btn.classList.toggle('is-active', daysOrNull !== null && Number(btn.dataset.days) === daysOrNull);
  });
}

function syncRangeInputs() {
  const today = todayStr();
  rangeStartInput.max = today;
  rangeEndInput.max = today;
  rangeStartInput.value = statsRange.start_date;
  rangeEndInput.value = statsRange.end_date;
}

async function enterStats() {
  if (!statsRange) {
    statsRange = { start_date: daysAgoStr(7), end_date: todayStr() };
    setActivePreset(7);
    syncRangeInputs();
  }
  showScreen('stats');
  await loadStats();
}

function applyPreset(days) {
  statsRange = { start_date: daysAgoStr(days), end_date: todayStr() };
  setActivePreset(days);
  syncRangeInputs();
  hideRangeError();
  loadStats();
}

function applyCustomRange() {
  const start = rangeStartInput.value;
  const end = rangeEndInput.value;
  if (!start || !end) return;
  setActivePreset(null);
  if (start > end) { showRangeError('起始日期不能晚于结束日期'); return; }
  if (end > todayStr()) { showRangeError('结束日期不能超过今天'); return; }
  statsRange = { start_date: start, end_date: end };
  hideRangeError();
  loadStats();
}

statsBtn.addEventListener('click', enterStats);
statsBackBtn.addEventListener('click', () => { showScreen('dashboard'); statsBtn.focus(); });
document.querySelectorAll('.range-preset').forEach((btn) =>
  btn.addEventListener('click', () => applyPreset(Number(btn.dataset.days))));
rangeStartInput.addEventListener('change', applyCustomRange);
rangeEndInput.addEventListener('change', applyCustomRange);
```

- [ ] **Step 4: loadStats 与汇总渲染**

```js
async function loadStats() {
  const [usageResult, localResult] = await Promise.allSettled([
    invoke('fetch_usage_stats', statsRange),
    invoke('get_local_stats', statsRange),
  ]);

  statsLocal = localResult.status === 'fulfilled' ? localResult.value : null;

  if (usageResult.status === 'fulfilled') {
    statsUsage = usageResult.value;
    statsSource = 'official';
    hideBlockError(trendErrorEl);
    hideBlockError(modelErrorEl);
    renderSummary(statsUsage.summary, 'official');
    renderModelBlock(statsUsage);
  } else {
    statsUsage = null;
    statsSource = 'local_estimate';
    const msg = getErrorMessage(usageResult.reason);
    const authHint = isAuthErrorMessage(msg) ? '，可前往设置页重新获取 Cookie' : '';
    showBlockError(trendErrorEl, '官方数据获取失败：' + msg + authHint + '。趋势与汇总已切换为本地估算。');
    showBlockError(modelErrorEl, '官方数据不可用：' + msg + authHint);
    renderSummary(statsLocal ? statsLocal.summary : null, 'local_estimate');
    renderModelBlock(null);
  }

  renderTrendChart();
  renderRequestsChart();
  renderRemainingChart();
  updateExportButtons();
}

function renderSummary(summary, source) {
  summaryTitleEl.textContent = source === 'local_estimate'
    ? 'RANGE SUMMARY（本地估算）'
    : 'RANGE SUMMARY';
  const set = (id, text) => { $(id).textContent = text; };
  if (!summary) {
    ['#sum-total-yuan', '#sum-avg-yuan', '#sum-peak', '#sum-days', '#sum-requests']
      .forEach((id) => set(id, '--'));
    return;
  }
  set('#sum-total-yuan', '¥' + summary.total_yuan.toFixed(6));
  set('#sum-avg-yuan', '¥' + summary.avg_yuan.toFixed(6));
  set('#sum-peak', summary.peak_date
    ? '¥' + summary.peak_yuan.toFixed(6) + '（' + summary.peak_date + '）'
    : '--');
  const days = source === 'official' ? summary.days_with_usage : summary.days_with_data;
  set('#sum-days', String(days));
  set('#sum-requests', summary.request_count != null ? String(summary.request_count) : '--');
}
```

- [ ] **Step 5: 消耗趋势图（含本地对比数据集与降级）**

```js
function renderTrendChart() {
  const theme = chartTheme();
  if (statsCharts.trend) { statsCharts.trend.destroy(); statsCharts.trend = null; }

  let labels = [];
  let yuanData = [];
  let tokensData = [];
  let dashed = false;

  if (statsUsage) {
    labels = statsUsage.daily.map((d) => d.date);
    yuanData = statsUsage.daily.map((d) => d.yuan);
    tokensData = statsUsage.daily.map((d) => d.tokens);
    trendNoteEl.textContent = '';
  } else if (statsLocal && statsLocal.daily.length) {
    labels = statsLocal.daily.map((d) => d.date);
    yuanData = statsLocal.daily.map((d) => d.yuan);
    tokensData = statsLocal.daily.map((d) => d.tokens);
    dashed = true;
    trendNoteEl.textContent = '本地估算（快照日末累计值）';
  } else {
    trendNoteEl.textContent = '暂无数据';
  }

  const datasets = [
    { label: '费用 ¥', data: yuanData, yAxisID: 'yYuan', borderColor: theme.accent, borderDash: dashed ? [5, 4] : [], tension: .25, pointRadius: 2, fill: false },
    { label: 'Tokens', data: tokensData, yAxisID: 'yTokens', borderColor: '#7a8a80', borderDash: dashed ? [5, 4] : [], tension: .25, pointRadius: 2, fill: false },
  ];

  // 官方模式下叠加本地对比数据集（默认隐藏，图例点击开启）
  if (statsUsage && statsLocal && statsLocal.daily.length) {
    const localByDate = new Map(statsLocal.daily.map((d) => [d.date, d]));
    datasets.push({
      label: '本地费用 ¥', data: labels.map((date) => localByDate.get(date)?.yuan ?? null),
      yAxisID: 'yYuan', borderColor: 'rgba(44, 107, 75, .55)', borderDash: [3, 3],
      tension: .25, pointRadius: 1, hidden: true, spanGaps: true,
    });
    datasets.push({
      label: '本地 Tokens', data: labels.map((date) => localByDate.get(date)?.tokens ?? null),
      yAxisID: 'yTokens', borderColor: 'rgba(122, 138, 128, .55)', borderDash: [3, 3],
      tension: .25, pointRadius: 1, hidden: true, spanGaps: true,
    });
  }

  statsCharts.trend = new Chart($('#trend-chart'), {
    type: 'line',
    data: { labels, datasets },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      interaction: { mode: 'index', intersect: false },
      scales: {
        x: { ticks: { color: theme.muted, font: theme.font, maxTicksLimit: 8, maxRotation: 0 }, grid: { color: theme.grid } },
        yYuan: { position: 'left', ticks: { color: theme.muted, font: theme.font }, grid: { color: theme.grid }, title: { display: true, text: '¥', color: theme.muted, font: theme.font } },
        yTokens: { position: 'right', ticks: { color: theme.muted, font: theme.font }, grid: { drawOnChartArea: false }, title: { display: true, text: 'Tokens', color: theme.muted, font: theme.font } },
      },
      plugins: { legend: { labels: { color: theme.muted, font: theme.font, boxWidth: 14 } } },
    },
  });
}
```

- [ ] **Step 6: 模型分布环图与列表**

```js
const MODEL_PALETTE = ['#2c6b4b', '#7a8a80', '#b08d57', '#5a6460', '#a3544f', '#4f6d7a', '#8b3e3e', '#6b7d5e'];

function renderModelBlock(usage) {
  if (statsCharts.model) { statsCharts.model.destroy(); statsCharts.model = null; }
  modelListEl.innerHTML = '';

  if (!usage || !usage.models.length) {
    modelBasisNoteEl.textContent = usage ? '暂无模型数据' : '';
    return;
  }

  modelBasisNoteEl.textContent = usage.percent_basis === 'tokens'
    ? '占比口径：Tokens（quota 全为 0）'
    : '占比口径：quota';

  const basisValue = (m) => (usage.percent_basis === 'tokens' ? m.tokens : m.yuan);

  statsCharts.model = new Chart($('#model-chart'), {
    type: 'doughnut',
    data: {
      labels: usage.models.map((m) => m.model),
      datasets: [{
        data: usage.models.map(basisValue),
        backgroundColor: usage.models.map((_, i) => MODEL_PALETTE[i % MODEL_PALETTE.length]),
        borderColor: '#e8ece8',
        borderWidth: 1,
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      cutout: '62%',
      plugins: {
        legend: { display: false },
        tooltip: {
          callbacks: {
            label: (item) => {
              const m = usage.models[item.dataIndex];
              return `${m.model}: ${m.percent.toFixed(1)}%`;
            },
          },
        },
      },
    },
  });

  usage.models.forEach((m) => {
    const li = document.createElement('li');
    const name = document.createElement('span');
    name.className = 'model-name';
    name.textContent = m.model;
    name.title = m.model;
    const nums = document.createElement('span');
    nums.className = 'model-nums';
    nums.textContent = '¥' + m.yuan.toFixed(4) + ' / ' + m.percent.toFixed(1) + '% / ' + m.request_count + ' 次';
    li.append(name, nums);
    modelListEl.appendChild(li);
  });
}
```

- [ ] **Step 7: Task 7/8 占位函数**

```js
// 占位实现 — Task 7（本地数据渲染）与 Task 8（导出）替换
function renderRequestsChart() {}
function renderRemainingChart() {}
function updateExportButtons() {}
```

- [ ] **Step 8: 语法检查与提交**

Run: `node --check dist/app.js`
Expected: 通过，无输出

```bash
git add dist/app.js
git commit -m "feat: 统计页官方数据渲染与区间联动"
```

---

### Task 7: app.js — 本地数据渲染（请求数、余额）与图表回收

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes：Task 6 的 `statsLocal` / `statsCharts` / `chartTheme` / `destroyStatsCharts` / `remainingBlock`。
- Produces：`renderRequestsChart` / `renderRemainingChart` 正式实现（替换 Task 6 占位）；返回看板时销毁全部图表实例。

- [ ] **Step 1: 替换 renderRequestsChart 占位**

将占位 `function renderRequestsChart() {}` 替换为：

```js
function renderRequestsChart() {
  const theme = chartTheme();
  if (statsCharts.requests) { statsCharts.requests.destroy(); statsCharts.requests = null; }

  const rows = (statsLocal && statsLocal.daily) || [];
  // 无任何有效差分（全为 null，如全新安装或老数据）时整块隐藏
  const block = $('#requests-chart').closest('.stats-block');
  if (!rows.some((d) => d.new_requests != null)) {
    block.classList.add('hidden');
    return;
  }
  block.classList.remove('hidden');

  statsCharts.requests = new Chart($('#requests-chart'), {
    type: 'line',
    data: {
      labels: rows.map((d) => d.date),
      datasets: [{
        label: '新增请求数',
        data: rows.map((d) => d.new_requests), // null 点自动留空
        borderColor: theme.accent,
        tension: .25,
        pointRadius: 2,
        fill: false,
        spanGaps: false, // 缺档日断开，避免误导
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      scales: {
        x: { ticks: { color: theme.muted, font: theme.font, maxTicksLimit: 8, maxRotation: 0 }, grid: { color: theme.grid } },
        y: { beginAtZero: true, ticks: { color: theme.muted, font: theme.font, precision: 0 }, grid: { color: theme.grid } },
      },
      plugins: { legend: { display: false } },
    },
  });
}
```

- [ ] **Step 2: 替换 renderRemainingChart 占位**

```js
function renderRemainingChart() {
  const theme = chartTheme();
  if (statsCharts.remaining) { statsCharts.remaining.destroy(); statsCharts.remaining = null; }

  const rows = (statsLocal && statsLocal.daily) || [];
  if (!rows.length) {
    remainingBlock.classList.add('hidden');
    return;
  }
  remainingBlock.classList.remove('hidden');

  statsCharts.remaining = new Chart($('#remaining-chart'), {
    type: 'line',
    data: {
      labels: rows.map((d) => d.date),
      datasets: [{
        label: '剩余额度 ¥',
        data: rows.map((d) => d.remaining),
        borderColor: '#b08d57',
        tension: .25,
        pointRadius: 2,
        fill: false,
        spanGaps: true, // 余额为状态量，缺口连接
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      scales: {
        x: { ticks: { color: theme.muted, font: theme.font, maxTicksLimit: 8, maxRotation: 0 }, grid: { color: theme.grid } },
        y: { ticks: { color: theme.muted, font: theme.font }, grid: { color: theme.grid } },
      },
      plugins: { legend: { display: false } },
    },
  });
}
```

- [ ] **Step 3: 离开统计页时销毁图表**

将 Task 6 的返回按钮绑定替换为：

```js
statsBackBtn.addEventListener('click', () => {
  destroyStatsCharts();
  showScreen('dashboard');
  statsBtn.focus();
});
```

- [ ] **Step 4: 语法检查与提交**

Run: `node --check dist/app.js`
Expected: 通过

```bash
git add dist/app.js
git commit -m "feat: 统计页本地趋势与图表回收"
```

---

### Task 8: app.js — CSV / JSON / XLSX 导出

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes：Task 6/7 的 `statsUsage` / `statsLocal` / `statsSource` / `statsRange`；全局 `XLSX`。
- Produces：`updateExportButtons` 正式实现（替换占位）、三个导出按钮的事件绑定、`buildExportData` / `buildCsv` / `downloadBlob`。

- [ ] **Step 1: 替换 updateExportButtons 占位并新增导出数据构建**

```js
function updateExportButtons() {
  const hasData = Boolean(statsUsage || (statsLocal && statsLocal.daily.length));
  [exportCsvBtn, exportJsonBtn, exportXlsxBtn].forEach((btn) => { btn.disabled = !hasData; });
}

// 汇总官方与本地数据为统一导出结构
function buildExportData() {
  const official = statsSource === 'official' && statsUsage;
  const localByDate = new Map((statsLocal && statsLocal.daily || []).map((d) => [d.date, d]));
  const baseDaily = official ? statsUsage.daily : (statsLocal && statsLocal.daily || []);
  return {
    app: 'amax-dashboard',
    exported_at: new Date().toISOString(),
    range: statsRange,
    source: statsSource,
    percent_basis: official ? statsUsage.percent_basis : null,
    summary: official ? statsUsage.summary : (statsLocal ? statsLocal.summary : null),
    daily: baseDaily.map((d) => ({
      date: d.date,
      yuan: d.yuan,
      tokens: d.tokens,
      input_tokens: official ? d.input_tokens : null,
      output_tokens: official ? d.output_tokens : null,
      new_requests: localByDate.has(d.date) ? localByDate.get(d.date).new_requests : null,
      remaining: localByDate.has(d.date) ? localByDate.get(d.date).remaining : null,
    })),
    models: official ? statsUsage.models : [],
  };
}

function exportFileName(ext) {
  return `amax-stats_${statsRange.start_date}_to_${statsRange.end_date}.${ext}`;
}

function downloadBlob(blob, filename) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
```

- [ ] **Step 2: CSV 生成与绑定**

```js
function csvEscape(value) {
  const text = value == null ? '' : String(value);
  return /[",\n\r]/.test(text) ? '"' + text.replace(/"/g, '""') + '"' : text;
}

function buildCsv(data) {
  const lines = [];
  lines.push('[区间汇总]');
  lines.push('指标,值');
  if (data.summary) {
    const s = data.summary;
    lines.push(`累计费用(元),${csvEscape(s.total_yuan)}`);
    lines.push(`日均费用(元),${csvEscape(s.avg_yuan)}`);
    lines.push(`单日峰值(元),${csvEscape(s.peak_yuan)}`);
    lines.push(`峰值日期,${csvEscape(s.peak_date)}`);
    lines.push(`累计Tokens,${csvEscape(s.total_tokens)}`);
    if (s.request_count != null) lines.push(`区间总请求数,${csvEscape(s.request_count)}`);
  }
  lines.push(`数据来源,${data.source}`);
  lines.push('');
  lines.push('[每日趋势]');
  lines.push('date,yuan,tokens,input_tokens,output_tokens,new_requests,remaining');
  data.daily.forEach((d) => lines.push(
    [d.date, d.yuan, d.tokens, d.input_tokens, d.output_tokens, d.new_requests, d.remaining]
      .map(csvEscape).join(',')));
  lines.push('');
  lines.push('[模型分布]');
  lines.push('model,yuan,tokens,request_count,percent,percent_basis');
  data.models.forEach((m) => lines.push(
    [m.model, m.yuan, m.tokens, m.request_count, m.percent, data.percent_basis]
      .map(csvEscape).join(',')));
  // BOM（U+FEFF）防 Excel 中文乱码；用 fromCharCode 避免源码中不可见字符
  return String.fromCharCode(0xFEFF) + lines.join('\r\n');
}

exportCsvBtn.addEventListener('click', () => {
  const data = buildExportData();
  downloadBlob(new Blob([buildCsv(data)], { type: 'text/csv;charset=utf-8' }), exportFileName('csv'));
});
```

- [ ] **Step 3: JSON 导出绑定**

```js
exportJsonBtn.addEventListener('click', () => {
  const data = buildExportData();
  downloadBlob(
    new Blob([JSON.stringify(data, null, 2)], { type: 'application/json;charset=utf-8' }),
    exportFileName('json'));
});
```

- [ ] **Step 4: XLSX 导出绑定**

```js
exportXlsxBtn.addEventListener('click', () => {
  const data = buildExportData();
  const wb = XLSX.utils.book_new();

  const summaryRows = [['指标', '值']];
  if (data.summary) {
    const s = data.summary;
    summaryRows.push(['累计费用(元)', s.total_yuan], ['日均费用(元)', s.avg_yuan],
      ['单日峰值(元)', s.peak_yuan], ['峰值日期', s.peak_date], ['累计Tokens', s.total_tokens]);
    if (s.request_count != null) summaryRows.push(['区间总请求数', s.request_count]);
  }
  summaryRows.push(['数据来源', data.source], ['区间起', data.range.start_date], ['区间止', data.range.end_date]);
  XLSX.utils.book_append_sheet(wb, XLSX.utils.aoa_to_sheet(summaryRows), '区间汇总');

  XLSX.utils.book_append_sheet(wb, XLSX.utils.json_to_sheet(
    data.daily.length ? data.daily : [{ date: '', yuan: '', tokens: '' }]), '每日趋势');

  XLSX.utils.book_append_sheet(wb, XLSX.utils.json_to_sheet(
    data.models.length ? data.models : [{ model: '', yuan: '', tokens: '', request_count: '', percent: '' }]), '模型分布');

  XLSX.writeFile(wb, exportFileName('xlsx'));
});
```

- [ ] **Step 5: 语法检查与提交**

Run: `node --check dist/app.js`
Expected: 通过

```bash
git add dist/app.js
git commit -m "feat: 统计页三格式数据导出"
```

---

### Task 9: 全量静态检查与 GUI 真实验证

**Files:**
- 无代码变更（验证任务；发现的缺陷就地修复并单独提交）

**Interfaces:**
- Consumes：Task 1–8 全部产出。

- [ ] **Step 1: 全量静态检查**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node --check dist/app.js
```

Expected: 全部通过。

- [ ] **Step 2: 启动真实应用**

前置：确认无 `amax.exe` 在运行（若有，请人工关闭，不要终止未知进程）。

Run: `cargo tauri dev`（后台运行，保持窗口可见）

- [ ] **Step 3: GUI 验证清单（逐项人工确认）**

看板与入口：

- [ ] 启动后进入看板（已有 Cookie 场景）
- [ ] 顶栏新增图表按钮，点击进入统计页
- [ ] 统计页"返回看板"按钮正常返回，焦点落回图表按钮

区间与联动：

- [ ] 默认 7 天区间，7天预设按钮高亮
- [ ] 切换 14 / 30 天，日期输入框同步更新，数据重新加载
- [ ] 手动修改日期输入：预设高亮清除；起 > 止显示就地错误且不发起请求
- [ ] 自定义长区间（如 180 天）查询成功

官方数据块：

- [ ] 汇总卡 5 项数值渲染（quota 为 0 时费用显示 ¥0.000000）
- [ ] 消耗趋势图双 Y 轴、双数据集，图例可开关
- [ ] 图例显示"本地费用 / 本地 Tokens"两个隐藏数据集，点击可叠加虚线对比
- [ ] 模型分布环图 + 列表（名称 / 金额 / 占比 / 请求数），标注"占比口径：Tokens（quota 全为 0）"
- [ ] 无使用量的日期趋势线连续（补 0）

本地数据块：

- [ ] 余额趋势图渲染（有快照数据时）
- [ ] 请求数趋势：有差分数据的日期显示，缺档日断开
- [ ] 全新无快照场景（可用临时数据目录验证）：请求数块与余额块整块隐藏

降级路径：

- [ ] 断网（或临时改错 BASE_URL 后复原）刷新：趋势/汇总切换本地估算并标注，模型块显示错误与 Cookie 提示，余额/请求数块不受影响
- [ ] 恢复网络后重新进入统计页，官方数据恢复

导出：

- [ ] 无任何数据时三个导出按钮禁用
- [ ] CSV 导出：Excel/WPS 打开三节内容完整、中文无乱码
- [ ] JSON 导出：结构含 app / exported_at / range / source / summary / daily / models / percent_basis
- [ ] XLSX 导出：三个工作表名称与内容正确

- [ ] **Step 4: 缺陷修复（如有）**

按缺陷归属任务的范围修复，每个缺陷独立提交：

```bash
git add <具体文件>
git commit -m "fix: <缺陷描述>"
```

---

### Task 10: CLAUDE.md 文档同步

**Files:**
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes：已落地的最终实现。

- [ ] **Step 1: 更新 api.rs 段落**

在 api.rs 段落的 by-model 说明行后追加：

```markdown
- by-model 响应除 `summary` 外还含 `models`（模型名、`request_count`、Token 明细、`quota`）与 `daily`（日 × 模型明细，`date` 为本地时区 0 点 Unix 时间戳，无使用量的日期无记录）；`fetch_usage_stats` 按日聚合补零、派生区间汇总，`quota` 全 0 时模型占比自动切换 `total_tokens` 口径（`percent_basis`）。
```

- [ ] **Step 2: 更新 db.rs 段落**

将：

```markdown
`src-tauri/src/db.rs` 管理应用数据目录中的 SQLite：`config` 保存认证信息，`dashboard_snapshot` 保存刷新快照并清理 90 天前记录。Cookie 保存后不设本地过期时间，实际失效由服务端判定（API 返回认证错误时前端回退配置页）。
```

替换为：

```markdown
`src-tauri/src/db.rs` 管理应用数据目录中的 SQLite：`config` 保存认证信息，`dashboard_snapshot` 保存刷新快照（含 `request_count` 累计值，`Db::open` 对旧库幂等 `ALTER TABLE ADD COLUMN` 迁移），快照永久保留、不清理。Cookie 保存后不设本地过期时间，实际失效由服务端判定（API 返回认证错误时前端回退配置页）。
```

- [ ] **Step 3: 更新前后端边界段落**

在 `fetch_dashboard` command 说明行后追加：

```markdown
- `get_local_stats`：按起止日期查询每日快照（日末条），返回余额、本地估算值与每日新增请求数（累计差分）。
- `fetch_usage_stats`：按起止日期调用官网聚合，返回补零后的每日消耗、模型分布与区间汇总。
```

并将"新增或重命名 command/event 时，需要同步修改 `src-tauri/src/lib.rs` 和 `dist/app.js`"一句保留不动。

- [ ] **Step 4: 更新数据与界面流程段落**

在该段落末尾追加：

```markdown
统计页经看板顶栏图表按钮进入：区间选择器（预设 7/14/30 天 + 自定义起止日期，跨度上限 1096 天）驱动 `fetch_usage_stats`（官方主源）与 `get_local_stats`（余额、请求数、对比与降级数据）并行调用；官方失败时趋势与汇总回退本地估算并标注，模型分布仅官方可用；消耗趋势图可叠加本地对比数据集（默认隐藏）；导出按钮将当前区间数据输出为 CSV / JSON / XLSX（Chart.js 与 SheetJS 以 UMD 单文件存放于 `dist/vendor/`）。
```

- [ ] **Step 5: 提交**

```bash
git add CLAUDE.md
git commit -m "docs: 同步统计功能架构说明"
```

---

## 交付定义

- 10 个任务全部完成，`cargo fmt --check` / `clippy -D warnings` / `cargo test` / `node --check` 均通过。
- GUI 验证清单全部勾选。
- 11 次左右独立提交（10 任务 + 可能的缺陷修复），提交信息符合中文祈使句规范。
