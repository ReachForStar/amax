# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概况

AMAX Dashboard 是面向 Windows 的 Tauri 2 桌面应用，展示 `ai.amaxsmp.com` 账户的当日 Token 消耗、当日费用和剩余额度。仓库是单成员 Cargo workspace；Rust 后端位于 `src-tauri/`，无框架前端是 `dist/` 中的静态 HTML/CSS/JavaScript，没有 Node.js 构建步骤。

## 常用命令

在仓库根目录执行：

```bash
# 开发运行；Rust 文件变化时自动重编译
cargo tauri dev

# Rust 静态检查
cargo check --workspace
cargo fmt --all -- --check
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# 全部测试 / 单个精确测试
cargo test --workspace
cargo test --workspace <test_name> -- --exact --nocapture

# 前端 JavaScript 语法检查
node --check dist/app.js

# 构建 Windows MSI
cargo tauri build
```

桌面端到端流程记录在 `.claude/skills/verify/SKILL.md`。涉及窗口、托盘、前端交互或数据展示时，必须运行真实应用验证；若 `target/debug/amax.exe` 被已有进程占用，应先让人工关闭应用，不要用 `cargo check` 代替运行验证。

## 架构

### 前后端边界

`dist/index.html`、`dist/style.css`、`dist/app.js` 构成完整前端。`src-tauri/tauri.conf.json` 的 `frontendDist` 指向 `../dist`，静态文件无需打包即可被 Tauri 加载。前端通过 `window.__TAURI__` 调用三个 IPC command：

- `get_config`：读取认证配置和 Cookie 过期状态。
- `save_config`：保存 Cookie 和可选 API Key。
- `fetch_dashboard`：刷新数据、保存快照并更新托盘。

后端通过 `dashboard-updated` 事件推送后台刷新结果。新增或重命名 command/event 时，需要同步修改 `src-tauri/src/lib.rs` 和 `dist/app.js`。

### Rust 后端

`src-tauri/src/lib.rs` 是应用编排中心：

- 注册 plugin、共享 `AppState` 和 IPC command。
- 用异步互斥锁串行化前台、托盘和定时刷新，避免重复网络请求。
- 动态创建唯一 ID 为 `main` 的托盘图标；关闭或最小化窗口时隐藏并移出任务栏，托盘单击恢复窗口。
- 每 30 分钟后台刷新；剩余额度低于 10% 时发送一次系统通知。
- 保存刷新快照并同步托盘 tooltip。

`src-tauri/src/api.rs` 负责 HTTP 聚合：

- `/api/user/self` 提供账户额度、用户 ID 和累计请求数。
- `/v1/logs/token-usage/by-model` 按本地时区当天的 Unix 时间范围及 `status=success` 返回官网同口径的 Token/费用汇总；请求必须携带字符串形式的 `user_id`。
- Token 使用 `summary.total_tokens/input_tokens/output_tokens`；费用使用 `summary.quota / QUOTA_PER_YUAN`，其中 `QUOTA_PER_YUAN = 500_000`。
- 日志汇总失败时仍返回账户额度，不能把额度和日志查询改成全有或全无。

`src-tauri/src/db.rs` 管理应用数据目录中的 SQLite：`config` 保存认证信息，`dashboard_snapshot` 保存刷新快照并清理 90 天前记录。Cookie 保存超过 24 小时视为过期。

`src-tauri/src/crypto.rs` 使用 Windows DPAPI 将 Cookie/API Key 绑定当前用户和机器，加密结果以 `dpapi:v1:<hex>` 存入 SQLite。`Db::get_cookie` / `get_api_key` 仍兼容旧明文记录；调整持久化格式时必须保留迁移路径。非 Windows 构建不提供不安全的明文加密降级。

### 配置与权限

- `src-tauri/tauri.conf.json`：静态前端入口、520×680 主窗口、CSP、Windows MSI/WiX 和图标。
- `src-tauri/capabilities/default.json`：主窗口核心权限和通知权限。
- CSP 的 `connect-src` 只允许 Tauri IPC；HTTP 请求由 Rust/reqwest 发起，不从前端直接访问 AMAX API。
- 托盘由 `lib.rs` 动态创建，不要在 `tauri.conf.json` 再添加 `app.trayIcon`，否则会生成重复图标。

## 数据与界面流程

启动后，前端调用 `get_config`：有效 Cookie 进入看板并调用 `fetch_dashboard`，否则显示配置页。`loadDashboard` 合并并发刷新请求；成功后 `renderDashboard` 更新卡片和更新时间，认证错误返回配置页，其他错误保留已有数据并显示重试信息。后台定时刷新走相同 Rust 聚合路径，通过事件更新前端。

真实认证数据只存放在用户应用数据目录的 SQLite 中。运行数据请求依赖有效 Session Cookie；API Key 当前仅作为兼容配置保留，不参与官网账户级聚合。没有凭据时仍可验证配置页、窗口和托盘生命周期。
