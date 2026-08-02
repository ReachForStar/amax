# HarmonyOS 原生版 · 子项目 1：基础 + 配置 + 看板 设计

日期：2026-08-02
状态：待评审

## 背景与目标

AMAX Dashboard 当前为 Windows Tauri 2 桌面应用。用户需要 HarmonyOS 6 手机原生适配。Tauri 不支持 HarmonyOS，本适配为独立 ArkTS/ArkUI 原生工程，置于现仓库 `harmony/` 子目录（monorepo）。

整体移植分两期，本文档为**子项目 1**：工程脚手架、网络层、存储与凭据加密、配置页（含应用内 WebView 登录）、看板页、**多设备自适应 UI 架构**（手机 / 折叠屏 / 平板）。完成标志：真机/模拟器可安装使用，登录后可查看当日消耗数据，各设备类型布局正常。

- 子项目 2：统计页（趋势 / 模型分布 / 汇总 / 余额 + 区间选择器）、数据导出（CSV / JSON / XLSX）、低余额通知、后台定时刷新（复用本期自适应框架）

## 关键决策（已与用户确认）

| 决策点 | 结论 |
|--------|------|
| 技术栈 | HarmonyOS 原生（ArkTS / ArkUI），Tauri 不支持鸿蒙，无法复用桌面代码 |
| 项目放置 | 现仓库 `harmony/` 子目录，共享 spec / plan / CHANGELOG 与 git 流程 |
| 实施节奏 | 两期拆分，本文档为子项目 1（含自适应架构） |
| 设备形态 | phone / 折叠屏 / 平板全支持；BreakpointSystem 断点系统内置于一期，二期页面直接复用 |
| Cookie 获取 | 应用内 WebView 加载官网登录页，成功后自动提取 session Cookie；保留手动粘贴备用入口 |
| 视觉风格 | 复刻桌面版设计语言（绿灰色调、等宽数字、网格背景、telemetry 标签） |
| 开发工具链 | devecocli v1.2.1（脚手架 / 构建 / 签名 / 模拟器 / UI 检测 / 本地文档） |

## 架构与模块边界

```
harmony/                          # devecocli create 脚手架生成
├── AppScope/                     # bundleName: com.amax.dashboard（与桌面版 identifier 一致）
├── entry/src/main/
│   ├── ets/
│   │   ├── entryability/EntryAbility.ets   # UIAbility 入口，窗口与初始页路由
│   │   ├── pages/
│   │   │   ├── DashboardPage.ets           # 看板
│   │   │   ├── ConfigPage.ets              # 配置（登录入口 + 手动粘贴 + API Key 可选）
│   │   │   └── LoginPage.ets               # WebView 官网登录
│   │   ├── model/
│   │   │   ├── Api.ets                     # 网络层（user/self + by-model）
│   │   │   ├── Store.ets                   # RDB 快照 + Preferences 配置
│   │   │   └── Secret.ets                  # HUKS 加密
│   │   └── common/
│   │       ├── Theme.ets                   # 设计常量
│   │       └── BreakpointSystem.ets        # 断点系统（mediaquery，sm/md/lg）
│   ├── resources/
│   └── module.json5                        # ohos.permission.INTERNET、页面路由、deviceTypes: phone/tablet/2in1
├── build-profile.json5                     # API 23（HarmonyOS 6）、签名配置
└── oh-package.json5
```

边界规则：

- 页面层只做 UI 与状态，经 model 层异步函数访问网络 / 存储，禁止直接调用系统 API 做数据访问。
- `Api.ets` 对应桌面版 `api.rs`：同请求头（UA / Accept / x-company / Cookie）、同金额口径（`QUOTA_PER_YUAN = 500_000`）、同错误分级、保留"日志失败仍返回额度"容错。
- `Store.ets` 对应桌面版 `db.rs`：快照表 schema 逐字段一致（含 `request_count`），子项目 1 即开始写入，为子项目 2 积累数据。
- `Secret.ets` 对应桌面版 `crypto.rs`：DPAPI → HUKS，`huks:v1:<base64>` 存储格式，兼容旧明文解密。
- 所有 ArkTS API 细节实施时以 `devecocli docs search/read` 查证为准，不凭记忆编码。

导航流：EntryAbility 启动 → 读凭据 → 有效 Cookie 进 DashboardPage，否则 ConfigPage；ConfigPage 双路径（登录按钮 → LoginPage；手动粘贴表单）；LoginPage 成功回退并跳看板；看板认证错误回退 ConfigPage。

## 数据层

### 配置（Preferences）

- `@ohos.data.preferences`，库名 `config`
- 键：`cookie`（加密串）、`api_key`（加密串，可选）、`cookie_source`（`webview` | `manual`）
- Cookie 不设本地过期，失效由服务端判定（与桌面版现行策略一致）

### 快照（RDB）

- `@ohos.data.relationalStore`，库名 `amax_dashboard.db`

```sql
CREATE TABLE IF NOT EXISTS dashboard_snapshot (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    saved_at TEXT NOT NULL,          -- 本地时区 RFC3339
    today_yuan REAL NOT NULL,
    total_tokens INTEGER NOT NULL,
    remaining REAL NOT NULL,
    used REAL NOT NULL,
    total REAL NOT NULL,
    request_count INTEGER
);
```

- `saved_at` 本地时区 RFC3339 写入；RDB 底层 SQLite 的 `date()` 语义与桌面版一致，子项目 2 按日查询沿用 `date(saved_at, 'localtime')` 口径（规避桌面版曾出现的时区归属缺陷）。
- 每次刷新写入一条快照（含 `request_count` 累计值）。

### 凭据加密（Secret.ets）

- `@ohos.security.huks`，AES-256-GCM，密钥别名 `amax_secret_key`
- 首次使用生成密钥：`AES | KEY_SIZE_256 | PURPOSE_ENCRYPT | PURPOSE_DECRYPT`，GCM 模式、`PADDING_NONE`（流式模式无需填充），密钥驻留 HUKS 不可导出，绑定本设备本应用沙箱（对应 DPAPI"当前用户 + 本机"定位）
- 加密：随机 12 字节 IV → GCM 加密（密文 || 16 字节 tag）→ `iv || 密文 || tag` 整体 Base64 → `huks:v1:<base64>`
- 解密：校验前缀 → 拆分 IV / 密文+tag → GCM 解密；无 `huks:v1:` 前缀的旧明文原样返回
- 加解密失败抛错，不静默吞错；调用方视为无有效凭据，引导重新登录
- HUKS GCM 参数（`HUKS_TAG_NONCE` 等）实施时经 `devecocli docs search "huks AES GCM"` 查证

## 网络层（Api.ets）

- `@ohos.net.http`；`BASE_URL = "https://ai.amaxsmp.com"`；`QUOTA_PER_YUAN = 500_000`；超时 30 秒
- 公共请求头：`User-Agent: Mozilla/5.0`、`Accept: application/json`、`x-company: AMAX`、`Cookie: <凭据>`
- `fetchDashboard(cookie) → DashboardData`（字段同桌面版：display_name / request_count / today_yuan / today_tokens / today_input / today_output / remaining / used / total / percent / logs_available）：
  1. `GET /api/user/self` → 额度、user_id、累计请求数
  2. `POST /v1/logs/token-usage/by-model`，body：本地时区当日 `start_time` / `end_time`（Unix 秒）、字符串 `user_id`、`status: "success"` → `summary` 聚合
  3. 日志请求失败仍返回额度数据，`logs_available = false`
- 错误分级：401/403 → "Cookie 无效或已过期，请重新登录"；429 → "请求过于频繁，请稍后重试"；连接失败 → "网络连接失败，请检查网络"

## WebView 登录流程（LoginPage.ets）

```
ConfigPage ──[前往登录]──> LoginPage（Web 组件加载 /login）
                              │
                              ├─ URL 进入 /dashboard → 登录成功
                              │     └─ WebCookieManager.fetchCookie(官网域)
                              │        → 正则取 session=([^;]+)
                              │        → Secret 加密存储（cookie_source=webview）
                              │        → 回退 ConfigPage → 跳转 DashboardPage
                              ├─ 页面加载失败 → 就地提示 + 重试按钮
                              └─ 用户手动返回 → 回 ConfigPage，不保存任何凭据
```

- 成功判据：URL 跳转至 `/dashboard`（官网登录后重定向行为）。
- 提取不到 `session` 视为登录未完成，不保存。
- ConfigPage 提供"重新登录"入口：清除凭据后重新进入登录流程。
- 风险：官网登录页在 HarmonyOS WebView 内的兼容性需真机实测；`WebCookieManager` API 签名经 `devecocli docs` 查证。

## UI 与视觉规范

### 设计常量（Theme.ets，桌面版色板逐值移植）

| 常量 | 值 | 用途 |
|------|-----|------|
| surface | #e8ece8 | 页面背景 |
| surfaceRaised | #f2f4f1 | 卡片底 |
| ink / muted | #19221e / #5a6460 | 正文 / 次级文字 |
| line | #bec9c1 | 分隔线 / 边框 |
| accent / accentSoft | #2c6b4b / #d2e2d7 | 主色（深绿） |
| error / errorBg | #8b3e3e / #eadada | 错误态 |
| fontMono / fontSans | monospace / 系统默认 | 数字 mono，正文 sans |

网格背景：Canvas 组件绝对定位铺满页面，`onReady` 绘制 24px 间距网格线（横线 `rgba(44,107,75,0.045)`、纵线 `0.035`），色值与桌面版一致。

### DashboardPage

- 顶栏：`AMAX / DAILY` + 同步状态（相对时间）+ 刷新 / 设置按钮
- 账户头：用户名 + 累计请求数
- COST TODAY：大数字（mono，¥ 6 位小数）+ 日志状态注记
- TOKENS 网格：两列（TOKENS / INPUT÷OUTPUT），M/K 单位格式化
- AVAILABLE QUOTA：百分比 + 进度条 + 剩余 / 已用 / 总额
- 交互：Refresh 下拉刷新（移动端增强）+ 顶栏刷新按钮；`onPageShow` 回到前台自动刷新（距上次刷新不足 60 秒则跳过，避免页面往返产生冗余请求）
- 认证错误 → 回退 ConfigPage；其他错误 → 错误横幅 + 保留已有数据

### ConfigPage

- `AMAX / ACCESS` + "连接你的 AMAX 账户" + 本地存储说明
- 主按钮"前往登录"（WebView 流程）
- 备用区：Cookie 手动粘贴（TextArea）+ API Key 可选 + "保存并验证"
- 已登录态显示"重新登录"（清凭据）入口
- 状态横幅（成功 / 失败 / 忙碌态）

### 多设备自适应（BreakpointSystem，手机 / 折叠屏 / 平板）

- 断点系统采用官方推荐模式：`@ohos.mediaquery` 监听窗口宽度，三档断点写入 AppStorage：
  - `sm`：< 600vp（手机竖屏）
  - `md`：600–840vp（折叠屏展开、平板竖屏）
  - `lg`：≥ 840vp（平板横屏、2in1）
- `BreakpointSystem.ets` 提供单例注册/注销监听；页面与组件经 `@StorageProp('currentBreakpoint')` 订阅，折叠屏开合、旋转、分屏等窗口尺寸变化自动触发布局更新。
- **自适应策略（KISS）**：内容区最大宽度约束 + 边距随断点递增，不做多列流重排——
  - `sm`：内容满宽，页面左右边距 24vp（对齐桌面版 520px 窗口的排版密度）
  - `md` / `lg`：内容区最大宽度 720vp 水平居中，左右边距 40vp；大数字字号随断点适度放大
- `module.json5` 的 `deviceTypes` 配置为 `["phone", "tablet", "2in1"]`。
- 子项目 2 的统计页直接复用该断点框架（图表容器宽度经断点约束，Chart 布局天然适配）。

## 测试与验证

### 本地单元测试（entry/src/test/，无需设备）

- `parseSessionCookie`：整串提取 session；无 session / 空串 / 多 Cookie 场景
- 金额换算：`quota / QUOTA_PER_YUAN`，含 0 与大额边界
- Token 格式化：M/K 单位切换
- 错误分级：401/403/429/网络错误 → 文案映射

### 仪器测试（entry/src/ohosTest/，模拟器或真机）

- HUKS 加解密往返：加密 → 解密 == 原文；旧明文兼容；篡改密文解密失败抛错
- RDB 快照：写入 → 查询行数与字段一致；建表幂等

### 构建与 GUI 实测

- `devecocli check lint` 零错误；`devecocli build` 编译通过
- 前置用户协作：`devecocli auth login`（华为开发者账号 OAuth）→ 真机（hdc）或模拟器（首次需下载 phone 镜像 + 接受协议）→ `devecocli signature generate` 自动签名
- `devecocli run` 部署；`devecocli ui screenshot` / `ui layout` 验证视觉与结构
- GUI 清单：WebView 登录提取 Cookie → 看板真实数据 → 下拉刷新 → 认证错误回退 → 手动粘贴路径 → 重新登录清凭据

## 风险与前置项

| 项 | 说明 | 处置 |
|----|------|------|
| 华为开发者账号 | 签名与真机调试必需 | 实施前用户执行 `devecocli auth login` |
| 官网登录页 WebView 兼容性 | 验证码 / JS 行为未知 | 实施早期真机实测，失败则降级为仅手动粘贴 |
| HUKS / WebCookieManager API 细节 | 参数签名不可凭记忆 | `devecocli docs` 逐项查证 |
| 模拟器镜像 | 首次下载约 30 分钟 + 协议接受 | 优先真机；无真机则用户配合镜像下载 |
| 折叠屏 / 平板实测设备 | 断点布局需多形态验证 | `devecocli emulator` 提供 phone / foldable / tablet 镜像可覆盖，无实体设备依赖 |

## 范围外（本子项目明确不做）

- 统计页、数据导出、低余额通知、后台定时刷新（子项目 2）
- HarmonyOS 应用市场上架（调试签名为准）
- 穿戴 / 智慧屏等设备形态（仅 phone / tablet / 2in1）
