// 统计页：区间选择器（预设 7/14/30 天 + 自定义起止日期）驱动官方聚合与本地快照展示
// 官方失败降级本地估算：趋势与汇总切换本地数据并标注，模型分布仅官方可用
// 偏差记录（与看板页同台账）：
// 1. 路由一律使用 this.getUIContext().getRouter() 实例而非静态 router：静态接口自 API 18 废弃，
//    异步回调中调全局 router 会闪退（faqs-arkui-388），导航调用补 try/catch。
// 2. getCookie 在密文损坏时会 reject（decryptSecret 抛错）：兜底视为无效凭据 → 与认证错误同路径回 ConfigPage。
import { Theme } from '../common/Theme';
import { GridBackground } from '../common/GridBackground';
import { ApiError, formatYuan, todayStr, daysAgoStr } from '../common/Format';
import { LineChart, ChartSeries } from '../common/LineChart';
import { DonutChart, DonutSegment } from '../common/DonutChart';
import { fetchUsageStats, UsageStats, UsageModel } from '../model/Api';
import {
  getCookie, getDailySnapshots, deriveDailyRequests, summarizeLocal, DailySnapshot,
} from '../model/Store';

// 序列色值：与桌面版 app.js 对齐（费用 / Tokens / 余额）
const COLOR_YUAN = '#2c6b4b';
const COLOR_TOKENS = '#7a8a80';
const COLOR_REMAINING = '#b08d57';

// 模型分布环图色板：桌面版 MODEL_PALETTE 逐值移植
const MODEL_PALETTE: string[] =
  ['#2c6b4b', '#7a8a80', '#b08d57', '#5a6460', '#a3544f', '#4f6d7a', '#8b3e3e', '#6b7d5e'];

// 预设区间天数
const PRESET_DAYS: number[] = [7, 14, 30];

// 区间上限（天），与 api.ets assertValidRange 同口径
const MAX_RANGE_DAYS = 1096;

// 汇总卡单项展示数据（ArkTS 禁止对象字面量类型，显式声明接口）
interface SummaryCell {
  label: string;
  value: string;
}

// 解析 'YYYY-MM-DD' 为本地时区日期；格式非法或日期不存在（如 2026-02-30 滚动进位）返回 null
function parseLocalDateSafe(dateStr: string): Date | null {
  const match = dateStr.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!match) {
    return null;
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day, 0, 0, 0, 0);
  // 非法日期会被 Date 滚动进位（如 02-30 → 03-02），回读字段做往返校验
  if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day) {
    return null;
  }
  return date;
}

// 区间校验：返回错误文案，空串为通过；空输入视为编辑中，返回空串（不报错也不触发加载）
function validateRange(startStr: string, endStr: string): string {
  if (startStr.length === 0 || endStr.length === 0) {
    return '';
  }
  const start = parseLocalDateSafe(startStr);
  if (start === null) {
    return '起始日期格式错误，应为 YYYY-MM-DD';
  }
  const end = parseLocalDateSafe(endStr);
  if (end === null) {
    return '结束日期格式错误，应为 YYYY-MM-DD';
  }
  if (start.getTime() > end.getTime()) {
    return '起始日期不能晚于结束日期';
  }
  // 等长 YYYY-MM-DD 串可直接按字典序比较
  if (endStr > todayStr()) {
    return '结束日期不能超过今天';
  }
  const spanDays = Math.round((end.getTime() - start.getTime()) / 86_400_000);
  if (spanDays > MAX_RANGE_DAYS) {
    return `查询区间不能超过 ${MAX_RANGE_DAYS} 天`;
  }
  return '';
}

@Entry
@Component
struct StatsPage {
  @StorageProp('currentBreakpoint') bp: string = 'sm';
  @State rangeDays: number = 7;                    // 预设选中天数，0 = 自定义区间
  @State startDate: string = daysAgoStr(7);        // 'YYYY-MM-DD'
  @State endDate: string = todayStr();
  @State rangeError: string = '';
  @State official: UsageStats | null = null;
  @State localSnapshots: DailySnapshot[] = [];
  @State dailyRequests: Array<number | null> = []; // 快照差分派生的每日请求数
  @State degraded: boolean = false;                // 官方失败降级标记
  @State localFailed: boolean = false;             // 本地快照查询失败标记
  @State errorMsg: string = '';
  @State loading: boolean = false;
  @State chartVersion: number = 0;                 // 数据变化递增，触发图表重绘
  // 刷新串行标志：onPageShow / 预设切换 / 日期输入并发触发时仅首个执行，
  // 对齐看板页 refreshInFlight 模式，避免重复网络请求
  private refreshInFlight: boolean = false;

  async onPageShow(): Promise<void> {
    // 页面打开即加载；返回重进重新加载可接受（brief 明确不节流）
    await this.loadStats();
  }

  // 凭据无效统一路径：清空栈后替换为配置页（回退键直接退出应用）
  private async gotoConfig(): Promise<void> {
    try {
      const nav = this.getUIContext().getRouter();
      nav.clear();
      await nav.replaceUrl({ url: 'pages/ConfigPage' });
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`跳转配置页失败：${reason}`);
      // 导航失败降级：留在当前页并提示，避免静默卡死
      this.errorMsg = '凭据无效，且跳转配置页失败，请重试';
    }
  }

  // 顶栏返回：回到上一页（看板）
  private goBack(): void {
    try {
      this.getUIContext().getRouter().back();
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`返回看板失败：${reason}`);
    }
  }

  // 预设点击：重算起止日期并加载（重复点击同一预设等效手动刷新）
  private applyPreset(days: number): void {
    this.rangeDays = days;
    this.startDate = daysAgoStr(days);
    this.endDate = todayStr();
    this.rangeError = '';
    this.loadStats();
  }

  // 手动编辑日期：每次变更校验格式/顺序/跨度，非法仅报错不加载，合法按自定义区间加载
  private onRangeEdited(): void {
    if (this.startDate.length === 0 || this.endDate.length === 0) {
      // 任一为空视为编辑中：清除报错，不触发加载
      this.rangeError = '';
      return;
    }
    const error = validateRange(this.startDate, this.endDate);
    if (error.length > 0) {
      this.rangeError = error;
      return;
    }
    this.rangeError = '';
    this.rangeDays = 0; // 自定义区间取消预设选中态
    this.loadStats();
  }

  // 加载当前区间统计：官方聚合 + 本地快照并行拉取；
  // 区间在加载开始时快照为局部常量，两查询共用；加载期间区间被改写
  // （该次编辑的加载被 refreshInFlight 丢弃）时，完成后按新区间补载，保证数据与 UI 区间一致
  private async loadStats(): Promise<void> {
    if (this.refreshInFlight) {
      return;
    }
    this.refreshInFlight = true;
    try {
      this.loading = true;
      let cookie: string;
      try {
        cookie = (await getCookie()) ?? '';
      } catch (secretError) {
        // getCookie 在密文损坏时 reject：兜底视为无效凭据，与认证错误同路径（同看板页）
        const reason = secretError instanceof Error ? secretError.message : JSON.stringify(secretError);
        console.error(`凭据读取失败，视为无效凭据：${reason}`);
        await this.gotoConfig();
        return;
      }
      let rangeStart = this.startDate;
      let rangeEnd = this.endDate;
      for (;;) {
        const authFailed = await this.loadOnce(cookie, rangeStart, rangeEnd);
        if (authFailed) {
          await this.gotoConfig();
          return;
        }
        // 区间未变 → 结束；区间已变（在途加载期间切换预设/编辑日期）→ 按新区间再载一次
        if (this.startDate === rangeStart && this.endDate === rangeEnd) {
          return;
        }
        rangeStart = this.startDate;
        rangeEnd = this.endDate;
      }
    } finally {
      this.loading = false;
      this.refreshInFlight = false;
    }
  }

  // 单次加载：两数据源并行，避免官方超时推迟本地降级呈现；两查询共用同一区间快照；
  // 返回是否遇到认证错误（导航统一在两源都落定后执行）
  private async loadOnce(cookie: string, startDate: string, endDate: string): Promise<boolean> {
    const officialTask = this.loadOfficial(cookie, startDate, endDate);
    const localTask = this.loadLocal(startDate, endDate);
    await Promise.all([officialTask, localTask]);
    if (await officialTask) {
      return true;
    }
    this.dailyRequests = deriveDailyRequests(this.localSnapshots);
    // 数据就绪后统一递增版本号，触发图表重绘与 ForEach 键值更新
    this.chartVersion++;
    return false;
  }

  // 官方聚合任务：返回是否遇到认证错误
  private async loadOfficial(cookie: string, startDate: string, endDate: string): Promise<boolean> {
    try {
      this.official = await fetchUsageStats(cookie, startDate, endDate);
      this.degraded = false;
      this.errorMsg = '';
      return false;
    } catch (error) {
      if (error instanceof ApiError && error.isAuthError) {
        return true;
      }
      // 非认证错误降级本地估算，保留页面可用性，失败原因供横幅与模型块展示
      this.official = null;
      this.degraded = true;
      this.errorMsg = error instanceof ApiError ? error.message : '请求失败，请重试';
      return false;
    }
  }

  // 本地快照任务：失败兜底空数组并记录 localFailed，不阻断主流程
  private async loadLocal(startDate: string, endDate: string): Promise<void> {
    try {
      this.localSnapshots = await getDailySnapshots(startDate, endDate);
      this.localFailed = false;
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`本地快照查询失败：${reason}`);
      this.localSnapshots = [];
      this.localFailed = true;
    }
  }

  // ---- 数据映射：状态 → 图表 props ----

  // 消耗趋势 labels：官方模式用官方补零日期串，降级模式用快照日期串
  private trendLabels(): string[] {
    const labels: string[] = [];
    if (this.official !== null) {
      for (const day of this.official.daily) {
        labels.push(day.date);
      }
      return labels;
    }
    for (const snap of this.localSnapshots) {
      labels.push(snap.date);
    }
    return labels;
  }

  // 消耗趋势序列：官方实线，降级本地估算同色虚线（与桌面版一致）
  private trendSeries(): ChartSeries[] {
    const yuan: number[] = [];
    const tokens: number[] = [];
    if (this.official !== null) {
      for (const day of this.official.daily) {
        yuan.push(day.yuan);
        tokens.push(day.tokens);
      }
      return [
        { name: '费用 ¥', color: COLOR_YUAN, data: yuan },
        { name: 'Tokens', color: COLOR_TOKENS, data: tokens },
      ];
    }
    for (const snap of this.localSnapshots) {
      yuan.push(snap.yuan);
      tokens.push(snap.tokens);
    }
    return [
      { name: '费用 ¥', color: COLOR_YUAN, data: yuan, dashed: true },
      { name: 'Tokens', color: COLOR_TOKENS, data: tokens, dashed: true },
    ];
  }

  // 快照日期串（请求数与余额图共用）
  private snapshotDates(): string[] {
    const labels: string[] = [];
    for (const snap of this.localSnapshots) {
      labels.push(snap.date);
    }
    return labels;
  }

  // 请求数图序列：累计差分值，null 由折线图断开显示
  private requestSeries(): ChartSeries[] {
    return [{ name: '新增请求数', color: COLOR_YUAN, data: this.dailyRequests }];
  }

  // 请求数块可见性：无任何有效差分（全新安装或老数据无请求数）整块隐藏
  private hasRequestData(): boolean {
    for (const value of this.dailyRequests) {
      if (value !== null) {
        return true;
      }
    }
    return false;
  }

  // 余额图序列
  private remainingSeries(): ChartSeries[] {
    const data: number[] = [];
    for (const snap of this.localSnapshots) {
      data.push(snap.remaining);
    }
    return [{ name: '剩余额度 ¥', color: COLOR_REMAINING, data: data }];
  }

  // 环图分段：占比口径决定取值（quota 口径取 yuan，tokens 口径取 tokens），色板循环取色
  private modelSegments(): DonutSegment[] {
    if (this.official === null) {
      return [];
    }
    const useTokens = this.official.percentBasis === 'tokens';
    const segments: DonutSegment[] = [];
    for (let i = 0; i < this.official.models.length; i++) {
      const model = this.official.models[i];
      segments.push({
        label: model.model,
        value: useTokens ? model.tokens : model.yuan,
        color: MODEL_PALETTE[i % MODEL_PALETTE.length],
      });
    }
    return segments;
  }

  // 模型列表占比口径标注：tokens 口径时标注"按 Token 占比"
  private modelBasisNote(): string {
    if (this.official === null) {
      return '';
    }
    return this.official.percentBasis === 'tokens'
      ? '按 Token 占比（quota 全为 0）'
      : '按费用占比';
  }

  // 当前模型列表（@Builder 内部直读状态，规避按值传参不刷新问题，见 faqs-arkui-375）
  private currentModels(): UsageModel[] {
    return this.official !== null ? this.official.models : [];
  }

  // 汇总卡：官方模式用官方汇总；降级用本地估算且请求数固定 '--'；
  // 降级且本地查询也失败时全部 '--'
  private summaryCells(): SummaryCell[] {
    if (this.official !== null) {
      const s = this.official.summary;
      return [
        { label: '累计费用', value: formatYuan(s.totalYuan) },
        { label: '日均费用', value: formatYuan(s.avgYuan) },
        { label: '单日峰值', value: s.peakDate.length > 0 ? `${formatYuan(s.peakYuan)}（${s.peakDate}）` : '--' },
        { label: '有使用天数', value: String(s.daysWithUsage) },
        { label: '区间总请求数', value: String(s.requestCount) },
      ];
    }
    if (this.localFailed) {
      return [
        { label: '累计费用', value: '--' },
        { label: '日均费用', value: '--' },
        { label: '单日峰值', value: '--' },
        { label: '有使用天数', value: '--' },
        { label: '区间总请求数', value: '--' },
      ];
    }
    const s = summarizeLocal(this.localSnapshots);
    return [
      { label: '累计费用', value: formatYuan(s.totalYuan) },
      { label: '日均费用', value: formatYuan(s.avgYuan) },
      { label: '单日峰值', value: s.peakDate.length > 0 ? `${formatYuan(s.peakYuan)}（${s.peakDate}）` : '--' },
      { label: '有使用天数', value: String(s.daysWithUsage) },
      // 本地快照无法派生区间请求数，降级固定 '--'
      { label: '区间总请求数', value: '--' },
    ];
  }

  // 图表高度：sm 紧凑、md/lg 放大
  private chartHeight(large: boolean): number {
    if (this.bp === 'sm') {
      return large ? 200 : 150;
    }
    return large ? 240 : 180;
  }

  // 区块标题（等宽小号大写，与看板页风格一致）
  @Builder
  blockTitle(title: string) {
    Text(title)
      .fontFamily(Theme.fontMono).fontSize(12).fontWeight(FontWeight.Bold)
      .fontColor(Theme.muted).letterSpacing(2)
  }

  // 模型列表行（sm 纵向 / md/lg 横排两种布局共用）
  // 无参直读状态：@Builder 按值传参不随状态变化刷新（faqs-arkui-375）；
  // ForEach 键纳入 chartVersion：区间切换后共有模型数值必须刷新（faqs-arkui-619）
  @Builder
  modelListRows() {
    Column({ space: 8 }) {
      ForEach(this.currentModels(), (model: UsageModel, index: number) => {
        Row({ space: 8 }) {
          Row()
            .width(8).height(8).borderRadius(4)
            .backgroundColor(MODEL_PALETTE[index % MODEL_PALETTE.length])
          Text(model.model)
            .fontSize(11).fontColor(Theme.ink)
            .layoutWeight(1)
            .maxLines(1).textOverflow({ overflow: TextOverflow.Ellipsis })
          Text(`¥${model.yuan.toFixed(4)} / ${model.percent.toFixed(1)}% / ${model.requestCount} 次`)
            .fontFamily(Theme.fontMono).fontSize(10).fontColor(Theme.muted)
        }
        .width('100%')
      }, (model: UsageModel, index: number): string => `${this.chartVersion}-${index}-${model.model}`)
    }
    .width('100%')
  }

  // 模型分布主体：sm 环图在上列表在下，md/lg 环图左列表右（无参直读状态，同上）
  @Builder
  modelBody() {
    if (this.bp === 'sm') {
      Column({ space: 12 }) {
        DonutChart({ segments: this.modelSegments(), version: this.chartVersion })
          .width('100%').height(170)
        this.modelListRows()
      }
      .width('100%')
    } else {
      Row({ space: 16 }) {
        DonutChart({ segments: this.modelSegments(), version: this.chartVersion })
          .width(200).height(190)
        Column() {
          this.modelListRows()
        }
        .layoutWeight(1)
      }
      .width('100%').alignItems(VerticalAlign.Top)
    }
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      GridBackground()
      Scroll() {
        Column({ space: 0 }) {
          // 顶栏：标题 + 导出（P4 接线）+ 返回看板
          Row() {
            Text('AMAX / ANALYTICS')
              .fontFamily(Theme.fontMono).fontSize(12).fontWeight(FontWeight.Bold)
              .fontColor(Theme.accent).letterSpacing(2)
            Blank()
            if (this.loading) {
              Text('加载中…').fontSize(10).fontColor(Theme.muted).margin({ right: 12 })
            }
            // 导出按钮：本任务仅占位并禁用，CSV/JSON/XLSX 导出由 P4 任务接线启用
            Text('导出')
              .fontFamily(Theme.fontMono).fontSize(10).fontColor(Theme.muted)
              .border({ width: 1, color: Theme.line })
              .padding({ left: 10, right: 10, top: 4, bottom: 4 })
              .opacity(0.45)
              .enabled(false)
            Text('返回看板')
              .fontSize(11).fontColor(Theme.muted).margin({ left: 12 })
              .onClick(() => this.goBack())
          }
          .width('100%').padding({ top: 14, bottom: 14 })

          // 区间选择器：预设按钮（选中态 accentSoft）
          // 键纳入 rangeDays：选中高亮随 rangeDays 变化，键恒定则复用旧项不刷新（faqs-arkui-619）
          Row({ space: 8 }) {
            ForEach(PRESET_DAYS, (days: number) => {
              Text(`${days} 天`)
                .fontFamily(Theme.fontMono).fontSize(11)
                .fontColor(this.rangeDays === days ? Theme.accent : Theme.muted)
                .backgroundColor(this.rangeDays === days ? Theme.accentSoft : Color.Transparent)
                .border({ width: 1, color: this.rangeDays === days ? Theme.accent : Theme.line })
                .padding({ left: 12, right: 12, top: 5, bottom: 5 })
                .onClick(() => this.applyPreset(days))
            }, (days: number): string => `${this.rangeDays}-${days}`)
          }
          .width('100%')

          // 区间选择器：手动起止日期（YYYY-MM-DD 手动输入，onChange 即校验）
          Row({ space: 8 }) {
            TextInput({ text: this.startDate, placeholder: 'YYYY-MM-DD' })
              .fontFamily(Theme.fontMono).fontSize(11).fontColor(Theme.ink)
              .placeholderColor(Theme.muted)
              .backgroundColor(Theme.surfaceRaised)
              .border({ width: 1, color: Theme.line })
              .height(34).layoutWeight(1)
              .onChange((value: string) => {
                this.startDate = value;
                this.onRangeEdited();
              })
            Text('~').fontSize(12).fontColor(Theme.muted)
            TextInput({ text: this.endDate, placeholder: 'YYYY-MM-DD' })
              .fontFamily(Theme.fontMono).fontSize(11).fontColor(Theme.ink)
              .placeholderColor(Theme.muted)
              .backgroundColor(Theme.surfaceRaised)
              .border({ width: 1, color: Theme.line })
              .height(34).layoutWeight(1)
              .onChange((value: string) => {
                this.endDate = value;
                this.onRangeEdited();
              })
          }
          .width('100%').margin({ top: 10 })

          if (this.rangeError.length > 0) {
            Text(this.rangeError)
              .fontSize(11).fontColor(Theme.error)
              .margin({ top: 8 })
          }

          // 降级横幅：官方失败原因 + 降级说明；非降级时的 errorMsg 单独显示（如跳转配置页失败）
          if (this.degraded) {
            Column({ space: 4 }) {
              Text(`官方数据获取失败：${this.errorMsg}`)
                .fontSize(12).fontColor(Theme.error)
              Text('官方数据不可用，趋势与汇总为本地估算')
                .fontSize(11).fontColor(Theme.error)
            }
            .alignItems(HorizontalAlign.Start)
            .width('100%').backgroundColor(Theme.errorBg)
            .padding(10).margin({ top: 12 })
          } else if (this.errorMsg.length > 0) {
            Text(this.errorMsg)
              .fontSize(12).fontColor(Theme.error).backgroundColor(Theme.errorBg)
              .width('100%').padding(10).margin({ top: 12 })
          }

          if (this.official !== null || this.degraded) {
            // 汇总卡
            Column({ space: 10 }) {
              // 调用点直写 Text 读取 this.degraded：@Builder 按值传参不随状态变化刷新（faqs-arkui-375）
              Text(this.degraded ? 'RANGE SUMMARY（本地估算）' : 'RANGE SUMMARY')
                .fontFamily(Theme.fontMono).fontSize(12).fontWeight(FontWeight.Bold)
                .fontColor(Theme.muted).letterSpacing(2)
              Flex({ wrap: FlexWrap.Wrap }) {
                // 键纳入 chartVersion：label 恒定 value 变化时必须换键才会刷新（faqs-arkui-619）
                ForEach(this.summaryCells(), (cell: SummaryCell, index: number) => {
                  Column({ space: 4 }) {
                    Text(cell.label).fontSize(10).fontColor(Theme.muted)
                    Text(cell.value)
                      .fontFamily(Theme.fontMono).fontSize(13).fontWeight(600).fontColor(Theme.ink)
                  }
                  .alignItems(HorizontalAlign.Start)
                  .width(this.bp === 'sm' ? '50%' : '20%')
                  .padding({ top: 8, bottom: 8, right: 10 })
                }, (cell: SummaryCell, index: number): string => `${this.chartVersion}-${index}`)
              }
              .width('100%')
            }
            .width('100%').padding({ top: 20, bottom: 20 })
            .border({ width: { top: 1 }, color: Theme.line })
            .margin({ top: 16 })

            // 消耗趋势：双轴（左 ¥ / 右 Tokens），官方实线 / 降级虚线
            Column({ space: 10 }) {
              Row() {
                this.blockTitle('COST / TOKENS TREND')
                Blank()
                if (this.degraded) {
                  Text('本地估算（快照日末累计值）')
                    .fontSize(10).fontColor(Theme.muted)
                }
              }
              .width('100%')
              LineChart({
                labels: this.trendLabels(),
                series: this.trendSeries(),
                version: this.chartVersion,
                dualAxis: true,
              })
                .width('100%').height(this.chartHeight(true))
            }
            .width('100%').padding({ top: 20, bottom: 20 })
            .border({ width: { top: 1 }, color: Theme.line })

            // 请求数趋势：本地快照累计差分，无有效差分整块隐藏
            if (this.hasRequestData()) {
              Column({ space: 10 }) {
                this.blockTitle('DAILY REQUESTS')
                LineChart({
                  labels: this.snapshotDates(),
                  series: this.requestSeries(),
                  version: this.chartVersion,
                })
                  .width('100%').height(this.chartHeight(false))
              }
              .width('100%').padding({ top: 20, bottom: 20 })
              .border({ width: { top: 1 }, color: Theme.line })
            }

            // 模型分布：仅官方可用；降级或空 models 显示错误/空态
            Column({ space: 10 }) {
              Row() {
                this.blockTitle('MODEL DISTRIBUTION')
                Blank()
                if (this.official !== null && this.official.models.length > 0) {
                  Text(this.modelBasisNote()).fontSize(10).fontColor(Theme.muted)
                }
              }
              .width('100%')

              if (this.degraded) {
                Text(`官方数据不可用：${this.errorMsg}`)
                  .fontSize(12).fontColor(Theme.error)
                  .width('100%').padding({ top: 8, bottom: 8 })
              } else if (this.official !== null && this.official.models.length === 0) {
                Text('暂无模型数据')
                  .fontSize(12).fontColor(Theme.muted)
                  .width('100%').padding({ top: 8, bottom: 8 })
              } else if (this.official !== null) {
                this.modelBody()
              }
            }
            .width('100%').padding({ top: 20, bottom: 20 })
            .border({ width: { top: 1 }, color: Theme.line })

            // 余额趋势：本地快照日末余额，空数据整块隐藏
            if (this.localSnapshots.length > 0) {
              Column({ space: 10 }) {
                this.blockTitle('BALANCE TREND')
                LineChart({
                  labels: this.snapshotDates(),
                  series: this.remainingSeries(),
                  version: this.chartVersion,
                })
                  .width('100%').height(this.chartHeight(false))
              }
              .width('100%').padding({ top: 20, bottom: 20 })
              .border({ width: { top: 1 }, color: Theme.line })
            }
          } else if (this.loading) {
            Text('正在加载…').fontSize(13).fontColor(Theme.muted).margin({ top: 60 })
          } else {
            // 正常不应到达（loadStats 结束后 official/degraded 必居其一），占位避免空白页
            Text('暂无数据').fontSize(13).fontColor(Theme.muted).margin({ top: 60 })
          }
        }
        .width('100%')
        .constraintSize({ maxWidth: Theme.contentMaxWidth(this.bp) })
        .padding({ left: Theme.pagePadding(this.bp), right: Theme.pagePadding(this.bp), bottom: 40 })
      }
      .width('100%').height('100%')
      .scrollBar(BarState.Auto)
      .align(Alignment.Top)
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
