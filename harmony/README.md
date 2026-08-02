# AMAX Dashboard（HarmonyOS 版）

AMAX Dashboard 的 HarmonyOS 6 原生适配（ArkTS / ArkUI，Stage 模型）。子项目一期：配置页（应用内 WebView 登录 + 手动粘贴）、看板页、HUKS 凭据加密、断点自适应（phone / tablet / 2in1）。

- bundleName：`com.amax.dashboard`，入口 ability：`EntryAbility`。
- SDK：targetSdk / compatibleSdk 均为 `6.1.0(23)`，runtimeOS HarmonyOS。
- 与桌面版（`src-tauri/`）共享数据口径与官网接口约定（Token / 费用 / 余额）。

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

## 结构

- `entry/src/main/ets/pages/` — DashboardPage（看板：Token / 费用 / 余额）/ ConfigPage（手动粘贴 + 登录入口）/ LoginPage（官网 WebView 登录，`onLoadIntercept` 捕获 `/dashboard` 重定向并经 `WebCookieManager.fetchCookieSync` 提取 Cookie）。
- `entry/src/main/ets/model/` — Api（官网接口，同桌面版 api.rs 协议与容错）/ Store（preferences 存凭据 + RDB 存快照，schema 同桌面版 db.rs，含 request_count）/ Secret（HUKS AES-256-GCM 凭据加密）。
- `entry/src/main/ets/common/` — Theme（主题常量）/ BreakpointSystem（sm<600 / 600≤md<840 / lg≥840 vp 断点）/ GridBackground（网格背景）/ Format（纯函数工具）。
- `entry/src/main/ets/entryability/` — EntryAbility：窗口生命周期、断点注册（须待窗口上屏后）、后台定时刷新与低余额通知。
- `entry/src/test/` — 本地单元测试（Format 等纯函数，@ohos/hypium）。
- `entry/src/ohosTest/` — 仪器测试脚手架（HUKS 加解密往返、旧明文兼容、Store 凭据与快照往返）。

## 实现要点

- 路由一律 `this.getUIContext().getRouter()` 实例；静态 `router` API 18 起废弃，异步回调中使用会闪退。
- HUKS GCM 参数契约（Secret.ets）：存储格式 `huks:v1:<base64(iv | 密文 | tag)>`；GCM 强制非空 AAD（固定常量，空 AAD 报 401）；解密须以 `HUKS_TAG_AE_TAG` 传入待校验 tag、密文单独送入会话，明文取自 finish（auth-then-release）；无前缀旧明文原样返回（兼容迁移）。
- 断点自适应：mediaquery 监听三档写入 AppStorage `currentBreakpoint`，内容区 md/lg 限宽 720vp 居中。

## 路线

- 二期：统计页（趋势 / 模型分布 / 汇总 / 余额）、数据导出、低余额通知、后台定时刷新。
