# AMAX Dashboard

`ai.amaxsmp.com` 账户用量看板的**双端原生实现**：Windows 桌面端（Tauri 2 + 无框架前端）与 HarmonyOS 端（ArkTS / ArkUI，Stage 模型）。两端共享同一套数据口径与官网接口约定，不做云同步。

- 桌面端仓库根即项目根：Rust 后端 `src-tauri/`，静态前端 `dist/`（无 Node.js 构建步骤）。
- HarmonyOS 端在 `harmony/`，开发命令与结构见 [`harmony/README.md`](harmony/README.md)。

## 功能

| 能力 | 说明 |
|---|---|
| 当日看板 | 当日费用（元）、当日 Token（含输入/输出拆分）、剩余额度与百分比进度条 |
| 统计分析 | 7 / 14 / 30 天预设与自定义起止区间（跨度上限 1096 天）：消耗趋势双轴、每日新增请求数、模型分布占比、区间汇总（累计/日均/单日峰值/有使用天数）、余额趋势 |
| 降级显示 | 官网聚合接口失败时趋势与汇总回退本地快照估算并标注；模型分布仅官方数据可得时显示 |
| 数据导出 | 桌面端 CSV / JSON / XLSX；手机端 CSV / JSON（字段口径与桌面端一致） |
| 官网登录取凭据 | 应用内 WebView 打开官网登录页，自动提取整串 Session Cookie（含 HTTP-only），手动粘贴保留为兜底 |
| 托盘与后台刷新 | 关闭/最小化即隐藏到托盘；每 10 分钟后台刷新；托盘菜单可刷新/恢复窗口/退出 |
| 低余额通知 | 剩余额度低于 10% 时发一次系统通知（后台/托盘刷新触发），恢复到阈值以上后重置 |
| 失效引导 | 凭据失效时回到配置页并直接指向登录入口；桌面端后台刷新撞上失效会主动广播引导重登 |

## 快速开始（桌面端）

环境：Windows 10/11、Rust stable（crate 用 edition 2024，需 1.85+；本地实测 1.98.1）、`tauri-cli` 2.x（本地实测 2.11.5）、WebView2。前端测试需要 Node.js（仅用于跑 `node --test`，不参与构建）。

```bash
cargo tauri dev            # 开发运行，Rust 改动自动重编译
cargo tauri build          # 产出 Windows MSI（WiX 含 zh-CN / en-US 两种语言）
```

```bash
# 静态检查与测试
cargo check --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                      # Rust 单元测试
node --check dist/app.js
node tests/frontend-navigation.test.js      # 前端逻辑回归（node:test + vm 沙箱）
```

> `cargo clippy -D warnings` 在**未改动**的 `src-tauri/src/crypto.rs:110` 上即会失败（新版 clippy 的 `chunks_exact_to_as_chunks` lint）。这是既有基线，不代表你的改动引入问题——判定时请用 `git stash` 前后对比，不要顺手改那行。

## 认证与数据口径

- 唯一必需的凭据是官网的 **Session Cookie**（请求头整串）。API Key 仅作历史兼容保留，不参与任何请求。
- 桌面端用 Windows DPAPI 把 Cookie/API Key 绑定当前用户 + 当前机器，落库格式 `dpapi:v1:<hex>`；手机端用 HUKS AES-256-GCM，格式 `huks:v1:<base64(iv|密文|tag)>`。**两者产物不可跨设备迁移，项目也不做跨设备凭据同步。**
- 数据来自两个官网接口：`/api/user/self`（额度、用户 ID、累计请求数）与 `POST /v1/logs/token-usage/by-model`（当日/区间的 Token 与 quota 聚合，必须携带字符串形式 `user_id`）。
- 费用换算 `QUOTA_PER_YUAN = 500_000`，即 `费用(元) = quota / 500000`。
- Cookie **不设本地过期时间**，失效一律由服务端判定（返回认证错误）。配置页展示的有效期是官网真实下发值，**只做展示**，不参与任何倒计时或拦截；官网未下发时明示"失效由服务端判定"。
- 本地数据：`%APPDATA%\com.amax.dashboard\amax_dashboard.db`（SQLite）。`config` 存凭据与 `cookie_expires_at`，`dashboard_snapshot` 存每次刷新的快照（含累计 `request_count`，永久保留不清理，旧库自动幂等迁移）。

## 架构概览

```
dist/            静态前端（index.html / style.css / app.js + vendor/）
src-tauri/src/
  lib.rs         应用编排：command 注册、共享状态、托盘、定时刷新、快照落库
  api.rs         HTTP 聚合：两个官网接口 + 状态码分级（status_error）
  db.rs          SQLite：config / dashboard_snapshot
  crypto.rs      Windows DPAPI 加解密
  error.rs       AppError{code,message}，IPC 错误契约
  login.rs       官网 WebView 登录窗与 Cookie 提取
```

前端通过 `window.__TAURI__` 调用六个 IPC command（`get_config` / `save_config` / `fetch_dashboard` / `get_local_stats` / `fetch_usage_stats` / `open_login_window`），后端经 `dashboard-updated`、`login://success|cancelled|timeout`、`auth://expired` 事件回推。CSP 的 `connect-src` 只允许 Tauri IPC，HTTP 请求一律由 Rust 侧 reqwest 发起，前端不直连官网。

所有 command 的错误统一序列化为 `{ code, message }`，`code ∈ auth | network | data | input | storage`，前端按 `code` 分支而非匹配文案。

## CI 与发布

`.github/workflows/release.yml`：推送 `v*` 标签时在 `windows-latest` 构建 MSI 并发布 GitHub Release（应用依赖 DPAPI，仅 Windows 可构建）。Release Notes 取自 `CHANGELOG.md` 中同版本小节，**发布前须先补该小节**，缺失时回退为提交列表；`workflow_dispatch` 可手动验证构建链路。

## 第三方资源

`dist/vendor/` 以 UMD 单文件存放、不参与构建：Chart.js 4.4.7（MIT）、SheetJS 0.20.3（Apache-2.0）。CSP 不变（`script-src 'self'`）。

## 文档

- [`CLAUDE.md`](CLAUDE.md) — 架构约束、模块职责、**两端有意差异表**（改动前必读，避免把"有意不实现"当成待办）
- [`CHANGELOG.md`](CHANGELOG.md) — 版本变更记录
- [`harmony/README.md`](harmony/README.md) — HarmonyOS 端开发命令、结构与实现要点
- [`docs/superpowers/specs/`](docs/superpowers/specs/) — 各功能的设计文档（含决策理由与被否决方案）
