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

# 全部测试 / 单个测试（`--exact` 需要完整模块路径）
cargo test --workspace
cargo test --workspace db::tests::save_config_clears_stale_expires_when_new_cookie_has_none -- --exact --nocapture

# 按名称过滤测试（无需完整模块路径）
cargo test --workspace <test_name> -- --nocapture

# 前端：JavaScript 语法检查 + 无依赖的 node:test 逻辑回归
node --check dist/app.js
node tests/frontend-navigation.test.js

# 构建 Windows MSI
cargo tauri build
```

上面这些命令就是 CI 的门槛：`.github/actions/verify/action.yml` 是唯一的检查命令列表，`test.yml`（push master / PR）与 `release.yml`（`v*` 标签）都调用它，只在 `windows-latest` 上跑（非 Windows 构建下 `crypto.rs` 的加解密一律返回 `Err`，跑不到真实路径）。要增删检查项只改那个 action，不要在两个 workflow 里各写一份。HarmonyOS 端不在 CI 内（hvigor / DevEco 装不上托管 runner），改 `harmony/` 后须本地跑 `hvigorw test -p testType=LocalTest` 与 `devecocli build`。

涉及窗口、托盘、前端交互或数据展示时，必须运行 `cargo tauri dev` 验证真实 WebView2 应用。若已有 `amax.exe` 运行，先让人工关闭，不要终止未知进程，也不要用 `cargo check` 代替运行验证。

## 架构

### 前后端边界

`dist/index.html`、`dist/style.css`、`dist/app.js` 构成完整前端。`src-tauri/tauri.conf.json` 的 `frontendDist` 指向 `../dist`，静态文件无需打包即可被 Tauri 加载。前端通过 `window.__TAURI__` 调用六个 IPC command：

- `get_config`：返回 `{ has_cookie, has_api_key, cookie_expires_at }`。
- `save_config`：保存 Cookie、可选 API Key 与官网下发的 `expires_at`（`Option<String>`，`null` 表示清掉旧值）。
- `fetch_dashboard`：刷新数据、保存快照并更新托盘。
- `get_local_stats`：按起止日期查询每日快照（日末条），返回余额、本地估算值与每日新增请求数（累计差分）。
- `fetch_usage_stats`：按起止日期调用官网聚合，返回补零后的每日消耗、模型分布与区间汇总。
- `open_login_window`：打开官网 WebView 登录窗口，登录成功后自动提取 Cookie（见 `login.rs`）。

后端通过 `dashboard-updated` 事件推送后台刷新结果，通过 `login://success` / `login://cancelled` / `login://timeout` 推送官网登录窗结果，通过 `auth://expired`（空载荷）广播后台或启动刷新撞上的凭据失效。新增或重命名 command/event 时，需要同步修改 `src-tauri/src/lib.rs` 和 `dist/app.js`。

所有 command 的错误统一为 `src-tauri/src/error.rs::AppError`，序列化为 `{ code, message }` 作为 `invoke()` 的 reject 载荷；`code` 取值 `auth` / `network` / `data` / `input` / `storage`，语义分别是"凭据问题（重试无用，必须重登）/ 暂时不可用（可重试）/ 接口契约异常 / 入参非法 / 本机故障"。前端 `getErrorCode()` 按 `code` 分支，**禁止再退回文案子串匹配**（`includes('认证失败')`、`includes('401')`）；HTTP 状态分级只允许走 `api.rs::status_error(status, api)` 这一个入口，否则 401 与 5xx 会混成一类。

### Rust 后端

`src-tauri/src/lib.rs` 是应用编排中心：

- 注册 plugin、共享 `AppState` 和 IPC command。
- 用异步互斥锁串行化前台、托盘和定时刷新，避免重复网络请求。
- 动态创建唯一 ID 为 `main` 的托盘图标；关闭或最小化窗口时隐藏并移出任务栏，托盘单击恢复窗口。
- 每 10 分钟后台刷新；剩余额度低于 10% 时发送一次系统通知。
- 保存刷新快照并同步托盘 tooltip。

`src-tauri/src/api.rs` 负责 HTTP 聚合：

- `/api/user/self` 提供账户额度、用户 ID 和累计请求数。
- `/v1/logs/token-usage/by-model` 按本地时区当天的 Unix 时间范围及 `status=success` 返回官网同口径的 Token/费用汇总；请求必须携带字符串形式的 `user_id`。
- by-model 响应除 `summary` 外还含 `models`（模型名、`request_count`、Token 明细、`quota`）与 `daily`（日 × 模型明细，`date` 为本地时区 0 点 Unix 时间戳，无使用量的日期无记录）；`fetch_usage_stats` 按日聚合补零、派生区间汇总，`quota` 全 0 时模型占比自动切换 `total_tokens` 口径（`percent_basis`）。
- Token 使用 `summary.total_tokens/input_tokens/output_tokens`；费用使用 `summary.quota / QUOTA_PER_YUAN`，其中 `QUOTA_PER_YUAN = 500_000`。
- 日志汇总失败时仍返回账户额度，不能把额度和日志查询改成全有或全无。

`src-tauri/src/db.rs` 管理应用数据目录中的 SQLite：`config` 保存认证信息（含明文字符串键 `cookie_expires_at`，非机密，随 Cookie 同事务 set-or-delete——新 Cookie 不带到期信息时必须清掉上一次的日期），`dashboard_snapshot` 保存刷新快照（含 `request_count` 累计值，`Db::open` 对旧库幂等 `ALTER TABLE ADD COLUMN` 迁移），快照永久保留、不清理。Cookie 保存后**不设本地过期时间**，实际失效由服务端判定；`cookie_expires_at` 只用于展示，任何代码不得拿它做过期比较或拦截（`save_config_clears_stale_expires_when_new_cookie_has_none` 锁定该不变量）。

`src-tauri/src/crypto.rs` 使用 Windows DPAPI 将 Cookie/API Key 绑定当前用户和机器，加密结果以 `dpapi:v1:<hex>` 存入 SQLite。`Db::get_cookie` / `get_api_key` 仍兼容旧明文记录；调整持久化格式时必须保留迁移路径。非 Windows 构建不提供不安全的明文加密降级。

`src-tauri/src/login.rs` 实现官网 WebView 登录获取 Cookie（对齐手机端 LoginPage）：`open_login_window`（async command，与 Tauri 内置 `create_webview_window` 同形态）创建 `login` 标签的官网登录窗，已存在则仅聚焦（幂等），初始隐藏、页面加载完成后显示。登录判定为单通道：spawned 轮询任务每 800ms 调 `cookies_for_url`（可读 HTTP-only Cookie），session 出现即成功，口径同手机端 `parseSessionCookie`。手机端另有「URL 跳转 /dashboard」快路径，桌面端**不实现**——Tauri 文档明确 `cookies_for_url` 在 Windows 上于同步 command 或事件处理器中调用会死锁（wry#583），只能在 async command/独立线程读取。到期时间由 `session_expires_at` 提取（wry 把 `IsSession==true` 或 `Expires==-1.0` 映射为 `Expiration::Session`；`Expires==0.0` 会解出 1970 年），**仅晚于当前时刻的时间戳才算真实到期**，随 `login://success` 一起 emit `{ cookie, expires_at }`。共享 `settled` 原子标志防重入并终止轮询；用户关窗广播 `login://cancelled`，约 10 分钟未完成广播 `login://timeout`。登录窗不在 `capabilities/default.json` 的 `windows: ["main"]` 内，官网页面因此不具备任何 IPC 权限。

### 配置与权限

- `src-tauri/tauri.conf.json`：静态前端入口、520×680 主窗口、CSP、Windows MSI/WiX 和图标。
- `src-tauri/capabilities/default.json`：主窗口核心权限和通知权限。
- CSP 的 `connect-src` 只允许 Tauri IPC；HTTP 请求由 Rust/reqwest 发起，不从前端直接访问 AMAX API。
- 托盘由 `lib.rs` 动态创建，不要在 `tauri.conf.json` 再添加 `app.trayIcon`，否则会生成重复图标。

## 数据与界面流程

启动后，前端调用 `get_config`：有效 Cookie 进入看板并调用 `fetch_dashboard`，否则显示配置页。`loadDashboard` 合并并发刷新请求；成功后 `renderDashboard` 更新数据与更新时间，`code==='auth'` 时经 `focusRelogin()` 回配置页（隐藏返回按钮、状态文案指向「使用官网登录获取」并聚焦该按钮），其他错误保留已有数据并显示重试信息（`network` 单独文案，避免把服务端故障说成凭据失效）。后台与启动定时刷新撞上认证错误时，`lib.rs::report_refresh_error` 广播 `auth://expired`，前端在用户未处于登录/保存流程且不在配置页时才切页引导重登。前端 JS 同时兼容 `window.__TAURI__.core.invoke`、`window.__TAURI_INTERNALS__.invoke` 和旧版 `window.__TAURI__.invoke`；调整 IPC 封装时不要删掉兼容分支。

真实认证数据只存放在用户应用数据目录的 SQLite 中。运行数据请求依赖有效 Session Cookie；API Key 当前仅作为兼容配置保留，不参与官网账户级聚合。没有凭据时仍可验证配置页、窗口和托盘生命周期。

配置页获取 Cookie 有两条路径，与手机端一致：主路径点击“使用官网登录获取”经 `open_login_window` 在应用内登录官网，`login.rs` 提取整串 Cookie 后广播 `login://success`，前端复用 `save_config` + `fetch_dashboard` 验证链路；兜底路径仍支持从浏览器 F12 手动粘贴 session 值（该路径无到期信息，`expiresAt` 传 `null`）。两端凭据均绑定本机加密保存，不做跨设备同步。配置页另有有效期一行：官网下发时显示“凭据有效期至 …（官网下发，仅供展示）”，未下发时显示“官网未下发过期时间，失效由服务端判定”——两态都只是展示，不参与任何拦截。

### 两端有意差异（勿“对齐”掉）

| 维度 | 桌面端 | 手机端 | 原因 |
|---|---|---|---|
| 登录判定 | 仅 800ms 轮询 `cookies_for_url` | 轮询 + `onLoadIntercept` 主框架跳 `/dashboard` 快路径 | Windows 上 `cookies_for_url` 在同步 command/事件处理器中调用会死锁（wry#583），桌面端只能在 async command/spawned 任务里读 Cookie。手机端 URL 快路径实测**不触发**（官网登录成功后不发生 `/dashboard` 重定向），保留仅作加速，判据仍靠轮询 |
| User-Agent | 不伪装（WebView2 即桌面 Chrome） | `controller.setCustomUserAgent` 伪装桌面 Chrome | 官网 Next.js 对含 OpenHarmony/ArkWeb 标识的默认 UA 走错代码路径抛客户端异常；组件属性 `.userAgent()` 自 API 10 废弃，须在 `onControllerAttached` 设置 |
| Cookie 属性可见性 | `cookies_for_url` 返回结构化 `Cookie`，可读 `expires_datetime()` | `fetchCookieSync` 只给 `name=value` 串，属性须走 `fetchAllCookies(false)`（@since 23，返回 `expiresDate` 字符串 + `isSessionCookie`，无 URL 过滤、无 `maxAge`） | 两套 WebView API 不同；`expiresDate` 官方仅称“时间格式详见 Date”，实测可能是 RFC-7601 串或 epoch，缺 `Expires` 时回 `-1`，故 `Format.ets::parseCookieExpiry` 必须多形态容错、拿不到即返回 null |
| 加密后端 | Windows DPAPI（`dpapi:v1:`） | HUKS AES-256-GCM（`huks:v1:`） | 平台原生密钥库；两者产物不可互迁，故无跨设备同步 |
| 定时刷新 | 每 10 分钟本地定时器 | WorkScheduler 延迟任务（最小间隔 2 小时起、非精确周期、回调上限 2 分钟） | 系统对后台任务的调度管控，无法复刻桌面节奏 |
| 失效引导载体 | `auth://expired` 事件 + `focusRelogin()` | `AppStorage` 键 `authNotice`（看门狗/看板/统计页三处生产者，`ConfigPage.onPageShow` 消费后清空） | ArkUI 无跨页事件总线，`AppStorage` 是等价的单通道 |

统计页经看板顶栏图表按钮进入：区间选择器（预设 7/14/30 天 + 自定义起止日期，跨度上限 1096 天）驱动 `fetch_usage_stats`（官方主源）与 `get_local_stats`（余额、请求数、对比与降级数据）并行调用；官方失败时趋势与汇总回退本地估算并标注，模型分布仅官方可用；消耗趋势图可叠加本地对比数据集（默认隐藏）；导出按钮将当前区间数据输出为 CSV / JSON / XLSX（Chart.js 与 SheetJS 以 UMD 单文件存放于 `dist/vendor/`）。

## HarmonyOS 版

`harmony/` 为 AMAX Dashboard 的 HarmonyOS 6 原生适配（ArkTS / ArkUI，Stage 模型），与桌面版共享数据口径与官网接口约定：

- 页面：DashboardPage（顶栏 📈 入口进统计页）/ StatsPage（区间选择器 7/14/30 天预设 + 自定义起止日期，跨度上限 1096 天；汇总卡、消耗趋势双轴、请求数、模型分布环图、余额趋势四图表；官方失败降级本地估算并标注，模型分布仅官方可用）/ ConfigPage（应用内 WebView 登录 + 手动粘贴，显示凭据有效期；`onPageShow` 重读凭据状态并消费 `AppStorage.authNotice`）/ LoginPage（官网登录页，主判据是 800ms 轮询 `WebCookieManager.fetchCookieSync`（传完整 URL）出现 session，`onLoadIntercept` 的主框架跳 `/dashboard` 只是加速用的快路径；10 分钟 `LOGIN_TIMEOUT_MS` 看门狗超时经 `AppStorage` 留文案后 `back()`；有效期走 `fetchAllCookies(false)` 读）。
- 跨页认证提示：登录超时、看板/统计页撞上凭据失效、本地凭据无法解密，都写 `AppStorage` 键 `authNotice` 后再 `getRouter().back()`/回配置页，由 `ConfigPage.onPageShow` 一次性消费（展示后清空）。新增失效入口走同一通道，不要在页面里各自造提示状态。
- model 层与桌面版模块对应：`Api.ets`（同 api.rs 协议与容错，含 fetchDashboard 与 fetchUsageStats 区间聚合；`ApiError.isAuthError` 对应桌面 `code==='auth'`）、`Store.ets`（同 db.rs 快照 schema，含 request_count 与 `cookie_expires_at` 键——与 Cookie 同写同删；按日快照查询 + 累计差分派生请求数 + 本地估算汇总）、`Secret.ets`（HUKS AES-256-GCM 替代 DPAPI，`huks:v1:<base64(iv | 密文 | tag)>` 前缀 + 旧明文兼容）、`Format.ets::parseCookieExpiry`（把 `expiresDate` 的 RFC-7601 串/epoch 秒/毫秒统一解析为本地 RFC3339，session Cookie、空串、`-1`/`0`、过去时间一律返回 null）。
- 统计导出：`common/Export.ets` 组装 CSV / JSON（字段口径对齐桌面版 buildExportData），经系统 DocumentViewPicker 另存；降级模式 input/output_tokens 与区间请求数置 null。图表为 Canvas 自绘 `LineChart` / `DonutChart`；数据就绪递增 chartVersion 触发重绘，ForEach 键须纳入版本号。
- 低余额通知：`common/Notify.ets`，percent<10 发一次系统通知（模块级标志防重发），回升到阈值及以上重置；前台刷新与后台刷新共用。
- 后台刷新：`workscheduler/RefreshWorkSchedulerExtension.ets`，退后台时申请 WorkScheduler 延迟任务（NETWORK_TYPE_ANY、非循环、不持久化），系统调度后执行一次完整刷新并重新申请。注意系统调度限制：触发时机按条件调度非精确周期（无法复刻桌面版 10 分钟刷新），频率按应用活跃分组管控（最小间隔 2 小时起），单次回调最长 2 分钟，Extension 运行在独立进程须重新 initStore。
- HUKS GCM 参数契约（仪器测试验证后的最终形态）：GCM 强制非空 AAD（固定常量，空 AAD 报 401）；解密须以 `HUKS_TAG_AE_TAG` 传入待校验 tag、密文单独送入会话，update 返回 null、明文取自 finish（auth-then-release）；调整 Secret.ets 时不得改回空 AAD 或「密文|tag 合并送 update」的写法。
- 路由一律 `this.getUIContext().getRouter()` 实例；静态 `router` 接口自 API 18 废弃，异步回调中使用会闪退。
- 断点自适应：`BreakpointSystem`（sm<600 / 600≤md<840 / lg≥840 vp）+ 内容区 md/lg 限宽 720vp 居中；deviceTypes 为 phone / tablet / 2in1。断点注册须在窗口上屏后（`loadContent` 回调）执行，提前调用 `matchMediaSync` 抛 1300002 会中断启动。
- 工具链：devecocli（`/c/nvm4w/nodejs/devecocli`，build / run / signature / docs / ui / log），**须在 `harmony/` 目录下执行**（仓库根目录会报 “Not in a valid project directory”），且 stdout 会混入 Windows 注册表噪声（`grep -v HKLM` 过滤）；增量命中时 `CompileArkTS` 显示 UP-TO-DATE 并不代表新代码通过类型检查，要确认改动可编译用 `devecocli build clean && devecocli build`。测试用 hvigorw（`D:/command-line-tools/bin/hvigorw.bat test -p module=entry -p product=default -p testType=LocalTest`，@ohos/hypium，无需设备；仪器测试改 `-p testType=InstrumentTest`，需设备），逐条结果落在 `entry/.test/default/intermediates/test/coverage_data/test_result.txt`。
- 签名：DevEco Studio 自动签名，材料在 `~/.ohos/config/`，signingConfigs 已入 `build-profile.json5`（IDE 加密串）；证书失效时用 `devecocli signature generate --product default` 重新生成。
- 开发命令详见 `harmony/README.md`；API 细节以 `devecocli docs` 查证为准。
