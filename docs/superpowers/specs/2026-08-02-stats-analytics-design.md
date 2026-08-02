# 数据统计分析功能设计

日期：2026-08-02
状态：已评审（含 2026-08-02 API 实测与三轮范围修订），待实施

## 背景与目标

AMAX Dashboard 当前仅展示当日消耗与账户额度。`dashboard_snapshot` 表随每次刷新（手动 + 每 10 分钟后台）积累历史数据，但从未被消费。本功能新增独立统计页，提供：

1. **消耗趋势**：每日费用 / Token 曲线（官网 API 权威数据），可叠加本地快照对比数据集
2. **请求数趋势**：每日新增请求数曲线（本地快照累计值差分，官方 `daily` 无此字段）
3. **模型分布**：所选区间内各模型费用 / Token 占比与请求数（官网 by-model 接口）
4. **区间汇总**：累计费用、日均费用、单日峰值、有使用天数、区间总请求数
5. **余额趋势**：剩余额度历史曲线（本地快照，官方接口不提供该维度）
6. **数据导出**：当前区间的趋势明细、模型明细与汇总指标（CSV / JSON / Excel 三格式）
7. **本地降级**：官方 API 失败时，趋势与汇总回退到本地快照聚合（标注"本地估算"）

## 关键决策（已与用户确认）

| 决策点 | 结论 |
|--------|------|
| 统计维度 | 消耗趋势 + 请求数趋势 + 模型分布 + 区间汇总 |
| 页面组织 | 独立第三屏（config / dashboard / stats），dashboard 顶栏加入口 |
| 时间口径 | 预设 7 / 14 / 30 天快捷按钮 **+ 起止日期选择器**（任意区间，含超过 30 天的长区间），默认 7 天，全部视图跟随 |
| 图表渲染 | Chart.js 本地引入（`dist/vendor/chart.umd.js`，CSP 禁止外部资源） |
| 数据流 | 双 command 分离：官方数据为主、本地数据为辅（对比 + 降级回退），独立容错 |
| 本地数据生命周期 | 快照**永久保留**，废除现有 90 天清理逻辑；超 30 天的历史数据不主动展示，自定义长区间时按需查询 |
| 数据准确性 | **Token 统计以官方 API 为主基准**；金额口径不变（`quota / QUOTA_PER_YUAN`），官方 quota 为 0 时如实展示 |
| 请求数粒度 | 区间级（模型分布 / 汇总卡）+ **每日曲线**（快照 `request_count` 累计值日末差分，缺失日留空） |
| 本地聚合定位 | 双重角色：① 官方失败时的**降级回退源**（标注"本地估算"）；② 正常态下趋势图**可开关的对比数据集**（默认隐藏） |
| 数据导出 | CSV + JSON + Excel（SheetJS 本地引入）三格式 |

## API 实测结果（2026-08-02，user_id=105，7 天范围）

`POST /v1/logs/token-usage/by-model` 实际响应结构（原生提供每日 × 每模型明细）：

```json
{
  "start_time": 1785081600,
  "end_time": 1785686399,
  "status": "success",
  "summary": {
    "input_tokens": 88062962,
    "output_tokens": 1873797,
    "completion_tokens": 1873797,
    "total_tokens": 89936759,
    "quota": 0.0
  },
  "models": [
    {
      "model": "qwen3.8-max-preview",
      "request_count": 2003,
      "input_tokens": 88062962,
      "output_tokens": 1873797,
      "completion_tokens": 1873797,
      "total_tokens": 89936759,
      "quota": 0.0
    }
  ],
  "daily": [
    {
      "date": 1785081600,
      "model": "qwen3.8-max-preview",
      "input_tokens": 5276286,
      "output_tokens": 358085,
      "completion_tokens": 358085,
      "total_tokens": 5634371,
      "quota": 0.0
    }
  ]
}
```

要点：

- `daily` 按"日 × 模型"展开，`date` 为本地时区当日 0 点的 Unix 时间戳；**无使用量的日期无记录**（非 0 值记录）。
- `models` 含 `request_count`（区间请求数）。
- `summary` 不含 `request_count`，区间总数由 Σ `models[].request_count` 派生。
- `daily` **不含** `request_count`——每日请求数曲线只能由本地快照差分派生。
- 实测 `quota` 全为 0.0（当前模型免费）——是真实数据，不是接口异常。

## 架构与数据流

官方数据为权威主源，本地快照提供余额、请求数、对比与降级数据：

```
stats-screen ──┬─ invoke fetch_usage_stats(start, end) ──→ api.rs（官网 by-model）
               │     → 每日消耗趋势（费用/Token）+ 区间汇总 + 模型分布
               │     失败时 ──┐
               └─ invoke get_local_stats(start, end) ──→ db.rs（纯 SQLite）
                     → 余额趋势（始终）
                     → 每日请求数（累计差分，始终）
                     → 本地对比数据集（正常态，默认隐藏）
                     → 本地估算趋势/汇总（降级回退，官方失败时启用）
```

沿用现有模块边界，不新增 Rust 文件：

- **`api.rs`**：新增 `fetch_usage_stats(client, cookie, start_date, end_date)`，复用现有 by-model 请求模式（本地时区时间戳、`status=success`、字符串 `user_id`），时间范围由参数给定（`start` 00:00:00 至 `end` 23:59:59）。扩展 `UsageResponse` 解析 `models` / `daily` 数组。新增纯函数 `aggregate_usage(response, start_date, end_date)`：按日期聚合 `daily`（跨模型求和）、补齐无记录日为 0、派生区间汇总、计算模型占比。
- **`db.rs`**：
  - `save_snapshot` 增写 `request_count` 参数（账户累计请求数，来自 `user/self`）。
  - `Db::open` 增加轻量迁移：`dashboard_snapshot` 缺 `request_count` 列时 `ALTER TABLE ADD COLUMN request_count INTEGER`（旧记录为 NULL）。
  - 新增 `get_daily_snapshots(start_date, end_date)`：按本地日期分组取每天最后一条快照，返回 `remaining` / `yuan` / `tokens` / `request_count`。
  - 新增纯函数 `derive_daily_requests(rows)`：每日新增 = 当日前一条快照累计值差分；前一记录缺失或为 NULL（老数据）则该日 `None`（图表留空）。
  - 快照全部字段保留写入与存档，表结构仅新增列。
- **`lib.rs`**：注册 `fetch_usage_stats`、`get_local_stats` 两个 command；日期参数经纯函数校验；`refresh_dashboard` 保存快照时传入 `user.request_count`。

### 日期参数校验（`lib.rs` 纯函数）

- 格式 `YYYY-MM-DD`，可解析为合法日期。
- `start_date <= end_date`，`end_date <= 今天`（本地时区）。
- 区间跨度上限 **1096 天**（约 3 年），超出返回错误提示——保护官方 API 与本地查询，也覆盖可预见的长区间需求。

## Command 契约

```jsonc
// fetch_usage_stats —— 异步 command，网络请求，权威数据
// 入参：{ "start_date": "2026-07-27", "end_date": "2026-08-02" }
{
  "daily": [
    // 按日期升序，完整覆盖区间每一天（无使用量的日补 0，保证趋势连续且 Token 统计精确）
    { "date": "2026-07-28", "yuan": 0.0, "tokens": 5634371,
      "input_tokens": 5276286, "output_tokens": 358085 }
  ],
  "models": [
    // 按主指标降序（见占比规则）
    { "model": "qwen3.8-max-preview", "yuan": 0.0, "tokens": 89936759,
      "request_count": 2003, "percent": 100.0 }
  ],
  "summary": {
    "total_yuan": 0.0,
    "avg_yuan": 0.0,
    "peak_yuan": 0.0,
    "peak_date": "2026-08-02",
    "total_tokens": 89936759,
    "request_count": 2003,
    "days_with_usage": 5
  },
  "percent_basis": "tokens"   // "quota" | "tokens"，供前端标注占比口径
}

// get_local_stats —— 同步 command，纯 DB，辅助 / 对比 / 降级数据
// 入参：{ "start_date": "2026-07-27", "end_date": "2026-08-02" }
{
  "daily": [
    // 仅有快照的日期，缺失日不补（余额是状态量，插值无意义）
    // yuan / tokens 为当日最后一条快照的累计值（对比 + 降级估算用）
    // new_requests 为累计差分结果，null = 前日无记录或老数据（图表留空）
    { "date": "2026-08-01", "remaining": 86.2, "yuan": 14.01,
      "tokens": 26072553, "new_requests": 312 }
  ],
  "summary": {
    // 由 daily 的 yuan / tokens 派生（口径同官方汇总，供降级展示）
    "total_yuan": 88.4, "avg_yuan": 12.6, "peak_yuan": 21.3,
    "peak_date": "2026-07-29", "total_tokens": 130000000, "days_with_data": 7
  }
}
```

契约规则：

- `date` 统一为本地日期字符串（`YYYY-MM-DD`）；官方 `daily[].date` 的 Unix 时间戳在 Rust 侧转换。
- `yuan` 由 `quota / QUOTA_PER_YUAN` 换算，口径与看板一致，不变。
- **占比规则**：`percent` 默认按 `quota` 计算；当全部模型 `quota` 之和为 0 时，改按 `total_tokens` 计算，`percent_basis` 标明实际口径，前端据此展示"按 Token 占比"标注。
- `days_with_usage` = 官方 `daily` 中出现过的不同日期数（有真实使用量的天数）。
- 前端降级时：趋势 / 汇总切换为 `get_local_stats` 数据并在标题标注"（本地估算）"；模型分布块无本地数据源，显示"官方数据不可用"。

## 聚合规则

### 官方 daily 聚合（`api.rs::aggregate_usage`，纯函数可单测）

1. 按 `date` 分组，跨模型求和 `quota` / `total_tokens` / `input_tokens` / `output_tokens`。
2. 补齐区间内无记录的日期为全 0（本地时区逐日遍历），保证趋势图连续、Token 累计精确。
3. 区间汇总：
   - 累计费用 = Σ yuan
   - 日均费用 = Σ yuan ÷ **days_with_usage**（有使用量的天数，非自然天数；无使用量时日均为 0）
   - 单日峰值 = max(日 yuan)，连带日期（全 0 时 peak_date 取 end_date）
   - 累计 Token = Σ tokens（与 summary.total_tokens 一致，不一致时 stderr 记录诊断日志）
   - 区间总请求数 = Σ `models[].request_count`
4. 模型占比：按上述占比规则计算，保留 1 位小数，`models` 按主指标降序。

### 本地快照聚合（`db.rs`）

```sql
SELECT date(saved_at, 'localtime') AS day, today_yuan, total_tokens, remaining, request_count
FROM dashboard_snapshot
WHERE id IN (SELECT MAX(id) FROM dashboard_snapshot
             GROUP BY date(saved_at, 'localtime'))
  AND day BETWEEN ?1 AND ?2        -- 起止日期字符串，含端点
ORDER BY day
```

- 每天取 **MAX(id)**（当日最后一次刷新的快照）。
- `summary` 在 Rust 侧由 daily 派生：total = Σ，avg = Σ ÷ days_with_data，peak 取 max(yuan) 连带日期。
- **请求数差分**（`derive_daily_requests` 纯函数）：`new_requests = 当日累计 − 前一记录累计`；差分基准是查询结果中的**前一行**（非自然日前一天），但仅当两行日期相邻时有效——日期不相邻（中间有缺档日）或前一行 `request_count` 为 NULL 时，结果为 `None`，避免把多日累计错误计入单日。
- 余额缺失日不补 0、不插值，折线图 `spanGaps: true` 连接缺口。

## 容错与错误处理

官方数据为主、本地为辅，两者完全独立：

- 前端 `Promise.allSettled` 并行调用两个 command。
- **`fetch_usage_stats` 失败（降级路径）**：消耗趋势 / 汇总切换为 `get_local_stats` 的 daily / summary 数据，块标题追加"（本地估算）"标注；模型分布块显示错误信息与重试按钮。本地数据也为空时显示空态。
- **`get_local_stats` 失败或为空**：余额趋势、请求数趋势、本地对比数据集全部不展示（隐藏对应块/数据集），不影响官方数据。
- 认证错误（401/403）复用现有消息文案，额外提示"可前往设置页重新获取 Cookie"。
- 429 与其他 HTTP 错误沿用 `fetch_dashboard` 的现有分级文案。
- 响应缺少 `models` / `daily` 字段（接口契约变更）：解析为默认空值，对应块显示"暂无数据"，stderr 记录诊断日志（延续现有"日志失败保留额度"的降级风格）。

## 前端结构

### 入口与导航

- `index.html` 新增 `stats-screen`（第三屏）；dashboard 顶栏 `topbar-actions` 新增图表 icon 按钮，样式同刷新/设置按钮。
- 统计页顶栏：`AMAX / ANALYTICS` 标题 + `CSV` / `JSON` / `XLSX` 三个导出按钮 + "返回看板"按钮。
- `showScreen` 扩展为三屏互斥；统计页入口仅在 dashboard 屏可见（配置页不可达）。

### 布局（520×680，整页滚动）

```
┌──────────────────────────────┐
│ AMAX / ANALYTICS [CSV][JSON][XLSX][返回]│  顶栏
│ [7天][14天][30天] [起]~[止]    │  预设按钮 + 起止日期 input[type=date]
├──────────────────────────────┤
│ 累计费用  日均费用             │  汇总卡片（5 项自适应网格，官方口径，
│ 单日峰值  有使用天数           │  降级时整体追加"（本地估算）"标注）
│ 区间总请求数                   │
├──────────────────────────────┤
│ 消耗趋势折线图                 │  官方费用(¥) + 官方Tokens 双 Y 轴（主轴）
│                               │  + 本地费用 / 本地Tokens 对比数据集
│                               │  （虚线，默认隐藏，图例点击开关）
├──────────────────────────────┤
│ 请求数趋势折线图（小）          │  每日新增请求数，差分为 null 的日留空
├──────────────────────────────┤
│ 模型分布环图 + 右侧列表        │  列表：名称 / 金额 / 占比 / 请求数
│                               │  quota 全 0 时标注"按 Token 占比"
├──────────────────────────────┤
│ 余额趋势折线图（小）           │  本地快照 remaining(¥)，spanGaps
│                               │  本地无数据时整块隐藏
└──────────────────────────────┘
```

### 交互规则

- 进入页面：默认 7 天区间，并行调用两个 command，各块独立渲染。
- 切换区间：预设按钮与日期选择器双向联动（点预设同步日期输入；手动改日期清除预设选中态），变更后两者均重新请求，图表原地 `chart.update()` 复用实例。
- 日期输入约束：`max` 为今天，`start > end` 时前端就地提示不发请求。
- Chart 实例持有引用，离开统计页时 `destroy()` 防泄漏；canvas 元素随页面常驻复用。
- 视觉语言复用现有 `telemetry-label` / 区块样式，不引入新设计概念。

### 数据导出

纯前端实现：由当前已加载数据生成，无需新 command。三个按钮独立触发，共享同一份数据快照（含数据来源标注 official / local_estimate）。

**CSV**

- 单文件分节：`[区间汇总]` / `[每日趋势]` / `[模型分布]` 三个标题行，各节独立表头与数据行，节间空行分隔（Excel/WPS 兼容）。
- 编码 UTF-8 带 BOM（防 Excel 中文乱码），Blob + `a[download]` 触发。
- 文件名：`amax-stats_<start>_to_<end>.csv`。
- 每日趋势字段：date, yuan, tokens, input_tokens, output_tokens, new_requests, remaining（后两者取自本地数据按日期合并，无则留空）。
- 模型分布字段：model, yuan, tokens, request_count, percent, percent_basis。
- 区间汇总：指标名, 值（纵向键值对，含数据来源标注）。

**JSON**

- 结构化直出，便于程序化处理：

```jsonc
{
  "app": "amax-dashboard",
  "exported_at": "2026-08-02T15:56:00+08:00",
  "range": { "start_date": "2026-07-27", "end_date": "2026-08-02" },
  "source": "official",              // "official" | "local_estimate"
  "summary": { /* 同 command 契约 */ },
  "daily": [ /* 消耗趋势明细，含 new_requests / remaining 合并 */ ],
  "models": [ /* 官方数据降级时为空数组 */ ],
  "percent_basis": "tokens"
}
```

- `JSON.stringify(data, null, 2)`，UTF-8（无 BOM），Blob `application/json`。
- 文件名：`amax-stats_<start>_to_<end>.json`。

**Excel（SheetJS）**

- 引入 `xlsx.full.min.js`（Apache-2.0，约 900KB）至 `dist/vendor/`，`<script>` 先于 `app.js` 加载。
- 标准 `.xlsx`：三个工作表 `区间汇总` / `每日趋势` / `模型分布`，字段与 CSV 一致（`XLSX.utils.json_to_sheet` + `book_append_sheet`）。
- 文件名：`amax-stats_<start>_to_<end>.xlsx`。
- 依赖提示：SheetJS 体积较大且需手动更新；选 full 单文件 UMD 版本避免构建步骤。

三格式导出按钮在无任何数据时禁用。

### Chart.js 引入

- 下载 `chart.umd.js`（MIT，约 200KB）至 `dist/vendor/chart.umd.js`，`<script>` 先于 `app.js` 加载。
- CSP `script-src 'self'` 天然允许本地脚本，无需改 CSP。

## 风险与前置项

| 项 | 说明 | 状态 |
|----|------|------|
| by-model 响应结构 | 已实测确认 `models` / `daily` / `request_count` 字段 | ✅ 已完成（2026-08-02） |
| quota 全为 0 | 当前模型免费所致，非异常；占比自动切换 Token 口径覆盖该场景 | 预期行为 |
| 快照早期数据稀疏 | 余额 / 请求数趋势初期点少；老记录 `request_count` 为 NULL，差分为空 | 预期行为，留空处理；数据随运行自然积累 |
| DB 迁移 | `ALTER TABLE ADD COLUMN` 为 SQLite 轻量操作，无数据重写 | 风险低，open 时幂等执行 |
| 官方 daily 时间戳时区 | 实测为本地时区 0 点，与请求参数同基准 | 已验证 |
| 超长区间性能 | 3 年上限内官方响应与本地查询均可接受；`saved_at` 已有索引 | 上限兜底 |
| 第三方依赖 | Chart.js（MIT ~200KB）+ SheetJS（Apache-2.0 ~900KB），均需手动更新 | 已告知维护成本 |

## 测试策略

### Rust 单元测试

- `api.rs`：
  - 以本次实测响应为样本新增解析测试（`models` / `daily` / `request_count` / `completion_tokens` 忽略）。
  - `aggregate_usage` 纯函数测试：跨模型按日聚合、无使用日补 0、区间汇总派生（含 request_count 求和）、quota 全 0 时占比切换 Token 口径（`percent_basis="tokens"`）、单模型场景 percent=100、补 0 后日期连续性。
- `db.rs`：
  - `get_daily_snapshots`：同日多条仅取末条、起止日期边界（含端点）、空表返回空数组。
  - `derive_daily_requests`：相邻日正常差分、缺档日返回 None、前行为 NULL 返回 None、首行返回 None。
  - 迁移幂等性：对含 / 不含 `request_count` 列的表各 open 一次均成功。
- `lib.rs`：日期校验纯函数——合法区间通过、start>end 拒绝、未来日期拒绝、超 1096 天拒绝、非法格式拒绝。

### 前端与集成

- `node --check dist/app.js` 语法检查。
- `cargo tauri dev` 真实 WebView2 验证（项目规范强制）：看板→统计页导航、预设与日期选择器联动、超长区间查询、对比数据集开关、请求数曲线留空渲染、占比口径标注、降级路径（断网下本地估算标注与模型块错误态）、余额块空态隐藏、CSV / JSON / XLSX 导出内容与文件名正确性。

## 文档同步

- `CLAUDE.md`：
  - api.rs 段落补充 by-model 响应含 `models` / `daily` 明细与 `request_count`。
  - db.rs 段落删除"清理 90 天前记录"，改为快照永久保留、新增 `request_count` 列与迁移说明。
  - 前后端边界段落补充两个新 command 与日期参数契约。
  - 数据与界面流程段落补充统计页入口、"官方为主、本地对比/降级"容错与三格式导出说明。

## 范围外（明确不做）

- 数据导出为 PDF 等其他格式（仅 CSV / JSON / Excel）。
- 每日请求数走官方数据源（接口不提供，已确认；本地差分为唯一可行路径）。
