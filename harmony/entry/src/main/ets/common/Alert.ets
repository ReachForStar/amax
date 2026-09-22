// 告警引擎（纯函数，与桌面端 src-tauri/src/alert.rs 同规则、同口径、同文案）：
// - quota_low：剩余额度百分比低于水位
// - runout_soon：剩余 ÷ 近 7 日日均花费 ≤ 天数
// - spike：当日花费 ≥ 近 7 日中位数 × 倍数（用中位数以免极端日自己抬高门槛）
// 去重是「同规则同日一次」且跨进程持久（alert_state 表，见 Store.ets），不随应用重启复位；
// 每日上限是所有规则合计的投递次数，0 表示当日全部静默。
// 宁可漏报不误报：基准窗口内完全没有用量记录时（新装、或一直没运行）三条规则一律不判。
// 本文件不碰数据库、不发通知，全部判定可单测（entry/src/test/Alert.test.ets）。

/// 基准窗口天数：燃尽日均与突增中位数都只看最近这些天（不含当日）
export const BASELINE_WINDOW_DAYS: number = 7;
/// 突增判定所需的最少有用量天数，样本不足直接不判
export const MIN_SPIKE_SAMPLES: number = 3;
/// 每日告警总数上限的可调上界
export const MAX_DAILY_FIRES_LIMIT: number = 10;

/// 用户可调的四个参数 + 总开关，持久化在 preferences config 库
export interface AlertSettings {
  enabled: boolean;
  /// 剩余额度水位（百分比），低于该值告警
  quotaPercent: number;
  /// 按日均花费还能撑多少天以内算告警
  runoutDays: number;
  /// 当日花费达到中位数的多少倍算异常
  spikeMultiplier: number;
  /// 所有规则合计的当日投递次数上限
  maxFiresPerDay: number;
}

export function defaultSettings(): AlertSettings {
  return {
    enabled: true,
    quotaPercent: 10.0,
    runoutDays: 5,
    spikeMultiplier: 3.0,
    maxFiresPerDay: 2,
  };
}

function clampOr(value: number, min: number, max: number, fallback: number): number {
  return Number.isFinite(value) ? Math.min(max, Math.max(min, value)) : fallback;
}

// 整数参数：先夹取再取整（preferences 可能存进小数或脏值，Rust 侧由 i64 类型天然保证）
function clampInt(value: number, min: number, max: number, fallback: number): number {
  return Math.round(clampOr(value, min, max, fallback));
}

/// 逐字段夹到合法区间，非有限值取默认值。
/// 任何输入都产出可用参数，调用方不必为「参数不合法」再写一套语义。
export function sanitizeSettings(settings: AlertSettings): AlertSettings {
  return {
    enabled: settings.enabled,
    quotaPercent: clampOr(settings.quotaPercent, 1.0, 99.0, 10.0),
    runoutDays: clampInt(settings.runoutDays, 1, 90, 5),
    spikeMultiplier: clampOr(settings.spikeMultiplier, 1.5, 100.0, 3.0),
    maxFiresPerDay: clampInt(settings.maxFiresPerDay, 0, MAX_DAILY_FIRES_LIMIT, 2),
  };
}

/// 可调参数的合法区间（设置界面据此做输入约束与提示，避免两处各写一份数字）
export interface ParamRange {
  key: string;
  min: number;
  max: number;
}

/// 四个可调参数的区间，`key` 与桌面端 param_ranges 一致（snake_case）
export function paramRanges(): ParamRange[] {
  const ranges: ParamRange[] = [
    { key: 'quota_percent', min: 1.0, max: 99.0 },
    { key: 'runout_days', min: 1.0, max: 90.0 },
    { key: 'spike_multiplier', min: 1.5, max: 100.0 },
    { key: 'max_fires_per_day', min: 0.0, max: MAX_DAILY_FIRES_LIMIT },
  ];
  return ranges;
}

/// 规则顺序即投递顺序：额度是存量、燃尽是趋势、突增是当日异常，
/// 当日剩余次数不够时优先留下更靠前的一条。key 与桌面端 alert_state.rule_key 一致，改名等于丢历史。
export const RULE_KEYS: string[] = ['quota_low', 'runout_soon', 'spike'];

export function ruleTitle(key: string): string {
  if (key === 'quota_low') {
    return '⚠️ AMAX 额度不足';
  }
  if (key === 'runout_soon') {
    return '⏳ AMAX 额度将耗尽';
  }
  if (key === 'spike') {
    return '📈 AMAX 今日花费异常';
  }
  return 'AMAX 额度告警';
}

/// 看板刷新那一刻的账户状态
export interface Current {
  /// 剩余额度百分比（与桌面版 assemble_dashboard 同口径：quota/total×100）
  percent: number;
  remaining: number;
  total: number;
  todayYuan: number;
}

/// 近期用量基准（不含当日）
export interface Baseline {
  avgYuan: number | null;
  medianYuan: number | null;
  /// 有用量的天数；0 表示窗口内完全无记录
  sampleDays: number;
}

export function emptyBaseline(): Baseline {
  return { avgYuan: null, medianYuan: null, sampleDays: 0 };
}

/// 一次命中
export interface Finding {
  rule: string;
  title: string;
  message: string;
}

/// alert_state 的一行读数
export interface RuleCount {
  key: string;
  count: number;
}

/// 当日已投递情况：total 是所有规则合计，perRule 与 RULE_KEYS 同序
export interface DayState {
  total: number;
  perRule: number[];
}

/// 由 alert_state 行构造；未知 rule_key 忽略（规则增删后的历史行不该影响新判断）
export function dayStateFromCounts(counts: RuleCount[]): DayState {
  const perRule: number[] = [0, 0, 0];
  let total = 0;
  for (const row of counts) {
    const index = RULE_KEYS.indexOf(row.key);
    if (index >= 0) {
      perRule[index] = row.count;
      total += row.count;
    }
  }
  return { total: total, perRule: perRule };
}

/// 单条规则是否命中（不含去重与上限）
function breaches(key: string, current: Current, baseline: Baseline, settings: AlertSettings): boolean {
  if (baseline.sampleDays === 0) {
    return false;
  }
  if (key === 'quota_low') {
    return current.percent < settings.quotaPercent;
  }
  if (key === 'runout_soon') {
    const avg = baseline.avgYuan;
    return avg !== null && avg > 0.0 && current.remaining / avg <= settings.runoutDays;
  }
  const median = baseline.medianYuan;
  return median !== null && median > 0.0 && baseline.sampleDays >= MIN_SPIKE_SAMPLES
    && current.todayYuan >= settings.spikeMultiplier * median;
}

/// 命中正文；null 表示这条规则此刻给不出可信的描述（例如日均为 0 无法算燃尽天数）
function ruleBody(key: string, current: Current, baseline: Baseline): string | null {
  if (key === 'quota_low') {
    return `剩余 ¥${current.remaining.toFixed(2)} / ¥${current.total.toFixed(2)} ` +
      `(${current.percent.toFixed(1)}%), 请及时充值`;
  }
  if (key === 'runout_soon') {
    const avg = baseline.avgYuan;
    if (avg === null || avg <= 0.0) {
      return null;
    }
    const days = current.remaining / avg;
    return Number.isFinite(days)
      ? `剩余 ¥${current.remaining.toFixed(2)}，按近${BASELINE_WINDOW_DAYS}日日均 ¥${avg.toFixed(2)} 约 ${days.toFixed(1)} 天耗尽`
      : null;
  }
  if (baseline.sampleDays < MIN_SPIKE_SAMPLES) {
    return null;
  }
  const median = baseline.medianYuan;
  if (median === null || median <= 0.0) {
    return null;
  }
  const ratio = current.todayYuan / median;
  return Number.isFinite(ratio)
    ? `今日已花 ¥${current.todayYuan.toFixed(2)}，是近${baseline.sampleDays}日中位数 ¥${median.toFixed(2)} 的 ${ratio.toFixed(1)} 倍`
    : null;
}

/// 评估当日应投递的告警：命中 + 已启用 + 今日未投，按 RULE_KEYS 顺序取到当日剩余额度为止
export function detect(current: Current, baseline: Baseline, settings: AlertSettings,
  rulesEnabled: string[], state: DayState): Finding[] {
  if (!settings.enabled || state.total >= settings.maxFiresPerDay) {
    return [];
  }
  const budget = settings.maxFiresPerDay - state.total;
  const findings: Finding[] = [];
  for (const key of RULE_KEYS) {
    if (findings.length >= budget) {
      break;
    }
    if (!rulesEnabled.includes(key)) {
      continue;
    }
    if (state.perRule[RULE_KEYS.indexOf(key)] !== 0) {
      continue;
    }
    if (!breaches(key, current, baseline, settings)) {
      continue;
    }
    const message = ruleBody(key, current, baseline);
    if (message === null) {
      continue;
    }
    findings.push({ rule: key, title: ruleTitle(key), message: message });
  }
  return findings;
}

/// 快照行里基准关心的两列（DailySnapshot 结构兼容，调用方直接传按日快照）
export interface UsageDay {
  date: string;
  yuan: number;
}

/// 'YYYY-MM-DD' 或本地 RFC3339 → 纪元日序号；解析失败抛错由调用方跳过本轮告警。
/// 用 UTC 毫秒折算整数日序号做窗口比较，避开本地/UTC 混算与跨月进位陷阱
function daySerial(value: string): number {
  const match = value.match(/^(\d{4})-(\d{2})-(\d{2})/);
  if (!match) {
    throw new Error(`告警基准日期无法解析: ${value}`);
  }
  const millis = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  if (Number.isNaN(millis)) {
    throw new Error(`告警基准日期无法解析: ${value}`);
  }
  return Math.floor(millis / 86_400_000);
}

function serialToDayString(serial: number): string {
  const date = new Date(serial * 86_400_000);
  const month = String(date.getUTCMonth() + 1).padStart(2, '0');
  const day = String(date.getUTCDate()).padStart(2, '0');
  return `${date.getUTCFullYear()}-${month}-${day}`;
}

/// 基准窗口起始日（含）：告警与查询共用，避免两处各算一次
export function baselineStartDay(day: string, windowDays: number): string {
  const window = windowDays > 0 ? windowDays : 1;
  return serialToDayString(daySerial(day) - window);
}

/// 日末快照序列 → 用量基准：排除当日与窗口外的日期，只取**有用量**（yuan > 0）的日子。
///
/// 空缺日（应用没运行）整行排除：当成 ¥0 会同时拉低日均（把正常一天判成突增）和抬高日均
/// （把快耗尽算成还能撑），两头都是误报。
export function buildBaseline(snapshots: UsageDay[], day: string, windowDays: number): Baseline {
  const today = daySerial(day);
  const start = today - (windowDays > 0 ? windowDays : 1);

  // 同 (日期, 金额) 重复行折叠：按日快照本不该重复，这里与桌面版同样防御
  const usage: UsageDay[] = [];
  for (const snapshot of snapshots) {
    const serial = daySerial(snapshot.date);
    if (snapshot.yuan > 0.0 && serial >= start && serial < today) {
      const duplicated = usage.some((row: UsageDay): boolean => row.date === snapshot.date
        && row.yuan === snapshot.yuan);
      if (!duplicated) {
        usage.push({ date: serialToDayString(serial), yuan: snapshot.yuan });
      }
    }
  }
  usage.sort((a: UsageDay, b: UsageDay): number => a.date.localeCompare(b.date));

  const values: number[] = usage.map((row: UsageDay): number => row.yuan);
  const sampleDays = values.length;
  if (sampleDays === 0) {
    return emptyBaseline();
  }
  let sum = 0;
  for (const value of values) {
    sum += value;
  }
  // values 按日期有序而非按金额有序，中位数要单独排序
  const sorted = values.slice();
  sorted.sort((a: number, b: number): number => a - b);
  const mid = Math.floor(sorted.length / 2);
  const median = sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2.0 : sorted[mid];
  return { avgYuan: sum / sampleDays, medianYuan: median, sampleDays: sampleDays };
}

/// 解析「哪些规则启用」的写入值：未传即保持现状，传了必须全在已知规则集合内。
/// 返回值按 RULE_KEYS 顺序规范化。
export function resolveRulesEnabled(current: string[], requested: string[] | null): string[] {
  if (requested === null) {
    return current.slice();
  }
  const unknown = requested.filter((key: string): boolean => !RULE_KEYS.includes(key));
  if (unknown.length > 0) {
    throw new Error(`未知告警规则: ${unknown.join(', ')}`);
  }
  return RULE_KEYS.filter((key: string): boolean => requested.includes(key));
}
