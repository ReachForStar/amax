// 告警编排（对应桌面端 lib.rs 的 run_alerts / deliver_alert）：
// 读配置与账本 → 算基准 → 判规则 → 逐渠道投递 → 记账 → 给出当日状态。
//
// 与桌面端一致的两条口径：
// - 日界只在这里取一次：已投次数与用量基准都基于同一个 day，分两次取时间会让跨午夜的那一轮
//   读到「新的一天 + 旧的一天的次数」而重复投递。
// - 整条链路只写日志不抛错：看板刷新已经成功，不能因为通知发不出去而回报失败。
// 手机端没有 SMTP 渠道，投递失败最坏只等满 MeoW 的 15 秒超时。
import {
  AlertSettings, BASELINE_WINDOW_DAYS, Baseline, Current, DayState, Finding,
  buildBaseline, baselineStartDay, dayStateFromCounts, detect,
} from '../common/Alert';
import { Channel, MAX_FAILED_DELIVERIES_PER_RULE, Receipt, deliver, summarize } from '../common/Deliver';
import {
  AlertChannels, countAlertFailures, getAlertChannels, getAlertDayCounts, getAlertRulesEnabled,
  getAlertSettings, getDailySnapshots, recordAlertDelivery, recordAlertFire,
} from './Store';
import { localRfc3339, todayStr } from '../common/Format';

/// 看板刷新那一刻的事实（DashboardData 里与告警相关的四列）
export interface RefreshFacts {
  percent: number;
  remaining: number;
  total: number;
  todayYuan: number;
}

/// 当日告警状态；三种 state 字段一致，界面无需分支解析
export interface AlertStatus {
  state: string;
  day: string;
  firedToday: number;
  cap: number;
}

export type Availability = 'ready' | 'incomplete' | 'off';

const CHANNELS: Channel[] = ['notification', 'meow'];

/// 渠道可用性：开着且配置齐全才投。MeoW 缺昵称时算「缺配置」而不是「未开启」。
export function channelAvailability(channels: AlertChannels, channel: Channel): Availability {
  const enabled = channel === 'meow' ? channels.meow : channels.notification;
  if (!enabled) {
    return 'off';
  }
  if (channel === 'meow' && channels.meowNickname === null) {
    return 'incomplete';
  }
  return 'ready';
}

/// 本轮真正能投的渠道
export function usableChannels(channels: AlertChannels): Channel[] {
  return CHANNELS.filter((channel: Channel): boolean =>
    channelAvailability(channels, channel) === 'ready');
}

function statusPayload(settings: AlertSettings, day: string, firedToday: number): AlertStatus {
  let state = 'armed';
  if (!settings.enabled) {
    state = 'disabled';
  } else if (firedToday >= settings.maxFiresPerDay) {
    state = 'cap_hit';
  }
  return { state: state, day: day, firedToday: firedToday, cap: settings.maxFiresPerDay };
}

/// 当日已投次数（所有规则合计）。账本读不到时返回 null，让界面宁可留白也不拿 0 假装「今天还没投过」
export async function readAlertStatus(): Promise<AlertStatus | null> {
  const day = todayStr();
  try {
    const settings = await getAlertSettings();
    const state = dayStateFromCounts(await getAlertDayCounts(day));
    return statusPayload(settings, day, state.total);
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    console.error(`读取当日告警次数失败，设置页不显示进度：${reason}`);
    return null;
  }
}

/// 评估并投递本轮告警，返回当日状态；配置或账本读不出来时返回 null（宁可不报，误报代价更高）
export async function runAlerts(facts: RefreshFacts): Promise<AlertStatus | null> {
  const day = todayStr();
  let settings: AlertSettings;
  let rulesEnabled: string[];
  let channels: AlertChannels;
  let state: DayState;
  let baseline: Baseline;
  try {
    settings = await getAlertSettings();
    rulesEnabled = await getAlertRulesEnabled();
    channels = await getAlertChannels();
    state = dayStateFromCounts(await getAlertDayCounts(day));
    baseline = await readBaseline(day);
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    console.error(`读取告警配置/账本/基准失败，跳过本轮告警：${reason}`);
    return null;
  }

  const current: Current = {
    percent: facts.percent,
    remaining: facts.remaining,
    total: facts.total,
    todayYuan: facts.todayYuan,
  };
  const findings = detect(current, baseline, settings, rulesEnabled, state);
  const usable = usableChannels(channels);
  if (findings.length === 0) {
    return statusPayload(settings, day, state.total);
  }
  if (usable.length === 0) {
    // 一条渠道都没开时不消耗额度：额度记的是「今日已投出几次」，没投出去就不算投过
    console.warn(`检测到 ${findings.length} 条告警但没有可用渠道，本轮跳过投递（设置页里开启系统通知或补全 MeoW 昵称）`);
    return statusPayload(settings, day, state.total);
  }
  for (const finding of findings) {
    await deliverOne(finding, day, channels, usable);
  }
  // 按账本真实次数回读，不拿「投了几条」当已投次数：可重试的失败没消耗额度
  const after = await readCountsQuietly(day);
  return statusPayload(settings, day, after === null ? state.total : after.total);
}

async function readCountsQuietly(day: string): Promise<DayState | null> {
  try {
    return dayStateFromCounts(await getAlertDayCounts(day));
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    console.error(`回读当日告警次数失败，沿用本轮开始时的次数：${reason}`);
    return null;
  }
}

/// 读基准：日末快照 → 近 7 日「有用量日子」的日均与中位数
async function readBaseline(day: string): Promise<Baseline> {
  const start = baselineStartDay(day, BASELINE_WINDOW_DAYS);
  const snapshots = await getDailySnapshots(start, day);
  return buildBaseline(snapshots, day, BASELINE_WINDOW_DAYS);
}

/// 单条告警投给所有可用渠道并记账。
///
/// 额度消耗口径与桌面端一致：任一渠道成功即算投过；全部永久失败也算投过（否则每轮刷新撞同一堵墙）；
/// 只有「还能重试」时不消耗，留给下一轮，并由 MAX_FAILED_DELIVERIES_PER_RULE 封顶。
async function deliverOne(finding: Finding, day: string, channels: AlertChannels,
  usable: Channel[]): Promise<void> {
  const failures = await countAlertFailures(finding.rule, day).catch((error: Error): number => {
    // 读不出失败次数时按「没失败过」处理：宁可多试一次，也不要把一次坏查询变成整天静默
    console.error(`读取告警失败次数失败，按 0 处理：${error.message}`);
    return 0;
  });
  if (failures >= MAX_FAILED_DELIVERIES_PER_RULE) {
    console.warn(`告警 ${finding.rule} 当日已累计 ${MAX_FAILED_DELIVERIES_PER_RULE} 条失败投递，今天不再重试`);
    await consumeSlot(finding.rule, day);
    return;
  }
  const receipts: Receipt[] = [];
  for (const channel of usable) {
    const nickname = channels.meowNickname === null ? '' : channels.meowNickname;
    const receipt = await deliver(channel, finding.rule, nickname, finding.title, finding.message);
    receipts.push(receipt);
    try {
      await recordAlertDelivery(finding.rule, day, channel, receipt.outcome === 'sent',
        receipt.error, localRfc3339(new Date()));
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`投递明细写库失败（不影响本轮判定）：${reason}`);
    }
  }
  if (summarize(receipts) !== 'retryable') {
    await consumeSlot(finding.rule, day);
  }
}

async function consumeSlot(rule: string, day: string): Promise<void> {
  try {
    await recordAlertFire(rule, day, localRfc3339(new Date()));
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    console.error(`告警记账失败，${rule} 今天可能重复投递：${reason}`);
  }
}
