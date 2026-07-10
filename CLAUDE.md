# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概况

AMAX Dashboard 是一个面向 Windows 的 Tauri 2 桌面应用，用于展示 `ai.amaxsmp.com` 账户的当日 Token 消耗和剩余额度。仓库是单成员 Cargo workspace，Rust 后端位于 `src-tauri/`，无框架前端以静态文件形式直接存放在 `dist/`；没有 Node.js 构建步骤。

## 常用命令

在仓库根目录执行：

```bash
# 开发运行（修改 Rust 文件时自动重编译）
cargo tauri dev

# Rust 编译检查
cargo check --workspace

# Rust 格式检查 / 自动格式化
cargo fmt --all -- --check
cargo fmt --all

# Clippy
cargo clippy --workspace --all-targets -- -D warnings

# 运行全部测试
cargo test --workspace

# 运行指定测试（当前仓库尚无测试）
cargo test --workspace <test_name> -- --exact --nocapture

# 检查前端 JavaScript 语法
node --check dist/app.js

# 构建 Windows MSI 安装包
cargo tauri build
```

端到端桌面验证流程记录在 `.claude/skills/verify/SKILL.md`。涉及窗口、托盘、前端交互或数据展示的改动，应运行真实应用验证，不以 `cargo check` 代替。

## 架构

### 前后端边界

`dist/index.html`、`dist/style.css`、`dist/app.js` 是完整前端。`tauri.conf.json` 的 `frontendDist` 直接指向 `../dist`，因此修改这些文件立即影响 Tauri 页面，无需打包器。前端通过 `window.__TAURI__` 调用三个 IPC command：

- `get_config`：读取认证配置及 Cookie 过期状态。
- `save_config`：保存 Cookie 和可选 API Key。
- `fetch_dashboard`：请求后端、保存快照并同步托盘提示。

后端通过 `dashboard-updated` 事件将后台刷新结果推送给前端。新增或重命名 command/event 时，必须同步修改 `src-tauri/src/lib.rs` 和 `dist/app.js`。

### Rust 后端

`src-tauri/src/lib.rs` 是应用编排中心：

- 注册 Tauri plugin、共享 `AppState` 和 IPC command。
- 创建唯一 ID 为 `main` 的系统托盘图标及菜单。
- 关闭或最小化主窗口时隐藏到托盘并移除任务栏按钮；托盘单击恢复窗口。
- 每 30 分钟后台刷新；额度低于 10% 时发送系统通知。
- 将最新当日消耗和剩余额度写入托盘 tooltip。

`src-tauri/src/api.rs` 负责 HTTP 聚合：先调用 `/api/user/self` 获取账户额度，再分页调用 `/v1/logs` 汇总当日使用量。额度单位按 `QUOTA_PER_YUAN = 500_000` 换算。日志接口失败时仍返回账户额度，因此不要将两个请求改成全有或全无的事务。

`src-tauri/src/db.rs` 管理应用数据目录中的 SQLite：`config` 保存认证信息，`dashboard_snapshot` 保存每次刷新快照。Cookie 以保存时间超过 24 小时判定过期。

`src-tauri/src/crypto.rs` 使用 Windows DPAPI 绑定当前用户和机器加密 Cookie/API Key，再以 hex 写入 SQLite；非 Windows 实现仅做 hex 编解码。调整持久化格式时需保留 `Db::get_cookie` / `get_api_key` 对旧明文数据的兼容回退。

### 配置与权限

- `src-tauri/tauri.conf.json`：窗口尺寸、静态前端入口、Windows MSI/WiX 打包和图标。
- `src-tauri/capabilities/default.json`：主窗口核心权限和通知权限。
- 托盘由 `lib.rs` 动态创建；不要同时在 `tauri.conf.json` 添加 `app.trayIcon`，否则可能生成重复托盘图标。

## 数据与界面流程

应用启动后，前端调用 `get_config`：有效 Cookie 进入看板并调用 `fetch_dashboard`，否则显示配置页。刷新成功后 `renderDashboard` 更新卡片、进度条和顶部更新时间；认证错误返回配置页，其他错误留在看板显示重试信息。Rust 侧同时保存快照并更新托盘 tooltip，后台定时刷新也走相同的数据获取和推送路径。

所有真实认证数据存放在用户应用数据目录的 SQLite 中，不在仓库内。运行依赖 AMAX 网络服务和有效 Session Cookie；没有凭据时仍可验证配置页、窗口和托盘生命周期。
