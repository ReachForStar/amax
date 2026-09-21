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
| 托盘与后台刷新 | 关闭/最小化即隐藏到托盘；每 10 分钟后台刷新；托盘菜单可刷新/恢复窗口/检查更新/退出 |
| 自动更新 | 启动 15 秒后首查、之后每 24 小时一查，发现新版本立即下载验签；主窗口隐藏/最小化或连续 90 秒无操作且后台刷新不在飞行中时才安装并重启，设置页可手动检查或「现在重启并安装」 |
| 低余额通知 | 剩余额度低于 10% 时发一次系统通知（后台/托盘刷新触发），恢复到阈值以上后重置 |
| 失效引导 | 凭据失效时回到配置页并直接指向登录入口；桌面端后台刷新撞上失效会主动广播引导重登 |

## 快速开始（桌面端）

环境：Windows 10/11、Rust stable（crate 用 edition 2024，本地实测 1.98.1；依赖树已不接受 1.88，`notify-rust` 4.18 声明 `rust-version = 1.89`）、`tauri-cli` 2.x（本地实测 2.11.5，release CI 固定同版本）、WebView2。前端测试需要 Node.js（CI 用 22，仅跑 `node:test`，不参与构建）。

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

> 上面五条命令就是 CI 的门槛（见「CI 与发布」），`clippy -D warnings` 的基线是干净的——出现告警即视为失败，别用 `-A` 绕过。

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
  lib.rs         应用编排：command 注册、共享状态、托盘、定时刷新、自动更新下载/空闲安装、快照落库
  api.rs         HTTP 聚合：两个官网接口 + 状态码分级（status_error）
  db.rs          SQLite：config / dashboard_snapshot
  crypto.rs      Windows DPAPI 加解密
  error.rs       AppError{code,message}，IPC 错误契约
  login.rs       官网 WebView 登录窗与 Cookie 提取
```

前端通过 `window.__TAURI__` 调用十个 IPC command（`get_config` / `save_config` / `fetch_dashboard` / `get_local_stats` / `fetch_usage_stats` / `open_login_window` / `get_app_version` / `report_user_activity` / `check_for_updates_now` / `apply_update_now`），后端经 `dashboard-updated`、`login://success|cancelled|timeout`、`auth://expired`、`update://status` 事件回推。CSP 的 `connect-src` 只允许 Tauri IPC，HTTP 请求一律由 Rust 侧 reqwest 发起，前端不直连官网。

自动更新的"什么时候装"由 Rust 侧单点判定：`report_user_activity` 只是前端活动的心跳（节流 5 秒），空闲阈值 90 秒这个常量只存在于 `lib.rs`，就绪文案随 `update://status` 一起下发，前端不复制一份。`update()` 在 Windows 上拉起 msiexec 后立即 `exit(0)`，所以下载与安装必须分成两段——先备好包再等时机，而不是检测到就装。

所有 command 的错误统一序列化为 `{ code, message }`，`code ∈ auth | network | data | input | storage`，前端按 `code` 分支而非匹配文案。

## CI 与发布

三个文件，检查命令只在 action 里写一份：

| 文件 | 触发 | 做什么 |
|---|---|---|
| `.github/actions/verify/action.yml` | 被两个工作流调用 | `cargo fmt --check` → `clippy -D warnings` → `cargo test` → `node --check dist/app.js` → `node tests/frontend-navigation.test.js` |
| `.github/workflows/test.yml` | push 到 master、PR、手动 | 在 `windows-latest` 跑上面那份检查；`paths-ignore` 跳过纯文档与纯 `harmony/` 改动；同分支旧任务自动取消 |
| `.github/workflows/release.yml` | 推送 `v*` 标签、手动 | 先跑同一份检查，再 `cargo tauri build` 出 MSI 并发布 GitHub Release |

- 两个工作流都只能在 `windows-latest` 上跑：应用依赖 DPAPI / `windows-sys`，非 Windows 构建下 `crypto.rs` 的加解密一律返回 `Err`，跑不到真实路径。私有仓库的 Windows 分钟数按 2 倍计费，这是 `paths-ignore` 与 `concurrency` 存在的原因。
- 发布门槛：标签号必须与 `src-tauri/tauri.conf.json` 的 `version` 一致，否则工作流直接失败（MSI 文件名会与 Release 标题对不上）；`Cargo.toml` 的 `version` 只用于 crate，不参与校验。
- Release Notes 取自 `CHANGELOG.md` 中同版本小节，**发布前须先补该小节**；缺失只告警并回退为上一标签以来的提交列表（该回退依赖 `fetch-depth: 0`）。
- `workflow_dispatch` 手动跑发布工作流只验证构建链路，不创建 Release，MSI 改为上传为 artifact。
- **自动更新通道由发布流程负责补齐**：`tauri.conf.json` 需 `"bundle": { "createUpdaterArtifacts": true }`（否则不产 `.sig`）；`latest.json` 由 `release.yml` 的「Generate updater manifest」步骤手写生成（`cargo tauri build` 从不产出清单，只有 `tauri-action` 会），清单里固定取 `AMAX.Dashboard_<版本>_x64_zh-CN.msi` 作为 windows-x86_64 的更新包。标签构建在缺 `TAURI_SIGNING_PRIVATE_KEY`、缺 `.sig`、或 `.sig` 的密钥 id 与 `plugins.updater.pubkey` 不一致时一律失败，不再静默发一个更新坏掉的版本。
- `plugins.updater.pubkey` 必须是 **base64(minisign `.pub` 文本块)**，不是裸 32 字节公钥——`tauri-cli` 解析不了后者，表现为构建期 "failed to decode pubkey"。换密钥要成对换：`cargo tauri signer generate` 出来的私钥进仓库 secret，公钥文件内容 base64 后填进配置。
- `tauri-cli` 版本固定在 `release.yml` 的 `TAURI_CLI_VERSION`，并据此缓存 `~/.cargo/bin/cargo-tauri.exe`；升级 CLI 时版本与缓存键要一起改，否则缓存会命中旧二进制。
- **HarmonyOS 端不在 CI 内**：hvigor / DevEco 工具链装不上 GitHub 托管 runner。改动 `harmony/` 后请本地跑 `hvigorw test -p testType=LocalTest` 与 `devecocli build`（见 [`harmony/README.md`](harmony/README.md)）。

## 第三方资源

`dist/vendor/` 以 UMD 单文件存放、不参与构建：Chart.js 4.4.7（MIT）、SheetJS 0.20.3（Apache-2.0）。CSP 不变（`script-src 'self'`）。

## 文档

- [`CLAUDE.md`](CLAUDE.md) — 架构约束、模块职责、**两端有意差异表**（改动前必读，避免把"有意不实现"当成待办）
- [`CHANGELOG.md`](CHANGELOG.md) — 版本变更记录
- [`harmony/README.md`](harmony/README.md) — HarmonyOS 端开发命令、结构与实现要点
- [`docs/superpowers/specs/`](docs/superpowers/specs/) — 各功能的设计文档（含决策理由与被否决方案）
