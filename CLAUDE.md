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

`dist/index.html`、`dist/style.css`、`dist/app.js` 构成完整前端。`src-tauri/tauri.conf.json` 的 `frontendDist` 指向 `../dist`，静态文件无需打包即可被 Tauri 加载。前端通过 `window.__TAURI__` 调用十五个 IPC command：

- `get_config`：返回 `{ has_cookie, has_api_key, cookie_expires_at }`。
- `save_config`：保存 Cookie、可选 API Key 与官网下发的 `expires_at`（`Option<String>`，`null` 表示清掉旧值）。
- `fetch_dashboard`：刷新数据、保存快照并更新托盘。
- `get_local_stats`：按起止日期查询每日快照（日末条），返回余额、本地估算值与每日新增请求数（累计差分）。
- `fetch_usage_stats`：按起止日期调用官网聚合，返回补零后的每日消耗、模型分布与区间汇总。
- `open_login_window`：打开官网 WebView 登录窗口，登录成功后自动提取 Cookie（见 `login.rs`）。
- `get_app_version`：返回 `Cargo`/`tauri.conf.json` 的当前版本号，设置页「更新」区显示。
- `report_user_activity`：前端活动心跳（`pointerdown`/`keydown`/`wheel`，节流 5 秒），只更新 `last_user_active`，返回值无意义，失败静默。
- `check_for_updates_now`：手动检查更新（设置页按钮），结果只经 `update://status` 事件回推。
- `apply_update_now`：立即安装已就绪的更新；没有暂存包时返回 `input` 类错误。成功时**不会返回**——安装会结束进程。
- `get_alert_settings`：返回 `{ settings, rulesEnabled, paramRanges, status }`，设置页「告警规则」区据此渲染；`settings` 是 `alert::AlertSettings`（**camelCase 字段**，Tauri 只自动转换 command 参数名，不转换嵌套结构体字段），`paramRanges` 的 `key` 用配置键的 snake_case 与 `settings` 字段名不同源，前端靠 `ALERT_PARAMS` 表映射。`status` 与 `alert://status` 负载同形，当日账本读不到时为 `null`。
- `set_alert_settings`：写参数与逐规则开关，返回**库里夹取后的真实值**（`sanitized()` 会把越界数字收敛），所以前端保存后按返回值回显，不拿提交值当回显；`rulesEnabled` 省略表示不动，含未知 key 返回 `input`。
- `get_alert_channels`：返回 `{ channels: deliver::view(...), deliveries: [...] }`，设置页「通知渠道」区一次读齐配置与当日投递复盘。
- `set_alert_channels`：渠道开关、MeoW 昵称与 SMTP 明文每次**整体覆盖**（视图给的就是原值，界面改了什么就提交什么）；只有 SMTP 授权码是凭据，`null` 表示保持原值、空串才删除。返回值是保存后的**新视图**，前端据此回读，不拿提交值当回显。
- `test_alert_channel`：按渠道 key 发一条测试投递，返回 `{ ok, channel, error }`——投递本身失败走返回值不走 reject。发之前校验渠道可用性，未知 key 与缺凭据返回 `input`；结果同样记进 `alert_delivery`（规则键 `test`，**不消耗**任何规则的当日额度）。

另有 `alert::rule_keys()` 的三个规则键（`quota_low` / `runout_soon` / `spike`）在前端写死成三个复选框，后端只认这三个 key。

后端通过 `dashboard-updated` 事件推送后台刷新结果，通过 `login://success` / `login://cancelled` / `login://timeout` 推送官网登录窗结果，通过 `auth://expired`（空载荷）广播后台或启动刷新撞上的凭据失效，通过 `update://status`（`{ state, version, message }`，`state ∈ disabled | checking | up_to_date | downloading | staged | installing | error`）推送更新流程各阶段，通过 `alert://status`（`{ state, day, firedToday, cap }`，`state ∈ disabled | armed | cap_hit`）推当日告警进度——设置页「告警规则」区右上角据此显示「今天 x / cap 次」，字段集合三种状态一致，前端无需按 state 分支取值。新增或重命名 command/event 时，需要同步修改 `src-tauri/src/lib.rs` 和 `dist/app.js`。

所有 command 的错误统一为 `src-tauri/src/error.rs::AppError`，序列化为 `{ code, message }` 作为 `invoke()` 的 reject 载荷；`code` 取值 `auth` / `network` / `data` / `input` / `storage`，语义分别是"凭据问题（重试无用，必须重登）/ 暂时不可用（可重试）/ 接口契约异常 / 入参非法 / 本机故障"。前端 `getErrorCode()` 按 `code` 分支，**禁止再退回文案子串匹配**（`includes('认证失败')`、`includes('401')`）；HTTP 状态分级只允许走 `api.rs::status_error(status, api)` 这一个入口，否则 401 与 5xx 会混成一类。

### Rust 后端

`src-tauri/src/lib.rs` 是应用编排中心：

- 注册 plugin（notification + single-instance + log + updater）、共享 `AppState` 和 IPC command。
- 用异步互斥锁串行化前台、托盘和定时刷新，避免重复网络请求。
- 动态创建唯一 ID 为 `main` 的托盘图标；关闭或最小化窗口时隐藏并移出任务栏，托盘单击恢复窗口。
- 每 10 分钟后台刷新，失败后 60s 起快速重试（封顶 5 分钟，成功复位）；刷新成功后走 `alert::` 告警引擎（见下文 `src-tauri/src/alert.rs` 段），**只有这条路径评估告警**，前端 `fetch_dashboard` 与启动兜底刷新都不评估；评估出 findings 后交给 `deliver::` 按渠道投递（见下文 `src-tauri/src/deliver.rs` 段）。
- 单实例：二次启动唤醒已有实例窗口，不重复创建托盘与定时器。
- 日志统一走 tauri-plugin-log（stdout + 应用日志目录 amax.log，5MB 轮转），不用 eprintln!。
- 自动更新（详见下节）：启动 15 秒后首查、之后每 24 小时一查，托盘「检查更新」与设置页按钮可手动触发；发现新版本立即下载验签并暂存，等应用空闲才安装（debug 构建跳过检查）。
- 保存刷新快照并同步托盘 tooltip。

`src-tauri/src/api.rs` 负责 HTTP 聚合：

- `/api/user/self` 提供账户额度、用户 ID 和累计请求数。
- `/v1/logs/token-usage/by-model` 按本地时区当天的 Unix 时间范围及 `status=success` 返回官网同口径的 Token/费用汇总；请求必须携带字符串形式的 `user_id`。
- by-model 响应除 `summary` 外还含 `models`（模型名、`request_count`、Token 明细、`quota`）与 `daily`（日 × 模型明细，`date` 为本地时区 0 点 Unix 时间戳，无使用量的日期无记录）；`fetch_usage_stats` 按日聚合补零、派生区间汇总，`quota` 全 0 时模型占比自动切换 `total_tokens` 口径（`percent_basis`）。
- Token 使用 `summary.total_tokens/input_tokens/output_tokens`；费用使用 `summary.quota / QUOTA_PER_YUAN`，其中 `QUOTA_PER_YUAN = 500_000`。
- 日志汇总失败时仍返回账户额度，不能把额度和日志查询改成全有或全无。

`src-tauri/src/db.rs` 管理应用数据目录中的 SQLite：`config` 保存认证信息（含明文字符串键 `cookie_expires_at`，非机密，随 Cookie 同事务 set-or-delete——新 Cookie 不带到期信息时必须清掉上一次的日期），`dashboard_snapshot` 保存刷新快照（含 `request_count` 累计值与 `day` 本地日期列，`Db::open` 对旧库幂等迁移：`ALTER TABLE ADD COLUMN` + 历史行回填 + `idx_snapshot_day` 索引），快照永久保留、不清理。注意 SQLite 的 `date(...,'localtime')` 是非确定性函数，不能建表达式索引，按日查询依赖存储的 `day` 列（写入时计算，不随查询时刻时区漂移）。Cookie 保存后**不设本地过期时间**，实际失效由服务端判定（API 返回认证错误时前端回退配置页）；`cookie_expires_at` 只用于展示，任何代码不得拿它做过期比较或拦截（`save_config_clears_stale_expires_when_new_cookie_has_none` 锁定该不变量）。

告警另存两张表，都由 `Db::open` 的 `CREATE TABLE IF NOT EXISTS` 建立（旧库开一次就有，无需 ALTER 迁移）：`alert_state(rule_key, day, count, last_fired_at)` 主键 `(rule_key, day)`，是**跨进程持久**的按日去重账本——`record_alert_fire` 用 upsert 累加，所以重启应用不会重复告警，去重语义是「同规则同日一次」而非旧版的「回升后再跌回来可以再通知一次」；`alert_delivery(rule_key, day, channel, ok, error, delivered_at)` 只增不改，记每次投递的渠道、结果与失败原因，`recent_alert_deliveries(day, limit)` 给设置页按当日倒序复盘（测试投递用规则键 `test`）。渠道开关、MeoW 昵称与 SMTP 明文走 `config` 表的明文键，只有 SMTP 授权码走 `dpapi:v1:` 加密键（`KEY_SMTP_AUTH_CODE`）——**它是这个功能唯一的凭据**：视图只给「是否已配」、永不写日志、永不进仓库，解密失败时报告「无法解密，请重新填写」而不是当成没配过。昵称按普通配置对待（推送按昵称路由，但服务端无鉴权、泄露了也只是能往你手机发消息），界面原样回显；读它用 `Db::get_secret`，明文与 `dpapi:v1:` 都认，所以昵称还按凭据存过的旧库照样读得出来，下次保存自然落成明文，解不开就按没填处理。告警四个可调参数 + 总开关 + 逐规则开关全部走 `config` 表的**明文**键（`alert_*`），判据同 `cookie_expires_at`：非机密、且要能直接查库排障；新增非机密配置沿用这一条，涉密的必须走 `dpapi:v1:`。

`src-tauri/src/alert.rs` 是纯函数告警引擎（不碰 DB、不发通知，便于单测）：`Rule::ALL` 声明顺序即投递顺序（`quota_low` 剩余水位 → `runout_soon` 燃尽天数 → `spike` 当日花费倍数），规则 key 同时是 `alert_state.rule_key`、`alert_delivery.rule_key` 和配置键后缀 `alert_rule_<key>`，**三处命名必须一致**——改一个 key 等于把历史去重账本、投递记录和该规则的开关一起孤儿掉（旧键读不到就当没投过、开关回默认开）。`build_baseline` 从日末快照算基准，只取有用量（`yuan > 0`）的日子且**排除当日**：空缺日（应用没运行）不能当成 ¥0，否则既拉低日均把正常一天判成突增、又抬高日均把快耗尽算成还能撑。突增用**中位数**而非均值（大额那天不该把自己以后各天的门槛一起抬走），样本 <3 天不判；`runout_soon` 日均为 0 时不可除，直接跳过。基准窗口内完全没有记录时三条规则一律不判（宁可漏报不误报，新装用户第一次刷新不该被轰炸）。去重/上限在 `detect`：总开关关闭或当日合计已达 `max_fires_per_day` 直接返回空；`AlertSettings::sanitized()` 逐字段夹到合法区间（非有限值取默认），所以参数读取**永不返回 None**，非法输入只会落回边界。参数经 `get_alert_settings` / `set_alert_settings` 读写，区间由 `param_ranges()` 单点定义给设置页用。`lib.rs` 的接线在 `run_alerts`：日界只取一次 `Local::now()`，参数/开关/当日次数/基准在同一次持锁里读完（拆开取会让跨午夜那一轮读到「新的一天 + 旧的一天的次数」而重复投递），整条链路只写日志不抛错——看板刷新已成功，不能因为通知发不出去而回报刷新失败；投递跑在 `tauri::async_runtime::spawn` 的分离任务里（一封超时 SMTP 要 20 秒，占着 `refresh_lock` 会把下一轮刷新一起卡住），`AppState.alerts_in_flight` 记已起飞未落库的规则防重复投递，`alert://status` 由投递任务结算完再推，避免 UI 显示「今天还没投过」。当日额度的消耗规则见 `deliver.rs` 段：**任一渠道成功、或整轮全是永久失败、或该规则当日失败行数撞上 `MAX_FAILED_DELIVERIES_PER_RULE`，都算已投**；只有「可重试」不消耗，所以下一轮还会再试——失败行上限就是给这条豁免兜底的，否则限流或断网会每 10 分钟重投一整天。`alert://status` 负载字段固定为 `{state, day, firedToday, cap}` 三种状态同构，只在内容与上次推出的一致时跳过。

`src-tauri/src/deliver.rs` 是投递层，三个渠道共用一条 `Receipt{channel, outcome, error}` 抽象：`Channel::ALL` 的声明顺序即投递顺序（系统通知 → MeoW → 邮件），`Channel::key()` 同时是配置键后缀、`test_alert_channel` 的入参和前端三个测试按钮写死的实参，改 key 会孤儿掉历史投递记录、让测试按钮报「未知渠道」。`Availability{off, incomplete, ready}` 只描述「开了且凭据齐」，UI 的状态标签与「测试投递」的前置校验共用它，不让前端自己猜。`DeliveryConfig::view()` 是给前端的唯一出口：**只有授权码不给原值**（视图给 `authCodeConfigured` 布尔），因此 `dist/app.js` 里授权码输入框读回来一律是空的，「留空 = 保持原值」、只有点「清除」才提交空串——把占位文本原样提交回去会写真凭据覆盖掉；昵称原样回显，套凭据那套「留空即保持」反而让界面说不清。`redact()` 在写日志与落库前把授权码从错误文本里替换掉，`DeliveryConfig` 与 `MailTarget` 的手写 `Debug` 同理（昵称照原值、授权码整体 `[redacted]`），所以 `{:?}` 可以安全进日志。MeoW 走 `POST https://api.chuckfang.com/{昵称}/{标题}?msgType=markdown`，昵称与标题必须经 `url` 百分号编码（中文昵称、含 `/` 的昵称会改写路由段），`status==200` 或 `data==true` 才算成功，429/5xx 归 `Retryable`；邮件用 lettre（`implicit` = 465 隐式 TLS，`starttls` = 587），其阻塞传输必须放 `spawn_blocking`，超时 20 秒，TLS 用 rustls + `rustls-native-certs`（CN 邮件服务商的证书链要能在 Windows 根证书库里验过）。`summarize()` 给整轮结论：任一成功即 `Sent`，否则有 `Retryable` 就整体 `Retryable`（不消耗当日额度，交下一轮），全永久失败才算 `Failed`。

`src-tauri/src/crypto.rs` 使用 Windows DPAPI 将 Cookie/API Key 绑定当前用户和机器，加密结果以 `dpapi:v1:<hex>` 存入 SQLite。`Db::get_cookie` / `get_api_key` 仍兼容旧明文记录；调整持久化格式时必须保留迁移路径。非 Windows 构建不提供不安全的明文加密降级。

`src-tauri/src/login.rs` 实现官网 WebView 登录获取 Cookie（对齐手机端 LoginPage）：`open_login_window`（async command，与 Tauri 内置 `create_webview_window` 同形态）创建 `login` 标签的官网登录窗，已存在则仅聚焦（幂等），初始隐藏、页面加载完成后显示。登录判定为单通道：spawned 轮询任务每 800ms 调 `cookies_for_url`（可读 HTTP-only Cookie），session 出现即成功，口径同手机端 `parseSessionCookie`。手机端另有「URL 跳转 /dashboard」快路径，桌面端**不实现**——Tauri 文档明确 `cookies_for_url` 在 Windows 上于同步 command 或事件处理器中调用会死锁（wry#583），只能在 async command/独立线程读取。到期时间由 `session_expires_at` 提取（wry 把 `IsSession==true` 或 `Expires==-1.0` 映射为 `Expiration::Session`；`Expires==0.0` 会解出 1970 年），**仅晚于当前时刻的时间戳才算真实到期**，随 `login://success` 一起 emit `{ cookie, expires_at }`。共享 `settled` 原子标志防重入并终止轮询；用户关窗广播 `login://cancelled`，约 10 分钟未完成广播 `login://timeout`。登录窗不在 `capabilities/default.json` 的 `windows: ["main"]` 内，官网页面因此不具备任何 IPC 权限。

### 自动更新（桌面端）

`lib.rs` 的更新流程刻意分成"下载"和"安装"两段，中间用 `AppState::pending_update: Mutex<Option<StagedUpdate>>` 传递，原因在插件实现本身：updater 的 `Update::install()` 在 Windows 上走 `ShellExecuteW` 拉起 `msiexec /i … AUTOLAUNCHAPP=True` 后立刻 `std::process::exit(0)`，**一旦调用就是杀进程**，不存在"边用边装"。所以 `check_for_updates()` 只做 `check()` + `download()`（`download()` 内部完成 minisign 验签，返回的字节即已验签的包），把 `{ update, bytes }` 暂存；`start_update_installer()` 每 `IDLE_INSTALL_POLL`（5 秒）轮询一次，`update_install_is_idle()` 判定时机——主窗口隐藏或最小化即视为无人看管，窗口可见则要求前端至少 `IDLE_AFTER`（90 秒）没上报过活动，另外用 `refresh_lock.try_lock()` 探测后台刷新是否在飞。`take_staged_update()` 用 `Option::take()` 保证单次消费。

要点约束：

- 阈值与文案只在 Rust 侧定义，`staged` 状态文案随 `update://status` 下发，前端不得复制 `90` 这类常量（改阈值只动 `IDLE_AFTER`）。
- `check_for_updates()` 用 `update_check_lock.try_lock()` 单飞，并发触发（定时/托盘/设置页）时后来者直接返回；已有暂存包时只重发 `staged` 状态，不重复拉包。
- debug 构建（`cfg!(debug_assertions)`）跳过检查——本地 `cargo tauri dev` 没有签名产物，报"已是最新"以外的一切结论都是噪声；表现给用户的是 `disabled` 状态加一句"开发构建不检查更新"。
- 检查结论一律有反馈：`emit_update_status()` 推事件，托盘/手动入口再补一条系统通知，失败路径走 `update_failed()`（日志 + `error` 状态 + 通知），不得静默。
- **更新端点必须匿名可读，仓库可见性属于更新契约**：`tauri-plugin-updater 2.10.1` 的 `Config` 只有 `dangerousInsecureTransportProtocol` / `endpoints` / `pubkey` / `windows`（`src/config.rs`），**没有任何凭据字段**，运行时 `app.updater()` 发的是匿名请求；私有仓库下 GitHub 对未认证请求一律 404，客户端表现为 `Could not fetch a valid release JSON from the remote`（v0.2.3 首发后正是这个现象——清单与签名都已修好，坏在源不可匿名访问）。所以发布流程把它做成门槛：`release.yml` 末尾的「Verify updater endpoint is anonymously reachable」用 `Invoke-WebRequest`（**不能用 `gh`，它带 token 会假绿**）探测 `tauri.conf.json` 里那个端点原值，清单非 200、或缺 `url`/`signature`、或更新包 Range 取首字节拿不到 200/206，都直接判失败。注意 Release 资产按 `application/octet-stream` 下发，`$resp.Content` 是 `byte[]`，解 JSON 前必须先按 UTF-8 转文本。本仓库于 2026-09-21 改为 public，**改回 private 等于关掉自动更新**。若哪天必须回到私有源码，替代方案是另建 public 发布仓或在 `app.updater_builder().headers()` 注入只读 token（token 会随二进制分发，属明确取舍而非默认）。
- **发布侧的更新通道是 CI 契约**：`bundle.createUpdaterArtifacts` 必须为 `true`（否则不产 `.sig`），`latest.json` 由 `release.yml`「Generate updater manifest」步骤生成（`cargo tauri build` 不产出清单）。更新包**从 `target/release/bundle/msi/` 里取唯一的 `*_x64_zh-CN.msi`**，取之前必须先把产物名里的空格换成点：`tauri-bundler 2.9.4 msi/mod.rs:229` 按 `productName` 原样命名（本项目 `"AMAX Dashboard"` 带空格），而 GitHub 创建 Release 资产时把空格换成点，于是 `%20` 形式的清单 url 上线即 404——v0.2.3 首发就是这样"构建成功、更新仍然是坏的"。发布末尾的「Verify updater manifest matches the published assets」拿 `gh api …/releases/tags/<tag>` 的 `browser_download_url` 与线上 `.sig` 资产内容，反查清单的 url 与 signature 两项，改名规则或签名来源任一处不一致都会当场失败，不必等客户端报。**手工修线上清单时只能改字段、签名必须取线上那份 MSI 的 `.sig`**：v0.2.3 第一次修复只想着 url，顺手用本地重建的清单整份覆盖，而 CI 构建的 MSI 与本地构建字节不同（6,877,184 vs 6,860,800），签名也就不同——url 通了，客户端下载完照样验签失败。`plugins.updater.pubkey` 是 `.tauri/amax.key.pub` 的内容原样（该文件本身即 minisign 公钥文本块的 base64，CLI 与运行时都按 `base64 → 文本块` 解，`tauri-cli bundle.rs:295` / `plugin-updater updater.rs:1453`）——裸 32 字节公钥会在打包期直接 `failed to decode pubkey`。私钥 `.tauri/amax.key` 只进仓库 secret `TAURI_SIGNING_PRIVATE_KEY`，**任何时候不得提交**（`.tauri/` 已 gitignore）；换密钥必须成对换，单边替换会被 CI 的 keyid 比对拦下。

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
| 更新通道 | GitHub Release + tauri-plugin-updater 应用内自更新（MSI 验签后空闲安装） | 无应用内更新，随应用市场/侧载分发 | 手机端上架渠道要求安装包经市场签名，应用内替换 hqf/hap 不在受支持路径内；不要为"两端对齐"给手机端加自更新 |
| 失效引导载体 | `auth://expired` 事件 + `focusRelogin()` | `AppStorage` 键 `authNotice`（看门狗/看板/统计页三处生产者，`ConfigPage.onPageShow` 消费后清空） | ArkUI 无跨页事件总线，`AppStorage` 是等价的单通道 |

统计页经看板顶栏图表按钮进入：区间选择器（预设 7/14/30 天 + 自定义起止日期，跨度上限 1096 天）驱动 `fetch_usage_stats`（官方主源）与 `get_local_stats`（余额、请求数、对比与降级数据）并行调用；官方失败时趋势与汇总回退本地估算并标注，模型分布仅官方可用；消耗趋势图可叠加本地对比数据集（默认隐藏）；导出按钮将当前区间数据输出为 CSV / JSON / XLSX。Chart.js 与 SheetJS 以 UMD 单文件存放于 `dist/vendor/`，由 `app.js` 在进入统计页 / 首次 XLSX 导出时按需注入（`ensureChartLib` / `ensureXlsxLib`），不要改回 index.html 同步加载。统计页 `loadStats` 用 `statsLoadSeq` 序号丢弃过期响应（快速切换区间时防止旧数据覆盖新区间），改加载逻辑时不要删掉该保护。

## HarmonyOS 版

`harmony/` 为 AMAX Dashboard 的 HarmonyOS 6 原生适配（ArkTS / ArkUI，Stage 模型），与桌面版共享数据口径与官网接口约定：

- 页面：DashboardPage（顶栏 📈 入口进统计页）/ StatsPage（区间选择器 7/14/30 天预设 + 自定义起止日期，跨度上限 1096 天；汇总卡、消耗趋势双轴、请求数、模型分布环图、余额趋势四图表；官方失败降级本地估算并标注，模型分布仅官方可用）/ ConfigPage（应用内 WebView 登录 + 手动粘贴，显示凭据有效期；`onPageShow` 重读凭据状态并消费 `AppStorage.authNotice`；**已集成告警区 `common/AlertPanel.ets`**）/ LoginPage（官网登录页，主判据是 800ms 轮询 `WebCookieManager.fetchCookieSync`（传完整 URL）出现 session，`onLoadIntercept` 的主框架跳 `/dashboard` 只是加速用的快路径；10 分钟 `LOGIN_TIMEOUT_MS` 看门狗超时经 `AppStorage` 留文案后 `back()`；有效期走 `fetchAllCookies(false)` 读）。
- 跨页认证提示：登录超时、看板/统计页撞上凭据失效、本地凭据无法解密，都写 `AppStorage` 键 `authNotice` 后再 `getRouter().back()`/回配置页，由 `ConfigPage.onPageShow` 一次性消费（展示后清空）。新增失效入口走同一通道，不要在页面里各自造提示状态。
- model 层与桌面版模块对应：`Api.ets`（同 api.rs 协议与容错，含 fetchDashboard 与 fetchUsageStats 区间聚合；`ApiError.isAuthError` 对应桌面 `code==='auth'`）、`Store.ets`（同 db.rs 快照 schema，含 request_count 与 `cookie_expires_at` 键——与 Cookie 同写同删；按日快照查询 + 累计差分派生请求数 + 本地估算汇总；**告警配置与账本**：`alert_state` / `alert_delivery` 两张表及 CRUD）、`Secret.ets`（HUKS AES-256-GCM 替代 DPAPI，`huks:v1:<base64(iv | 密文 | tag)>` 前缀 + 旧明文兼容）、`Format.ets::parseCookieExpiry`（把 `expiresDate` 的 RFC-7601 串/epoch 秒/毫秒统一解析为本地 RFC3339，session Cookie、空串、`-1`/`0`、过去时间一律返回 null）。
- 告警模块（手机端对齐桌面端）：`common/Alert.ets`（纯函数引擎：基准构建、规则判定、消息文案，与桌面 `src-tauri/src/alert.rs` 同口径）、`common/Deliver.ets`（投递层：系统通知 + MeoW 推送，无 SMTP；失败分类 sent/retryable/failed）、`model/Alerts.ets`（编排入口：读配置→判规则→逐渠道投递→记账→回读当日状态）、`common/AlertPanel.ets`（设置页告警区 UI：三条规则开关 + 四个参数调校 + 总开关 + 通知渠道 + 测试投递 + 当日投递复盘）。
- 统计导出：`common/Export.ets` 组装 CSV / JSON（字段口径对齐桌面版 buildExportData），经系统 DocumentViewPicker 另存；降级模式 input/output_tokens 与区间请求数置 null。图表为 Canvas 自绘 `LineChart` / `DonutChart`；数据就绪递增 chartVersion 触发重绘，ForEach 键须纳入版本号。
- 低余额通知：**已对齐桌面端**——手机端现已换成 `alert::` 三规则引擎（`common/Alert.ets`，与桌面 `src-tauri/src/alert.rs` 同口径：剩余额度低于阈值 / 按近期日均预计耗尽天数 / 今日花费达近7日中位数倍数；按日去重落 `alert_state` + `alert_delivery` 两张表，配置界面在设置页「告警规则」区逐条开关与调参），投递层为 `common/Deliver.ets`（系统通知 + MeoW 推送，无 SMTP）；编排入口 `model/Alerts.ets` 被 DashboardPage 前台刷新与 WorkScheduler 后台刷新共用。
- 后台刷新：`workscheduler/RefreshWorkSchedulerExtension.ets`，退后台时申请 WorkScheduler 延迟任务（NETWORK_TYPE_ANY、非循环、不持久化），系统调度后执行一次完整刷新并重新申请。注意系统调度限制：触发时机按条件调度非精确周期（无法复刻桌面版 10 分钟刷新），频率按应用活跃分组管控（最小间隔 2 小时起），单次回调最长 2 分钟，Extension 运行在独立进程须重新 initStore。
- HUKS GCM 参数契约（仪器测试验证后的最终形态）：GCM 强制非空 AAD（固定常量，空 AAD 报 401）；解密须以 `HUKS_TAG_AE_TAG` 传入待校验 tag、密文单独送入会话，update 返回 null、明文取自 finish（auth-then-release）；调整 Secret.ets 时不得改回空 AAD 或「密文|tag 合并送 update」的写法。
- 路由一律 `this.getUIContext().getRouter()` 实例；静态 `router` 接口自 API 18 废弃，异步回调中使用会闪退。
- 断点自适应：`BreakpointSystem`（sm<600 / 600≤md<840 / lg≥840 vp）+ 内容区 md/lg 限宽 720vp 居中；deviceTypes 为 phone / tablet / 2in1。断点注册须在窗口上屏后（`loadContent` 回调）执行，提前调用 `matchMediaSync` 抛 1300002 会中断启动。
- 工具链：devecocli（`/c/nvm4w/nodejs/devecocli`，build / run / signature / docs / ui / log），**须在 `harmony/` 目录下执行**（仓库根目录会报 “Not in a valid project directory”），且 stdout 会混入 Windows 注册表噪声（`grep -v HKLM` 过滤）；增量命中时 `CompileArkTS` 显示 UP-TO-DATE 并不代表新代码通过类型检查，要确认改动可编译用 `devecocli build clean && devecocli build`。测试用 hvigorw（`D:/command-line-tools/bin/hvigorw.bat test -p module=entry -p product=default -p testType=LocalTest`，@ohos/hypium，无需设备；仪器测试改 `-p testType=InstrumentTest`，需设备），逐条结果落在 `entry/.test/default/intermediates/test/coverage_data/test_result.txt`。
- 签名：DevEco Studio 自动签名，材料在 `~/.ohos/config/`，signingConfigs 已入 `build-profile.json5`（IDE 加密串）；证书失效时用 `devecocli signature generate --product default` 重新生成。
- 开发命令详见 `harmony/README.md`；API 细节以 `devecocli docs` 查证为准。
