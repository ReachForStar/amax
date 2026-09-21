# 更新日志

本文件记录各版本的修改说明。发布新版本（推送 `v*` 标签）前，先在此文件顶部新增 `## [x.y.z] - YYYY-MM-DD` 小节；Release 工作流会自动提取对应小节作为 Release Notes，缺失时回退为提交列表。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，小节标题必须为 `## [版本号]` 形式。

## [0.2.3] - 2026-09-21

### 新增

- **桌面端官网 WebView 登录获取 Cookie**（获取方式对齐手机端）：配置页新增“使用官网登录获取”主按钮，点击后在应用内打开官网登录窗口，登录成功自动提取整串 Session Cookie（含 HTTP-only）并复用现有保存验证链路进入看板；关闭登录窗口视为取消，约 10 分钟未完成登录则超时提示
  - 新增后端 command `open_login_window` 与模块 `src-tauri/src/error.rs`、`src-tauri/src/login.rs`（基于 Tauri `WebviewWindow::cookies_for_url`，无新增依赖）
  - 新增事件：`login://success`（携带 Cookie 与服务端下发的到期时间）、`login://cancelled`、`login://timeout`
  - 手动粘贴保留为兜底路径，配置页文案相应降级
- **结构化错误契约**（桌面端）：所有 IPC command 的失败载荷由裸字符串改为 `{ code, message }`，`code` 为 `auth` / `network` / `data` / `input` / `storage` 五类；前端按 `code` 而非文案子串分支，网络故障不再被显示成“Cookie 失效”
- **认证失败一键重登**（两端）：凭据失效时统一落到配置页并直接指向登录按钮（桌面端聚焦“使用官网登录获取”，手机端提示“前往登录（推荐）”）；后台与启动定时刷新撞上失效时，桌面端广播 `auth://expired` 主动引导重登，不再让看板静默陈旧
- **凭据有效期展示**（两端，仅展示不拦截）：登录窗能读到官网下发的 `Expires` 时，配置页显示“凭据有效期至 …（官网下发，仅供展示）”，读不到则明示“官网未下发过期时间，失效由服务端判定”；手机端经 `WebCookieManager.fetchAllCookies`（API 23）读取到期属性
- 手机端登录页补齐 10 分钟等待看门狗（对齐桌面版），超时自动返回配置页并给出下一步提示；配置页补齐 `onPageShow` 重读凭据状态，从登录页手动返回后按钮状态即时正确
- **Test CI**：新增 `.github/workflows/test.yml`，push 到 master / PR / 手动触发时在 `windows-latest` 跑 fmt、`clippy -D warnings`、`cargo test`、前端语法检查与逻辑回归；`paths-ignore` 跳过纯文档与纯鸿蒙改动，同分支旧任务自动取消
- 新增共享检查步骤 `.github/actions/verify/action.yml`，Test CI 与 Release CI 调用同一份命令列表，避免“CI 拒绝什么”与“发布前检查什么”漂移

### 变更

- **Release CI 加固**：发布前先跑同一份检查；标签号与 `tauri.conf.json` 的 `version` 不一致时直接失败；`checkout` 改 `fetch-depth: 0`（原浅克隆下“上一标签以来的提交列表”回退取不到历史，只会输出空内容）；`CHANGELOG.md` 缺小节由静默回退改为显式告警；`tauri-cli` 固定版本并缓存 `cargo-tauri.exe`；手动运行改为上传 MSI artifact
- `crypto.rs::hex_decode` 由“先判偶数长度再 `chunks_exact(2)`”改为 `as_chunks::<2>()` + 余片判空，长度不变量与切分收敛到同一处表达，`clippy -D warnings` 基线随之转绿（CI 因此可以把它当真门槛）
- Cookie 有效期只由官网下发、只用于展示：手动粘贴路径保存时会清掉上一次登录留下的有效期，避免展示与当前凭据不匹配的时间
- 手机端跨页认证提示统一收进 `AppStorage` 的 `authNotice` 单通道（登录超时、看板失效、凭据无法解密共用），由配置页一次性消费

### 修复

- 修复统计页 Cookie 失效被静默降级：`/v1/logs/token-usage/by-model` 返回 401/403 时原实现被包成“用量查询失败: …”，不带认证语义，导致统计页只在本地估算上标注一句而不引导重登；现按状态码统一分级为认证错误
- 修复 429 与 5xx 被当成认证错误：原桌面端把所有非 2xx 都冠以“认证失败”，服务端限流或故障会把用户无端踢回配置页；现 `429`/`5xx` 归为可重试的网络错误

## [0.2.2] - 2026-08-14

### 新增

- **自动更新**：启动 15 秒后自动检查 + 托盘「检查更新」菜单，从 GitHub Releases 拉取更新清单，发现新版本即下载安装并自动重启（debug 构建跳过）；发布产物含签名文件（.sig）与更新清单（latest.json）
- **单实例保护**：二次启动唤醒已有实例窗口，不再重复创建托盘与后台定时器
- **日志落盘**：统一走 tauri-plugin-log（stdout + 应用日志目录 amax.log，5MB 轮转），替换原有 eprintln
- **看板加载提速**：缓存 user_id 后账户信息与当日用量并行拉取，省一个网络往返；Cookie 更换自动失效回退
- 启动时显式申请系统通知权限（Windows 下恒为已授权，防平台差异）

### 变更

- 后台刷新失败后 60 秒起快速重试（封顶 5 分钟，成功复位为 10 分钟间隔）
- 快照表新增 `day` 本地日期列并建索引，统计页按日查询改为索引范围扫描（旧库幂等迁移回填；不再使用 `date(...,'localtime')` 表达式——SQLite 视为非确定性函数禁止入索引）
- Chart.js / SheetJS 改为进入统计页 / 首次 XLSX 导出时按需注入，启动不再同步解析约 1.1MB 第三方脚本
- tokio 特性裁剪（`full` → `sync/time/macros`）、chrono 移除未使用的 serde 特性
- CI 缓存 tauri-cli 安装（约省 5–10 分钟），发布流程支持 updater 签名（secret 名称 `TAURI_SIGNING_PRIVATE_KEY`）
- 前端自定义区间补齐 1096 天跨度校验，与后端口径一致

### 修复

- 统计页快速切换区间时，过期响应覆盖新区间数据（加载序号丢弃过期结果，与 Harmony 版行为对齐）
- 看板当日请求数由占位文案改为真实值（by-model 各模型 request_count 求和）
- 启动重复刷新：窗口可见启动时前端已刷新，后端 3 秒兜底任务检测到近期成功刷新即跳过
- 修复日志汇总时间戳换算失败时仍发起无效查询的问题

## [0.2.1] - 2026-08-02

### 新增

- **统计分析页**（看板顶栏图表按钮进入）：
  - 消耗趋势：每日费用 / Token 曲线（官网 API 权威数据，逐日补零），可叠加本地快照对比数据集（默认隐藏，图例开关）
  - 请求数趋势：每日新增请求数（本地快照累计值差分，缺档日留空）
  - 模型分布：环图 + 列表，展示各模型费用 / Token 占比与区间请求数；计费 quota 全为 0 时占比自动切换 Token 口径并标注
  - 区间汇总：累计费用、日均费用、单日峰值（连带日期）、有使用天数、区间总请求数
  - 余额趋势：剩余额度历史曲线（本地快照）
- **区间选择器**：预设 7 / 14 / 30 天快捷按钮 + 自定义起止日期（跨度上限 1096 天）
- **数据导出**：当前区间数据导出为 CSV（UTF-8 BOM，Excel 兼容）/ JSON（结构化）/ XLSX（三工作表）
- 官方接口失败时趋势与汇总自动降级为本地估算并标注，模型分布独立显示错误提示
- Token 显示统一为百万单位（M）
- 新增后端 command：`get_local_stats`（本地快照每日查询 + 差分）、`fetch_usage_stats`（官网 by-model 区间聚合）

### 变更

- Session Cookie 不再设本地过期时间（原为保存后固定 15 天过期），实际失效改由服务端判定（API 返回认证错误时回退配置页）
- 刷新快照永久保留（原为清理 90 天前记录）；快照表新增 `request_count` 累计字段（旧库自动幂等迁移）
- 引入 Chart.js 4.4.7（MIT）与 SheetJS 0.20.3（Apache-2.0），以 UMD 单文件存放于 `dist/vendor/`，无构建步骤、CSP 不变

### 修复

- 修复快照按日查询的日期归属缺陷：SQLite `date()` 对带时区偏移的时间串先归一 UTC 再截取，导致 UTC+8 下凌晨 0–8 点的快照被计入前一天、本地估算系统性偏少；现统一使用 `date(saved_at, 'localtime')`，历史数据归属一并修正
