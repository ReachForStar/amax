# HarmonyOS 二期实施计划（统计页 + 导出 + 通知 + 后台刷新）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 鸿蒙版实现统计页（趋势/模型分布/汇总/余额 + 区间选择）、CSV/JSON 导出、低余额通知、WorkScheduler 机会性后台刷新。

**Architecture:** model 层扩展（官方区间聚合 + 本地快照查询/差分）、Canvas 自绘图表组件、StatsPage 集成、notificationManager 通知、workSchedulerExtension 延迟任务。规则继承桌面版统计 spec（金额口径 500_000、补零、占比切换、'localtime' 日期口径、差分留空）。

**Tech Stack:** ArkTS/ArkUI、@kit.NetworkKit、@kit.ArkData、@ohos.notificationManager、@ohos.resourceschedule.workScheduler、Canvas。

## Global Constraints

- 金额口径：`yuan = quota / 500_000`；占比 quota 全 0 切 tokens 口径（`percentBasis`）
- 官方 `daily` 按本地时区 Unix 时间戳解析；无使用日补 0；区间上限 1096 天
- 本地快照查询 `date(saved_at, 'localtime')`；请求数差分仅相邻日（其余留空）
- 路由一律 `this.getUIContext().getRouter()`；导航 try/catch
- 提交信息中文祈使句；`git add` 仅具体文件；不 push
- **节奏要求：轮询/重试等待一律 2-5 秒短间隔，跳过非必要等待**
- 构建：`cd harmony && /c/nvm4w/nodejs/devecocli build`；本地测试：`D:/command-line-tools/bin/hvigorw.bat test -p module=entry -p product=default -p testType=LocalTest --no-daemon`
- 部署：`devecocli run --device 127.0.0.1:5555`（Pura 90）

---

### Task P1: model 层扩展与本地测试（TDD）

**Files:**
- Modify: `harmony/entry/src/main/ets/model/Api.ets`
- Modify: `harmony/entry/src/main/ets/model/Store.ets`
- Create: `harmony/entry/src/test/Usage.test.ets`
- Modify: `harmony/entry/src/test/List.test.ets`（注册）

**Interfaces:**
- Produces:
  - `Api.ets`: `interface UsageDaily/UsageModel/UsageSummary/UsageStats`；`aggregateUsage(rawDaily, rawModels, startDate, endDate): UsageStats`（纯函数）；`fetchUsageStats(cookie, startDate, endDate): Promise<UsageStats>`
  - `Store.ets`: `interface DailySnapshot`；`getDailySnapshots(start, end): Promise<DailySnapshot[]>`；`deriveDailyRequests(snapshots): (number|null)[]`（纯函数）；`summarizeLocal(snapshots)`（纯函数）
  - `Api.ets` 现有 `todayRange` 重构为 `dateRangeToUnix(startDate, endDate)`（纯函数，供测试）

- [ ] **Step 1: 写失败测试**

`harmony/entry/src/test/Usage.test.ets`：

```typescript
import { describe, it, expect } from '@ohos/hypium';
import { aggregateUsage, dateRangeToUnix } from '../main/ets/model/Api';
import { deriveDailyRequests, summarizeLocal, DailySnapshot } from '../main/ets/model/Store';

// rawDaily 元素：{ date: 本地 0 点 Unix 秒, model, quota, total_tokens, input_tokens, output_tokens }
function ts(dateStr: string): number {
  const parts = dateStr.split('-');
  return Math.floor(new Date(Number(parts[0]), Number(parts[1]) - 1, Number(parts[2]), 0, 0, 0, 0).getTime() / 1000);
}

export default function usageTest() {
  describe('aggregateUsage', () => {
    it('fillsMissingDaysAndMergesModels', 0, () => {
      const d1 = ts('2026-08-01');
      const d3 = ts('2026-08-03');
      const rawDaily = [
        { date: d1, quota: 1_000_000, total_tokens: 200, input_tokens: 150, output_tokens: 50 },
        { date: d1, quota: 500_000, total_tokens: 100, input_tokens: 60, output_tokens: 40 },
        { date: d3, quota: 500_000, total_tokens: 100, input_tokens: 80, output_tokens: 20 },
      ];
      const rawModels = [
        { model: 'm-a', quota: 1_500_000, total_tokens: 300, request_count: 10 },
        { model: 'm-b', quota: 500_000, total_tokens: 100, request_count: 4 },
      ];
      const stats = aggregateUsage(rawDaily, rawModels, '2026-08-01', '2026-08-03');
      expect(stats.daily.length).assertEqual(3);
      expect(stats.daily[0].yuan).assertClose(3.0, 0.000001);   // 跨模型求和
      expect(stats.daily[0].tokens).assertEqual(300);
      expect(stats.daily[1].yuan).assertEqual(0);               // 补零
      expect(stats.summary.totalYuan).assertClose(4.0, 0.000001);
      expect(stats.summary.daysWithUsage).assertEqual(2);
      expect(stats.summary.avgYuan).assertClose(2.0, 0.000001);
      expect(stats.summary.peakYuan).assertClose(3.0, 0.000001);
      expect(stats.summary.peakDate).assertEqual('2026-08-01');
      expect(stats.summary.requestCount).assertEqual(14);
      expect(stats.percentBasis).assertEqual('quota');
      expect(stats.models[0].model).assertEqual('m-a');
      expect(stats.models[0].percent).assertClose(75.0, 0.01);
    });
    it('switchesToTokensBasisWhenQuotaZero', 0, () => {
      const rawDaily = [{ date: ts('2026-08-01'), quota: 0, total_tokens: 400, input_tokens: 0, output_tokens: 0 }];
      const rawModels = [
        { model: 'm-a', quota: 0, total_tokens: 300, request_count: 3 },
        { model: 'm-b', quota: 0, total_tokens: 100, request_count: 1 },
      ];
      const stats = aggregateUsage(rawDaily, rawModels, '2026-08-01', '2026-08-01');
      expect(stats.percentBasis).assertEqual('tokens');
      expect(stats.models[0].percent).assertClose(75.0, 0.01);
    });
    it('emptyRangeAllZero', 0, () => {
      const stats = aggregateUsage([], [], '2026-08-01', '2026-08-02');
      expect(stats.daily.length).assertEqual(2);
      expect(stats.summary.daysWithUsage).assertEqual(0);
      expect(stats.summary.peakDate).assertEqual('2026-08-02');
    });
  });
  describe('deriveDailyRequests', () => {
    it('adjacentGapAndNull', 0, () => {
      const snaps: DailySnapshot[] = [
        { date: '2026-08-01', yuan: 0, tokens: 0, remaining: 0, requestCount: 100 },
        { date: '2026-08-02', yuan: 0, tokens: 0, remaining: 0, requestCount: 130 },
        { date: '2026-08-04', yuan: 0, tokens: 0, remaining: 0, requestCount: 200 },
        { date: '2026-08-05', yuan: 0, tokens: 0, remaining: 0, requestCount: null },
      ];
      const diff = deriveDailyRequests(snaps);
      expect(diff[0]).assertNull();
      expect(diff[1]).assertEqual(30);
      expect(diff[2]).assertNull();   // 缺档
      expect(diff[3]).assertNull();   // 前值为 null
    });
  });
  describe('summarizeLocal', () => {
    it('totalsAndPeak', 0, () => {
      const snaps: DailySnapshot[] = [
        { date: '2026-08-01', yuan: 2.5, tokens: 100, remaining: 90, requestCount: 1 },
        { date: '2026-08-02', yuan: 4.5, tokens: 300, remaining: 80, requestCount: 2 },
      ];
      const s = summarizeLocal(snaps);
      expect(s.totalYuan).assertClose(7.0, 0.000001);
      expect(s.avgYuan).assertClose(3.5, 0.000001);
      expect(s.peakYuan).assertClose(4.5, 0.000001);
      expect(s.peakDate).assertEqual('2026-08-02');
      expect(s.totalTokens).assertEqual(400);
    });
  });
  describe('dateRangeToUnix', () => {
    it('localTimezoneBoundaries', 0, () => {
      const r = dateRangeToUnix('2026-08-01', '2026-08-02');
      expect(r.end - r.start).assertEqual(2 * 86400 - 1); // 起 00:00:00 止 23:59:59
    });
  });
}
```

List.test.ets 注册 `usageTest()`。

- [ ] **Step 2: 运行测试确认失败**（编译失败即 RED 证据）

- [ ] **Step 3: 实现 Api.ets 扩展**

要点：
- `dateRangeToUnix(startDate, endDate)`：`new Date(y, m-1, d)` 本地构造（禁止 `new Date('YYYY-MM-DD')`，UTC 解析陷阱）；end 用 23:59:59。
- `aggregateUsage`：移植桌面版逻辑——按 date 分组求和（时间戳 → 本地日期串经 `new Date(ts*1000)` 取 y/m/d）、`Map` 逐日补零、summary 派生（avg 除 daysWithUsage、peak 全 0 回退 endDate、requestCount = Σ models）、percentBasis 切换（Σquota <= 0 → tokens）、percent 保留 1 位小数、models 按主指标降序。
- `fetchUsageStats(cookie, startDate, endDate)`：复用现有 request 封装与错误分级；POST by-model body `{ start_time, end_time, user_id, status: 'success' }`；解析 `data.daily` / `data.models`（注意官网响应顶层即 `{ summary, models, daily }`，无 data 包裹——与 user/self 不同）；经 aggregateUsage 返回。
- 日期解析失败/区间非法（start>end、跨度>1096 天）抛 ApiError。

- [ ] **Step 4: 实现 Store.ets 扩展**

```typescript
export async function getDailySnapshots(startDate: string, endDate: string): Promise<DailySnapshot[]> {
  const predicates = new relationalStore.RdbPredicates('dashboard_snapshot');
  // 子查询日末条：RdbPredicates 不支持子查询，用 executeSql + raw query
  const resultSet = await requireDb().querySql(
    `SELECT date(saved_at, 'localtime') AS day, today_yuan, total_tokens, remaining, request_count
     FROM dashboard_snapshot
     WHERE id IN (SELECT MAX(id) FROM dashboard_snapshot GROUP BY date(saved_at, 'localtime'))
       AND day BETWEEN ? AND ?
     ORDER BY day`,
    [startDate, endDate]);
  // 遍历 resultSet 组装数组，finally resultSet.close()
}
```

`deriveDailyRequests` / `summarizeLocal` 纯函数（逻辑同测试断言）。`querySql` API 形式经 `devecocli docs read` 查证（js-apis-data-rdb），不符则以文档为准。

- [ ] **Step 5: 运行测试确认通过**（全绿 + 原 10 例不回归）

- [ ] **Step 6: build 验证 + 提交**

```bash
git add harmony/entry/src/main/ets/model/Api.ets harmony/entry/src/main/ets/model/Store.ets harmony/entry/src/test/Usage.test.ets harmony/entry/src/test/List.test.ets
git commit -m "feat: 实现区间聚合与本地快照查询"
```

---

### Task P2: Canvas 图表组件

**Files:**
- Create: `harmony/entry/src/main/ets/common/LineChart.ets`
- Create: `harmony/entry/src/main/ets/common/DonutChart.ets`

**Interfaces:**
- Produces:
  - `LineChart` @Component：props `labels: string[]`、`series: ChartSeries[]`（ChartSeries: { name, color, data: (number|null)[], dashed? }）、`yMax?: number`（null 自适应）、`version: number`（@Watch 触发重绘）、双轴开关 `dualAxis: boolean`（series 前一半左轴 ¥、后一半右轴 Tokens）
  - `DonutChart` @Component：props `segments: { label, value, color }[]`、`centerText: string`、`version: number`

- [ ] **Step 1: LineChart.ets**

要点（Canvas 绘制规范）：
- `private settings = new RenderingContextSettings(true); private ctx = new CanvasRenderingContext2D(settings);`
- 绘制函数 `draw()`：清空 → 计算边距（左 48vp 轴标签、右双轴时 48vp、下 24vp）→ y 轴刻度 4 格（左轴 ¥ toFixed(2)、右轴 formatTokens）→ x 标签抽稀（最多 8 个）→ 网格线（Theme.gridH 横）→ 逐 series 折线（null 点断开，dashed 用 setLineDash([6,4])）→ 数据点圆点 r=2
- 触摸提示：`.onTouch` 记录最近数据点索引，`@State tooltipIndex` 变化触发重绘，命中点画竖虚线 + 顶部浮层文本（日期 + 各序列值）
- `@Watch('onVersion') onVersion() { this.draw(); }` + `.onReady(() => this.draw())`
- 坐标换算：Canvas 回调坐标单位为 vp，直接按组件 width/height 比例计算（context.width 即 vp）
- 空数据（labels 空）：居中绘制"暂无数据"（Theme.muted 12vp）

- [ ] **Step 2: DonutChart.ets**

要点：
- 环图：外半径 = min(w,h)/2 - 8，内半径 = 外 * 0.62；逐 segment 按比例画 arc（ctx.beginPath + arc + 填充扇环：外 arc 顺时针 + 内 arc 逆时针闭合）
- 中心文本：centerText（14vp mono，Theme.ink）
- value 总和为 0：画整环（Theme.line 色）+ 中心"暂无数据"
- 同样 @Watch version 重绘 + onReady 初绘

- [ ] **Step 3: build 验证 + 提交**

```bash
git add harmony/entry/src/main/ets/common/LineChart.ets harmony/entry/src/main/ets/common/DonutChart.ets
git commit -m "feat: 实现 Canvas 折线与环图组件"
```

---

### Task P3: 统计页主体

**Files:**
- Create: `harmony/entry/src/main/ets/pages/StatsPage.ets`
- Modify: `harmony/entry/src/main/resources/base/profile/main_pages.json`（注册 pages/StatsPage）

**Interfaces:**
- Consumes: P1（fetchUsageStats / getDailySnapshots / deriveDailyRequests / summarizeLocal / UsageStats）、P2（LineChart / DonutChart）、Theme / GridBackground / Format
- Produces: 完整统计页

- [ ] **Step 1: StatsPage.ets 结构**

页面骨架（@Entry @Component，断点 @StorageProp('currentBreakpoint')）：

```typescript
@Entry
@Component
struct StatsPage {
  @StorageProp('currentBreakpoint') bp: string = 'sm';
  @State rangeDays: number = 7;              // 预设选中，0 = 自定义
  @State startDate: string = '';             // YYYY-MM-DD
  @State endDate: string = '';
  @State rangeError: string = '';
  @State official: UsageStats | null = null;
  @State localSnapshots: DailySnapshot[] = [];
  @State degraded: boolean = false;          // 官方失败降级标记
  @State errorMsg: string = '';
  @State chartVersion: number = 0;           // 数据变化递增，触发图表重绘
  private refreshInFlight: boolean = false;
  ...
}
```

- [ ] **Step 2: 区间选择器与加载逻辑**

- 顶栏：`AMAX / ANALYTICS` + 导出按钮（P4 接线）+ 返回看板（router.back()）
- 区间行：三个预设按钮（7/14/30，选中态 Theme.accentSoft）+ 两个日期 TextInput（YYYY-MM-DD 手动输入；onChange 校验格式、start<=end、end<=今天、跨度<=1096，非法显示 rangeError 不加载）
- `applyPreset(days)`：startDate = daysAgoStr(days)（复用 DashboardPage 逻辑或迁移至 Format.ets）、endDate = todayStr()、rangeDays = days
- `loadStats()`：refreshInFlight 串行化（同看板）；并行两数据源：
  ```typescript
  const cookie = await getCookie() ?? '';
  try {
    this.official = await fetchUsageStats(cookie, this.startDate, this.endDate);
    this.degraded = false;
  } catch (e) {
    if (e instanceof ApiError && e.isAuthError) { 回退 ConfigPage; return; }
    this.degraded = true;
    this.errorMsg = ...;
  }
  this.localSnapshots = await getDailySnapshots(this.startDate, this.endDate); // try/catch 兜底空数组
  this.chartVersion++;
  ```
- onPageShow 触发首次加载（与看板同样 60s 节流可选，本页打开即加载，返回重进重新加载可接受）

- [ ] **Step 3: 汇总卡与四图表区块**

- 汇总卡：官方模式用 official.summary（累计/日均/峰值/有使用天数/区间总请求数）；降级模式用 summarizeLocal(localSnapshots) 并在标题加"（本地估算）"；请求数降级时显示"--"
- 消耗趋势：LineChart dualAxis；官方模式序列 [费用(官方), Tokens(官方)]；降级模式 [费用(本地估算, dashed), Tokens(本地估算, dashed)]；labels 官方补零日期串/降级快照日期串
- 请求数趋势：deriveDailyRequests(localSnapshots)，null 断开；全空隐藏整块
- 模型分布：DonutChart（segments = official.models 映射，颜色数组复用桌面版 MODEL_PALETTE 色值）+ 列表（ForEach models：名称/¥金额/占比%/请求数次，percentBasis==='tokens' 时列表头标注"按 Token 占比"）；降级或空 models 显示错误/空态
- 余额趋势：localSnapshots.remaining LineChart；空隐藏整块
- 降级横幅：degraded 时顶部显示 errorMsg + "官方数据不可用，趋势与汇总为本地估算"（Theme.error/errorBg）

- [ ] **Step 4: build 验证 + 提交**

```bash
git add harmony/entry/src/main/ets/pages/StatsPage.ets harmony/entry/src/main/resources/base/profile/main_pages.json
git commit -m "feat: 实现统计页主体"
```

---

### Task P4: 导出（CSV / JSON）

**Files:**
- Modify: `harmony/entry/src/main/ets/pages/StatsPage.ets`（导出按钮接线）
- Create: `harmony/entry/src/main/ets/common/Export.ets`（生成与保存）
- Modify: `harmony/entry/src/test/Usage.test.ets`（CSV 纯函数测试）

- [ ] **Step 1: 写失败测试（CSV 生成纯函数）**

```typescript
import { buildStatsCsv } from '../main/ets/common/Export';
// describe('buildStatsCsv')：
// - 含三节标题 [区间汇总]/[每日趋势]/[模型分布]
// - 逗号/引号/换行字段转义（双引号包裹 + 引号加倍）
// - 空 models 时模型节仅表头
```

- [ ] **Step 2: 实现 Export.ets**

```typescript
export function csvEscape(value: string | number | null): string { ... }
export function buildStatsCsv(range, summary, daily, models, percentBasis): string { ... }  // 同桌面版三节结构，CRLF 行尾
export function buildStatsJson(...): string { ... }  // JSON.stringify(data, null, 2)，data 结构同桌面版（app/exported_at/range/source/percent_basis/summary/daily/models）

// 保存：系统 picker 另存对话框
import { picker } from '@kit.CoreFileKit';
export async function saveTextFile(context: Context, fileName: string, content: string, mimeType: string): Promise<void> {
  // DocumentSaveOptions：new picker.DocumentSaveOptions(); options.newFileNames = [fileName];
  // const documentPicker = new picker.DocumentViewPicker(context);
  // const uris = await documentPicker.save(options);
  // uris 非空时经 fileIo 打开 uri 写入 content（fs.open(uri, ReadWrite) + fs.write + fs.close）
  // API 细节（DocumentSaveOptions 字段名、save 方法签名）经 devecocli docs read "API参考/文件管理/.../js-apis-file-picker" 查证
}
```

- [ ] **Step 3: StatsPage 导出接线**

导出按钮（顶栏"导出"点击展开 CSV / JSON 两选项，或两个并排小按钮——KISS 用并排）：点击后以当前已加载数据生成文本 → saveTextFile（csv: text/csv，json: application/json）；无数据（official 与 localSnapshots 均空）时按钮 disabled。保存成功/失败用页面内提示。

- [ ] **Step 4: 测试 + build + 提交**

```bash
git add harmony/entry/src/main/ets/common/Export.ets harmony/entry/src/main/ets/pages/StatsPage.ets harmony/entry/src/test/Usage.test.ets
git commit -m "feat: 实现统计数据 CSV 与 JSON 导出"
```

---

### Task P5: 低余额通知与后台刷新

**Files:**
- Modify: `harmony/entry/src/main/ets/entryability/EntryAbility.ets`（通知检查钩子）
- Modify: `harmony/entry/src/main/ets/pages/DashboardPage.ets`（刷新成功后调通知检查）
- Create: `harmony/entry/src/main/ets/workscheduler/RefreshWorkSchedulerExtension.ets`
- Modify: `harmony/entry/src/main/module.json5`（extensionAbilities 注册）

- [ ] **Step 1: 通知模块**

```typescript
// common/Notify.ets（或并入 EntryAbility）
import { notificationManager } from '@kit.NotificationKit';
let lowQuotaNotified = false;   // 启动周期内只发一次
export async function checkLowQuota(percent: number, remaining: number, total: number): Promise<void> {
  if (percent >= 10.0) { lowQuotaNotified = false; return; }
  if (lowQuotaNotified) { return; }
  try {
    const enabled = await notificationManager.isNotificationEnabled();
    if (!enabled) { return; }   // 未授权静默跳过
    await notificationManager.publish({
      id: 1,
      content: {
        notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
        normal: { title: 'AMAX 额度不足', text: `剩余 ¥${remaining.toFixed(2)} / ¥${total.toFixed(2)} (${percent.toFixed(1)}%)，请及时充值`, additionalText: '' },
      },
    });
    lowQuotaNotified = true;
  } catch (e) { console.error(`低余额通知失败：${...}`); }
}
```

DashboardPage.refresh() 成功后调用 `checkLowQuota(data.percent, data.remaining, data.total)`（API 名经 docs 查证）。

- [ ] **Step 2: WorkScheduler 延迟任务**

```typescript
// workscheduler/RefreshWorkSchedulerExtension.ets
import { WorkSchedulerExtensionAbility, workScheduler } from '@kit.BackgroundTasksKit';
export default class RefreshWorkSchedulerExtension extends WorkSchedulerExtensionAbility {
  onWorkStart(workInfo: workScheduler.WorkInfo): void {
    // 执行一次刷新：initStore(context) → getCookie → fetchDashboard → saveSnapshot → checkLowQuota
    // 完成后重新申请下一次：workScheduler.startWork({ workId: 100, bundleName: 'com.amax.dashboard', abilityName: 'RefreshWorkSchedulerExtension', isPersisted: false })
  }
  onWorkStop(workInfo: workScheduler.WorkInfo): void {}
}
```

- module.json5 注册 extensionAbilities（type: 'workScheduler'，srcEntry 对应路径）
- EntryAbility.onBackground（或 onWindowStageDestroy 前）申请 `workScheduler.startWork`；首次申请前 `workScheduler.stopWork` 防重复（API 细节 docs 查证）
- **明确约束写入注释**：延迟任务触发时机由系统决定（非精确周期），仅用于机会性后台快照积累

- [ ] **Step 3: build + 提交**

```bash
git add harmony/entry/src/main/ets/common/Notify.ets harmony/entry/src/main/ets/pages/DashboardPage.ets harmony/entry/src/main/ets/workscheduler/RefreshWorkSchedulerExtension.ets harmony/entry/src/main/module.json5 harmony/entry/src/main/ets/entryability/EntryAbility.ets
git commit -m "feat: 实现低余额通知与后台延迟刷新"
```

---

### Task P6: 看板入口、文档与部署验证

**Files:**
- Modify: `harmony/entry/src/main/ets/pages/DashboardPage.ets`（顶栏统计按钮）
- Modify: `harmony/README.md`、根 `CLAUDE.md`

- [ ] **Step 1: 看板顶栏加统计入口**

DashboardPage 顶栏（刷新/设置按钮旁）加"📈"文本按钮 → `this.getUIContext().getRouter().pushUrl({ url: 'pages/StatsPage' })`（try/catch）。

- [ ] **Step 2: 文档同步**

- README：路线图二期项移除（已实现），结构段加 StatsPage / LineChart / DonutChart / Export / Notify / RefreshWorkSchedulerExtension；开发命令不变
- 根 CLAUDE.md「HarmonyOS 版」段落补二期内容：统计页、导出、通知、WorkScheduler（注明机会性调度限制）

- [ ] **Step 3: 部署 + GUI 验证清单**

`devecocli run --device 127.0.0.1:5555`，逐项验证：
- [ ] 看板顶栏统计按钮 → 统计页
- [ ] 默认 7 天：汇总卡有数、消耗趋势（官方 quota 为 0 时费用全 0 正常）、模型分布环图 + Tokens 口径标注、请求数/余额趋势（有快照数据时）
- [ ] 切换 14/30 天与自定义区间（含非法输入提示）
- [ ] 导出 CSV / JSON（picker 保存，文件内容正确）
- [ ] 断网降级：横幅 + 本地估算标注
- [ ] 断点：Pura 90 正常；（可选）Mate X7 折叠展开布局
- [ ] 低余额通知（可临时把阈值调高验证后改回，或 percent<10 账户直接观察）
- [ ] 缺陷修复独立提交 `fix: <描述>`

- [ ] **Step 4: 提交**

```bash
git add harmony/entry/src/main/ets/pages/DashboardPage.ets harmony/README.md CLAUDE.md
git commit -m "feat: 统计入口与二期文档同步"
```