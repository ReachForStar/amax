# AMAX Dashboard（HarmonyOS 版）

AMAX Dashboard 的 HarmonyOS 6 原生适配（ArkTS / ArkUI，Stage 模型）。一期：配置页（应用内 WebView 登录 + 手动粘贴）、看板页、HUKS 凭据加密、断点自适应（phone / tablet / 2in1）。二期：统计页（区间选择 / 汇总 / 趋势 / 模型分布 / 降级）、CSV / JSON 导出、低余额通知、WorkScheduler 后台刷新。三期：凭据生命周期收口——登录页 10 分钟等待看门狗、配置页 `onPageShow` 状态重读、凭据有效期展示（只展示不拦截）、认证失败一键重登提示。

桌面端（仓库根）的功能与数据口径见 [`../README.md`](../README.md)，两端有意差异见 [`../CLAUDE.md`](../CLAUDE.md) 的"两端有意差异"表。

- bundleName：`com.amax.dashboard`，入口 ability：`EntryAbility`。
- SDK：targetSdk / compatibleSdk 均为 `6.1.0(23)`，runtimeOS HarmonyOS。
- 与桌面版（`src-tauri/`）共享数据口径与官网接口约定（Token / 费用 / 余额），但两端凭据各自绑定本机加密，不做跨设备同步。

## 开发命令

以下命令均在 `harmony/` 目录下执行。`devecocli` 位于 `/c/nvm4w/nodejs/devecocli`，`hvigorw` 位于 `D:/command-line-tools/bin/hvigorw.bat`。

```bash
# 构建（devecocli 内部注入 JBR，直接调 hvigorw 构建可能缺 java）
/c/nvm4w/nodejs/devecocli build

# 本地单元测试（entry/src/test，@ohos/hypium，无需设备）
D:/command-line-tools/bin/hvigorw.bat test -p module=entry -p product=default -p testType=LocalTest --no-daemon

# 仪器测试（entry/src/ohosTest，需连接真机或模拟器）
D:/command-line-tools/bin/hvigorw.bat test -p module=entry -p product=default -p testType=InstrumentTest --no-daemon

# 签名：首次由 DevEco Studio 自动签名，材料落在 ~/.ohos/config/，
# signingConfigs 已入 build-profile.json5（IDE 加密串）。证书失效需重新生成时：
/c/nvm4w/nodejs/devecocli signature generate --product default

# 部署运行（构建 → 签名 → 安装 → 启动）
/c/nvm4w/nodejs/devecocli run

# 截图 / 日志
/c/nvm4w/nodejs/devecocli ui screenshot --path ./tmp/shot.png
/c/nvm4w/nodejs/devecocli log --bundle-name com.amax.dashboard --from 10m

# 本地官方文档检索（API 细节以此为准）
/c/nvm4w/nodejs/devecocli docs search "<关键词>" --limit 5
/c/nvm4w/nodejs/devecocli docs read "<documentId>"
```

`devecocli ui` 系列命令需设备先开启 UITest 能力（`hdc shell param set persist.ace.testmode.enabled 1`）。

命中增量缓存时 `CompileArkTS` 会显示 `UP-TO-DATE`，这**不代表**新改的代码通过了类型检查；要确认改动可编译，用 `devecocli build clean && devecocli build`。`devecocli` 的 stdout 会混入 Windows 注册表噪声，管道里可用 `grep -v HKLM` 过滤。本地单元测试的逐条结果落在 `entry/.test/default/intermediates/test/coverage_data/test_result.txt`。

## 结构

- `entry/src/main/ets/pages/` — DashboardPage（看板：Token / 费用 / 余额，顶栏 📈 入口进统计页）/ StatsPage（统计：7/14/30 天预设 + 自定义起止区间、汇总卡、消耗趋势（双轴）/ 请求数 / 模型分布 / 余额四图表、官方失败降级本地估算并标注、CSV/JSON 导出）/ ConfigPage（手动粘贴 + 登录入口，显示凭据有效期，`onPageShow` 重读状态并消费跨页提示）/ LoginPage（官网 WebView 登录：主判据是 800ms 轮询 `WebCookieManager.fetchCookieSync` 出现 session，`onLoadIntercept` 捕 `/dashboard` 重定向只是加速用的快路径且实测不触发；10 分钟等待看门狗；有效期经 `fetchAllCookies(false)` 读取）。
- `entry/src/main/ets/model/` — Api（官网接口，同桌面版 api.rs 协议与容错，含 fetchDashboard 与 fetchUsageStats 区间聚合；`ApiError.isAuthError` 对应桌面端 `code === 'auth'`）/ Store（preferences 存凭据 + RDB 存快照，schema 同桌面版 db.rs，含 request_count 与 `cookie_expires_at`；另提供按日快照查询、累计差分派生请求数、本地估算汇总）/ Secret（HUKS AES-256-GCM 凭据加密）。
- `entry/src/main/ets/common/` — Theme（主题常量）/ BreakpointSystem（sm<600 / 600≤md<840 / lg≥840 vp 断点）/ GridBackground（网格背景）/ Format（纯函数工具，含 `parseSessionCookie`、`parseCookieExpiry`、`localRfc3339`、`formatLocalDateTime`）/ LineChart（Canvas 折线图：双轴、降级虚线、null 断线）/ DonutChart（Canvas 环图）/ Export（CSV/JSON 组装，字段口径对齐桌面版，系统 Picker 另存）/ Notify（低余额通知：percent<10 一次，回升重置）。
- `entry/src/main/ets/workscheduler/` — RefreshWorkSchedulerExtension：退后台时申请 WorkScheduler 延迟任务，由系统按条件调度执行一次完整刷新（拉数 → 存快照 → 低余额检查），完成后重新申请形成机会性周期。
- `entry/src/main/ets/entryability/` — EntryAbility：窗口生命周期、断点注册（须待窗口上屏后）、onBackground 申请延迟刷新。
- `entry/src/test/` — 本地单元测试（Format 等纯函数，@ohos/hypium）。
- `entry/src/ohosTest/` — 仪器测试脚手架（HUKS 加解密往返、旧明文兼容、Store 凭据与快照往返）。

## 实现要点

- 路由一律 `this.getUIContext().getRouter()` 实例；静态 `router` API 18 起废弃，异步回调中使用会闪退。
- HUKS GCM 参数契约（Secret.ets）：存储格式 `huks:v1:<base64(iv | 密文 | tag)>`；GCM 强制非空 AAD（固定常量，空 AAD 报 401）；解密须以 `HUKS_TAG_AE_TAG` 传入待校验 tag、密文单独送入会话，明文取自 finish（auth-then-release）；无前缀旧明文原样返回（兼容迁移）。
- 断点自适应：mediaquery 监听三档写入 AppStorage `currentBreakpoint`，内容区 md/lg 限宽 720vp 居中。
- WorkScheduler 系统调度限制（文档查证 work-scheduler）：触发时机由系统按条件调度（网络/充电/空闲、内存、功耗等），非精确周期，无法复刻桌面版 10 分钟刷新；频率按应用活跃分组管控（活跃组最小间隔 2 小时，低活跃 4/24/48 小时）；单次回调最长 2 分钟；Extension 运行在独立进程，Store 单例须回调内重新 initStore。
- 统计页状态驱动图表重绘：数据就绪递增 `chartVersion`，ForEach 键纳入版本号（faqs-arkui-619）；@Builder 直读状态规避按值传参不刷新（faqs-arkui-375）。
- 跨页认证提示单通道：登录超时、看板/统计页凭据失效、本地凭据无法解密，一律写 `AppStorage` 键 `authNotice` 后回配置页，由 `ConfigPage.onPageShow` 一次性消费（展示后清空）。新增失效入口走同一通道，不要在页面里各自造提示状态。
- 凭据有效期（只展示不拦截）：官网未下发、`expiresDate` 为空/`-1`/过去时间、session Cookie、解析异常，全部按"未提供"处理（`Format.ets::parseCookieExpiry`）。`fetchAllCookies` 返回全站 Cookie，须自行按 `name === 'session'` 且 `domain` 后缀筛选；该接口无 URL 过滤、无 `maxAge`，且官方未定义 `expiresDate` 字符串格式，故解析器同时兼容 RFC-7601 串与 epoch 秒/毫秒。**任何代码不得用这个值做过期比较或拦截**——失效判定归服务端。
- 登录页定时器成对停止：看门狗与轮询在成功、超时、`aboutToDisappear` 三条路径上都要停表，失败重试时再成对重启，否则离开页面后仍会回调。

## 路线

- 二期已完成：统计页（趋势 / 模型分布 / 汇总 / 余额）、CSV / JSON 导出、低余额通知、WorkScheduler 后台刷新。
- 三期已完成：凭据生命周期（登录看门狗、有效期展示、一键重登、`onPageShow`）。
- **已否决**：跨端历史快照导入（两端日末条按 `MAX(id)` 取值，导入的外部快照会改写本地当日日末值；若将来重提，前置条件是先改成按 `saved_at` 判定并加唯一约束）。除此以外当前无既定后续路线。
