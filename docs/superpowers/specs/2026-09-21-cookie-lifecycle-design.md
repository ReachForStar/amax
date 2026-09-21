# Cookie 生命周期与错误表达 设计

- 日期：2026-09-21
- 状态：已批准，四项均已实施；自动化验证全绿（Rust 40 例 / 前端 30 例 / 手机端 LocalTest 29 例 + ArkTS 清编译通过），仅剩真机登录确认服务端是否真下发 `Expires`
- 前置：`2026-09-20-desktop-webview-login-design.md`（桌面端 WebView 登录已落地，commit `f489c7d`）

## 目标

把上一轮打通的"两端都能自动取 Cookie"补齐为可用的生命周期闭环：

1. 桌面端错误从 `Result<_, String>` 改为结构化 `AppError`，前端按 `code` 而不是文案子串分支。
2. 认证失败时两端都给出明确的"重新登录"指向，复用已就绪的登录窗 / 登录页。
3. 凭据有效期可见：只展示服务端真实下发的过期时间，取不到就直说。
4. 两端差异收口：手机端补 10 分钟登录看门狗与 `ConfigPage.onPageShow`，并把有意保留的差异写成表。

## 非目标

- **不做倒计时拦截**：项目已删除"保存后固定 15 天过期"的启发式（见 CHANGELOG 0.2.1），失效判定归服务端。有效期只做展示。
- **不做跨端历史快照导入**（用户已否决）。
- **不做跨设备凭据同步**：桌面 DPAPI 绑当前用户+机器、手机 HUKS 绑本机沙箱，不可迁移。
- 不改 `crypto.rs` 的加密格式、不改快照 schema、不改官网接口口径。

## 1. 桌面端结构化错误

新增 `src-tauri/src/error.rs`：

```rust
pub enum ErrorCode { Auth, Network, Data, Input, Storage }   // 序列化为小写字符串

pub struct AppError { pub code: ErrorCode, pub message: String }
```

- `Serialize` 成 `{ code, message }`，即前端 `invoke()` 的 reject 载荷；`Display` 只输出 `message`，供 `eprintln!` 与后台刷新日志用。
- 提供 `AppError::auth/network/data/input/storage(...)` 构造函数与 `is_auth()`；`impl From<rusqlite::Error>` 归 `storage`，避免每处手写前缀。
- 语义边界：`auth` = 凭据问题（重试无用，必须重登）；`network` = 网络/服务端暂时不可用（可重试）；`data` = 响应解析或接口契约异常；`input` = 入参非法（日期、空凭据）；`storage` = 本地数据库/窗口等本机故障。

`Result<_, String>` → `Result<_, AppError>` 涉及：

| 位置 | 分级 |
|---|---|
| `api.rs::fetch_user_info` 非 2xx | 收进共用 `status_error(status)`：401/403→`auth`，429 与 5xx→`network`，其余→`data` |
| `api.rs::fetch_usage_stats` logs 接口非 2xx | 同一 `status_error`（**修缺陷**：原实现 401 被包成"用量查询失败: …"，不带认证语义，统计页 Cookie 失效时只静默降级本地估算） |
| `api.rs` 空 Cookie | `auth`："尚未配置 Cookie，请先登录" |
| `api.rs` send 失败 / json 解析失败 | `network` / `data` |
| `lib.rs::validate_date_range` | `input` |
| `lib.rs` 锁中毒、快照保存、开窗失败 | `storage` |
| `db.rs::save_config` / 加密 / 查询 | `storage`（统一返回 `AppError`） |
| `login.rs` 建窗/聚焦失败 | `storage` |

原来的 `认证失败: {detail}` 前缀对 429 与 5xx 也是错的（服务端故障会被当成 Cookie 失效把用户踢回配置页），一并按状态码分级修掉。

### 前端

`dist/app.js` 新增 `getErrorCode(error)`（读 reject 载荷的 `code`），替换三处文案匹配：

- `loadDashboard` catch 的 `msg.includes('401') || msg.includes('认证失败')`
- `isAuthErrorMessage()`（删除）
- `loadStats` 降级分支的认证提示判断

`getErrorMessage()` 保留（`{code, message}` 的 `message` 字段照常取出），非 Tauri 环境/字符串错误仍走它展示。

## 2. 认证失败一键重登

- **桌面**：认证错误统一经 `focusRelogin()`——切到配置页、隐藏返回按钮、状态文案改为指向 `使用官网登录获取` 按钮（`open_login_window` + `login://success` 状态机上一轮已就绪，不新增 IPC）。三处入口共用：看板刷新、手动保存验证、登录后验证。
- **手机**：`ConfigPage.saveManual` 的 catch 原来只取 `error.message`，把 `ApiError.isAuthError` 丢了；改为认证错误给出指向 `前往登录（推荐）` 的文案，非认证错误保留原 message。`DashboardPage` 已有的 `isAuthError → gotoConfig` 不动。

## 3. 有效期可见

判据（源码查证 wry 0.55.1 `webview2/mod.rs:1550-1564`）：wry 把 WebView2 的 `IsSession == true` 或 `Expires == -1.0` 映射为 `cookie::Expiration::Session`，否则按 Unix 秒映射为 `Expiration::DateTime`；`Expires == 0.0` 不被特判，会解出 1970 年。

- `login.rs`：取 session Cookie 的 `expires_datetime()`，**仅当时间戳晚于当前时间**才算真实到期（`Session`、1970、解析失败一律视为"服务端未提供"），以本地 RFC3339 随 `login://success` 一起 emit `{ cookie, expires_at }`。
- `db.rs`：`config` 表新增键 `cookie_expires_at`（明文时间戳，非机密，不动加密格式、不需迁移）。写入 Cookie 时同事务 set-or-delete 该键——换发的新 Cookie 若不带到期信息，必须清掉旧值，不能留着上一次的日期。
- `get_config` 返回 `cookie_expires_at`；配置页显示"有效期至 2026-10-01 08:00（服务端下发）"，取不到显示"服务端未提供过期时间，失效由官网判定"。
- 手动粘贴路径无到期信息 → 传 `null` 并显示"未提供"。
- **手机端结论（已查证 SDK 23）**：`fetchCookieSync` 只返回 `name=value` 串，看不到属性；`WebCookieManager.fetchAllCookies(incognito)`（`@since 23`，仅 `@syscap Web.Webview.Core`，无 `@permission`/`@systemapi`，第三方应用可调）是手机端唯一能读到 `expiresDate` + `isSessionCookie` 的通道，但**无 URL 过滤、无 `maxAge`**，须自行按 `name==='session'` 且 `domain` 后缀筛。官方对 `expiresDate` 只写"时间格式详见 Date"（未给 strftime/ISO/RFC 声明），缺 `Expires` 时实测回 `-1`，因此解析必须容错：`Format.ets::parseCookieExpiry(expiresDate, isSessionCookie, now)` 依次试 RFC-7601 串、epoch 秒/毫秒，session Cookie、空串、`-1`/`0`、非正数、过去时间全部返回 `null`，落"未提供"文案；任何异常也返回 `null`，读取失败不影响登录主流程。
- 桌面端补齐失效可见性：后台与启动定时刷新撞上认证错误时，`lib.rs::report_refresh_error` 广播 `auth://expired`，前端 `focusRelogin` 引导重登；用户正在登录（`loginPending`）、正在保存（`isSavingConfig`）或已在配置页时跳过，不打断当前操作。

## 4. 两端差异与手机端补齐

- `LoginPage.ets`：`LOGIN_TIMEOUT_MS = 10 * 60 * 1000` 看门狗（对齐桌面 `LOGIN_TIMEOUT`），超时停轮询、置 `handled`、经 `AppStorage` 留一条登录结果文案后 `router.back()`；成功与 `aboutToDisappear` 都要停表，避免离开页面后仍回调。
- `ConfigPage.ets`：补 `onPageShow()`——重读 `hasCookie` 与有效期（从登录页手动返回后状态原先不刷新），并消费 `AppStorage.authNotice`（展示后清空）。`authNotice` 是手机端跨页认证提示的**单通道**：登录超时、看板/统计页凭据失效、本地凭据无法解密，四者都是"写文案 → 回配置页"，不再各页自造提示状态。
- `Store.ets`：`saveCookie(plain, source, expiresAt)` 与桌面同构（同写同删 `cookie_expires_at`），`clearCredentials` 一并删除。
- CLAUDE.md 增加两端差异表（"两端有意差异（勿"对齐"掉）"一节），把"有意不实现"写清楚：URL 快路径（Windows `cookies_for_url` 在同步 command/事件处理器里死锁，wry#583；且手机端该快路径实测不触发，官网登录成功后不发生 `/dashboard` 重定向）、UA 伪装（官网 Next.js 对 ArkWeb 默认 UA 走错分支）、Cookie 属性可见性、加密后端（DPAPI / HUKS）、失效引导载体（事件 vs `AppStorage`）、后台刷新节奏。

## 影响文件

| 文件 | 改动 |
|---|---|
| `src-tauri/src/error.rs` | 新增 `AppError` / `ErrorCode` |
| `src-tauri/src/api.rs` | `status_error` 共用分级，签名改 `AppError`，修 logs 401 漏判 |
| `src-tauri/src/db.rs` | `save_config` 接收并写/清 `cookie_expires_at`，错误改 `AppError` |
| `src-tauri/src/lib.rs` | command 签名、`get_config` 输出有效期、`mod error` |
| `src-tauri/src/login.rs` | 到期时间提取与 emit，错误改 `AppError` |
| `dist/app.js` / `dist/index.html` | `getErrorCode`、`focusRelogin`、有效期展示、`auth://expired` 监听 |
| `harmony/.../pages/LoginPage.ets` / `ConfigPage.ets` | 看门狗、`onPageShow`、认证错误文案、有效期展示 |
| `harmony/.../common/Format.ets` / `model/Store.ets` | `parseCookieExpiry` / `formatLocalDateTime`；`cookie_expires_at` 读写与清除 |
| `harmony/.../pages/DashboardPage.ets` / `StatsPage.ets` | 认证失效与凭据解密失败时写 `authNotice` |
| `tests/frontend-navigation.test.js` | reject 载荷改 `{code, message}`，新增分支断言 |
| `harmony/entry/src/test/Format.test.ets` | `parseCookieExpiry` 四态与 `formatLocalDateTime` |
| `CLAUDE.md` / `CHANGELOG.md` | 差异表与说明 |

## 测试

- Rust：`AppError` 序列化为 `{code,message}` 与 `Display` 只出 message；`status_error` 对 401/403/429/500/其他 的分级；`validate_date_range` 断言由"文案包含"改判 `code`；到期时间提取（`Session`/1970/未来时间戳三态）；`save_config` 写/清 `cookie_expires_at` 三例。
- 前端：认证错误 reject `{code:'auth'}` 时切配置页并给出重登指向（含按钮聚焦、返回按钮隐藏）；`network` 时留在看板仅提示且**不**出现登录指向；契约错误走通用文案；统计页官方失败且 `code==='auth'` 时出现重登提示；有效期有/无两态文案；登录窗下发的 `expires_at` 透传 `save_config`；手动粘贴传 `null` 清掉旧值；`auth://expired` 在登录/保存中或已在配置页时不打断。
- 手机端（LocalTest，29 例全绿）：session Cookie 与空串/`-1`/`0`/非日期串→`null`；过去时间→`null`；RFC-7601 串、epoch 秒、epoch 毫秒三种写法解析到同一时刻；`formatLocalDateTime` 补零到分钟。
- 手工：真机登录后确认服务端是否真下发 `Expires`（此项无法匿名探测，见"未决事实"）。

## 验证命令

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node tests/frontend-navigation.test.js
# HarmonyOS（在 harmony/ 下）
D:/command-line-tools/bin/hvigorw.bat test -p module=entry -p product=default -p testType=LocalTest
devecocli build clean && devecocli build
```

## 未决事实

服务端是否真的在登录时下带 `Expires`：匿名探测 `ai.amaxsmp.com` 的 `/login` 与 `/api/user/self` 均不返回 `Set-Cookie`，无法离线确认；session 值形如 `base64(签发时间|签名)`，其中时间戳是**签发**时间不是到期时间，不可用于计算剩余期。因此本节代码只做"有则展示、无则明说"，真机登录后由 WebView2 的 `IsSession`/`Expires`（桌面）与 `fetchAllCookies` 的 `expiresDate`/`isSessionCookie`（手机）决定实际显示哪一态。

已收口的两点：wry 0.55.1 的映射读法已按源码确认（`IsSession==true` 或 `Expires==-1.0` → `Expiration::Session`，`0.0` 不解特判会得 1970，故两端都只接受晚于当前时刻的时间戳）；`fetchAllCookies` 的可用性（API 23、无权限要求）与 `WebHttpCookie` 字段表（无 `maxAge`）也已确认，唯一残留是 `expiresDate` 的字符串格式官方未定义，故按多形态容错解析。
