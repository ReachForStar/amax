// 存储层：preferences 库 config 存认证凭据（经 Secret 加密）与告警配置（明文），RDB 库 amax_dashboard.db 存刷新快照与告警账本
// schema 与桌面版 src-tauri/src/db.rs 的 dashboard_snapshot / alert_state / alert_delivery 表逐字段一致
// initStore 由 EntryAbility 启动时调用一次；未初始化即调用其余接口抛明确错误
import { preferences, relationalStore } from '@kit.ArkData';
import { encryptSecret, decryptSecret } from './Secret';
import {
  AlertSettings, RuleCount, defaultSettings, resolveRulesEnabled, RULE_KEYS, sanitizeSettings,
} from '../common/Alert';

const CONFIG_STORE = 'config';
const DB_NAME = 'amax_dashboard.db';
const KEY_COOKIE = 'cookie';
const KEY_API_KEY = 'api_key';
const KEY_COOKIE_SOURCE = 'cookie_source';
const KEY_COOKIE_EXPIRES_AT = 'cookie_expires_at';
// 告警配置键名与桌面版 db.rs 的 config 表逐一对应（两端各存本机，不做同步）
const KEY_ALERT_ENABLED = 'alert_enabled';
const KEY_ALERT_QUOTA_PERCENT = 'alert_quota_percent';
const KEY_ALERT_RUNOUT_DAYS = 'alert_runout_days';
const KEY_ALERT_SPIKE_MULTIPLIER = 'alert_spike_multiplier';
const KEY_ALERT_MAX_FIRES_PER_DAY = 'alert_max_fires_per_day';
const KEY_ALERT_RULE_PREFIX = 'alert_rule_';
const KEY_ALERT_CH_NOTIFICATION = 'alert_ch_notification';
const KEY_ALERT_CH_MEOW = 'alert_ch_meow';
const KEY_MEOW_NICKNAME = 'meow_nickname';

export interface SnapshotRow {
  savedAt: string;        // 本地时区 RFC3339
  todayYuan: number;
  totalTokens: number;
  remaining: number;
  used: number;
  total: number;
  requestCount: number;
}

// 统计页本地快照行：区间内每个本地日取日末一条快照
export interface DailySnapshot {
  date: string;                 // 'YYYY-MM-DD' 本地时区
  yuan: number;
  tokens: number;
  remaining: number;
  requestCount: number | null;  // request_count 列可空（早期快照无请求数）
}

// 本地快照区间汇总（summarizeLocal 派生）
export interface LocalSummary {
  totalYuan: number;
  avgYuan: number;
  peakYuan: number;
  peakDate: string;
  totalTokens: number;
  // 有快照天数（同桌面版 days_with_data，本地模式汇总卡与导出用此口径）
  daysWithData: number;
  // 有用量天数（yuan>0 || tokens>0），保留供需要该口径的调用方使用
  daysWithUsage: number;
}

let configStore: preferences.Preferences | null = null;
let db: relationalStore.RdbStore | null = null;

function requireConfig(): preferences.Preferences {
  if (!configStore) {
    throw new Error('Store 未初始化：先调用 initStore');
  }
  return configStore;
}

function requireDb(): relationalStore.RdbStore {
  if (!db) {
    throw new Error('Store 未初始化：先调用 initStore');
  }
  return db;
}

export async function initStore(context: Context): Promise<void> {
  configStore = await preferences.getPreferences(context, CONFIG_STORE);
  // securityLevel 为 StoreConfig 必填字段（文档标注非可选）；快照仅含用量统计，取最低级 S1
  const config: relationalStore.StoreConfig = {
    name: DB_NAME,
    securityLevel: relationalStore.SecurityLevel.S1,
  };
  db = await relationalStore.getRdbStore(context, config);
  // schema 与桌面版 db.rs 逐字段一致
  await db.executeSql(
    `CREATE TABLE IF NOT EXISTS dashboard_snapshot (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      saved_at TEXT NOT NULL,
      today_yuan REAL NOT NULL,
      total_tokens INTEGER NOT NULL,
      remaining REAL NOT NULL,
      used REAL NOT NULL,
      total REAL NOT NULL,
      request_count INTEGER
    )`
  );
  // 告警账本：alert_state 是跨进程去重（前台刷新与 WorkScheduler 扩展各一进程，重启也不复位），
  // alert_delivery 只增不改地记下每次投递结果与失败原因
  await db.executeSql(
    `CREATE TABLE IF NOT EXISTS alert_state (
      rule_key TEXT NOT NULL,
      day TEXT NOT NULL,
      count INTEGER NOT NULL,
      last_fired_at TEXT NOT NULL,
      PRIMARY KEY (rule_key, day)
    )`
  );
  await db.executeSql(
    `CREATE TABLE IF NOT EXISTS alert_delivery (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      rule_key TEXT NOT NULL,
      day TEXT NOT NULL,
      channel TEXT NOT NULL,
      ok INTEGER NOT NULL,
      error TEXT,
      delivered_at TEXT NOT NULL
    )`
  );
  await db.executeSql(
    `CREATE INDEX IF NOT EXISTS idx_alert_delivery_day ON alert_delivery(day)`
  );
}

export async function getCookie(): Promise<string | null> {
  const stored = await requireConfig().get(KEY_COOKIE, '') as string;
  if (stored.length === 0) {
    return null;
  }
  return decryptSecret(stored);
}

export async function saveCookie(plain: string, source: string, expiresAt: string | null = null): Promise<void> {
  const store = requireConfig();
  await store.put(KEY_COOKIE, await encryptSecret(plain));
  await store.put(KEY_COOKIE_SOURCE, source);
  // 有效期与 Cookie 同写同清：新凭据没带到期时间就删掉旧值，避免展示上一轮凭据的期限
  if (expiresAt !== null && expiresAt.length > 0) {
    await store.put(KEY_COOKIE_EXPIRES_AT, expiresAt);
  } else {
    await store.delete(KEY_COOKIE_EXPIRES_AT);
  }
  await store.flush();
}

// 官网下发的凭据到期时间（本地时区 RFC3339）；未下发时为 null，仅用于展示
export async function getCookieExpiresAt(): Promise<string | null> {
  const stored = await requireConfig().get(KEY_COOKIE_EXPIRES_AT, '') as string;
  return stored.length === 0 ? null : stored;
}

export async function getApiKey(): Promise<string | null> {
  const stored = await requireConfig().get(KEY_API_KEY, '') as string;
  if (stored.length === 0) {
    return null;
  }
  return decryptSecret(stored);
}

export async function saveApiKey(plain: string): Promise<void> {
  const store = requireConfig();
  await store.put(KEY_API_KEY, await encryptSecret(plain));
  await store.flush();
}

export async function getCookieSource(): Promise<string> {
  return await requireConfig().get(KEY_COOKIE_SOURCE, 'manual') as string;
}

export async function clearCredentials(): Promise<void> {
  const store = requireConfig();
  await store.delete(KEY_COOKIE);
  await store.delete(KEY_API_KEY);
  await store.delete(KEY_COOKIE_SOURCE);
  await store.delete(KEY_COOKIE_EXPIRES_AT);
  await store.flush();
}

// ═══ 告警配置与账本 ═══
// 配置走 preferences（与凭据同库，但都是明文本机配置），账本走 RDB 两张告警表。
// 键名与表结构与桌面版 db.rs 一一对应；两端各存本机、互不同步。

async function getNumber(key: string, fallback: number): Promise<number> {
  const stored = await requireConfig().get(key, fallback) as number;
  const value = Number(stored);
  return Number.isFinite(value) ? value : fallback;
}

async function getFlag(key: string, fallback: boolean): Promise<boolean> {
  return await requireConfig().get(key, fallback) as boolean;
}

/// 读取告警参数；未落库的键取默认值，落库的脏值一律夹回合法区间
export async function getAlertSettings(): Promise<AlertSettings> {
  const defaults = defaultSettings();
  const settings: AlertSettings = {
    enabled: await getFlag(KEY_ALERT_ENABLED, defaults.enabled),
    quotaPercent: await getNumber(KEY_ALERT_QUOTA_PERCENT, defaults.quotaPercent),
    runoutDays: await getNumber(KEY_ALERT_RUNOUT_DAYS, defaults.runoutDays),
    spikeMultiplier: await getNumber(KEY_ALERT_SPIKE_MULTIPLIER, defaults.spikeMultiplier),
    maxFiresPerDay: await getNumber(KEY_ALERT_MAX_FIRES_PER_DAY, defaults.maxFiresPerDay),
  };
  return sanitizeSettings(settings);
}

/// 写入前一律 sanitizeSettings，并回读存进去的值——界面据此显示，不会亮出一个并不生效的数字
export async function setAlertSettings(settings: AlertSettings): Promise<AlertSettings> {
  const saved = sanitizeSettings(settings);
  const store = requireConfig();
  await store.put(KEY_ALERT_ENABLED, saved.enabled);
  await store.put(KEY_ALERT_QUOTA_PERCENT, saved.quotaPercent);
  await store.put(KEY_ALERT_RUNOUT_DAYS, saved.runoutDays);
  await store.put(KEY_ALERT_SPIKE_MULTIPLIER, saved.spikeMultiplier);
  await store.put(KEY_ALERT_MAX_FIRES_PER_DAY, saved.maxFiresPerDay);
  await store.flush();
  return getAlertSettings();
}

/// 启用的规则 key；未落库即默认开启，不要求用户先动过设置
export async function getAlertRulesEnabled(): Promise<string[]> {
  const enabled: string[] = [];
  for (const key of RULE_KEYS) {
    if (await getFlag(`${KEY_ALERT_RULE_PREFIX}${key}`, true)) {
      enabled.push(key);
    }
  }
  return enabled;
}

/// 未传即保持现状；传了必须全在已知规则集合内（未知 key 抛错，同桌面版 resolve_rules_enabled）
export async function setAlertRulesEnabled(requested: string[] | null): Promise<string[]> {
  const resolved = resolveRulesEnabled(await getAlertRulesEnabled(), requested);
  const store = requireConfig();
  for (const key of RULE_KEYS) {
    await store.put(`${KEY_ALERT_RULE_PREFIX}${key}`, resolved.includes(key));
  }
  await store.flush();
  return resolved;
}

export interface AlertChannels {
  notification: boolean;
  meow: boolean;
  meowNickname: string | null;
}

/// 渠道开关 + MeoW 昵称。昵称只是收件路由名，按明文配置存、界面原样回显；
/// 手机端唯一的凭据仍是 Cookie / API Key（SMTP 邮件是桌面端专属渠道）
export async function getAlertChannels(): Promise<AlertChannels> {
  const nickname = await requireConfig().get(KEY_MEOW_NICKNAME, '') as string;
  return {
    notification: await getFlag(KEY_ALERT_CH_NOTIFICATION, true),
    meow: await getFlag(KEY_ALERT_CH_MEOW, false),
    meowNickname: nickname.length > 0 ? nickname : null,
  };
}

export interface ChannelInput {
  notification: boolean;
  meow: boolean;
  meowNickname: string;
}

/// 开关与昵称每次整体覆盖（界面改了什么就提交什么），昵称清空即删除
export async function setAlertChannels(input: ChannelInput): Promise<AlertChannels> {
  const store = requireConfig();
  await store.put(KEY_ALERT_CH_NOTIFICATION, input.notification);
  await store.put(KEY_ALERT_CH_MEOW, input.meow);
  const nickname = input.meowNickname.trim();
  if (nickname.length === 0) {
    await store.delete(KEY_MEOW_NICKNAME);
  } else {
    await store.put(KEY_MEOW_NICKNAME, nickname);
  }
  await store.flush();
  return getAlertChannels();
}

/// 当日各规则已投次数（未知 rule_key 由上层忽略）
export async function getAlertDayCounts(day: string): Promise<RuleCount[]> {
  let resultSet: relationalStore.ResultSet | null = null;
  try {
    resultSet = await requireDb().querySql(
      'SELECT rule_key, count FROM alert_state WHERE day=?', [day]);
    const keyIdx = resultSet.getColumnIndex('rule_key');
    const countIdx = resultSet.getColumnIndex('count');
    const rows: RuleCount[] = [];
    while (resultSet.goToNextRow()) {
      rows.push({ key: resultSet.getString(keyIdx), count: resultSet.getLong(countIdx) });
    }
    return rows;
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    throw new Error(`告警账本查询失败：${reason}`);
  } finally {
    if (resultSet) {
      resultSet.close();
    }
  }
}

/// 记一次「这条规则今天投过了」：同 (规则, 日) 累加，跨进程与重启都不复位
export async function recordAlertFire(ruleKey: string, day: string, firedAt: string): Promise<void> {
  await requireDb().executeSql(
    `INSERT INTO alert_state(rule_key, day, count, last_fired_at) VALUES(?, ?, 1, ?)
     ON CONFLICT(rule_key, day)
     DO UPDATE SET count=count+1, last_fired_at=excluded.last_fired_at`,
    [ruleKey, day, firedAt]);
}

/// 渠道投递结果，只增不改；error 由调用方保证已截断且不含凭据
export async function recordAlertDelivery(ruleKey: string, day: string, channel: string,
  ok: boolean, error: string | null, deliveredAt: string): Promise<void> {
  await requireDb().executeSql(
    `INSERT INTO alert_delivery(rule_key, day, channel, ok, error, delivered_at)
     VALUES(?, ?, ?, ?, ?, ?)`,
    [ruleKey, day, channel, ok ? 1 : 0, error, deliveredAt]);
}

/// 当日某规则已失败几次：给「可重试的投递失败」封顶，避免每轮刷新撞同一堵墙
export async function countAlertFailures(ruleKey: string, day: string): Promise<number> {
  let resultSet: relationalStore.ResultSet | null = null;
  try {
    resultSet = await requireDb().querySql(
      'SELECT COUNT(*) AS failures FROM alert_delivery WHERE rule_key=? AND day=? AND ok=0',
      [ruleKey, day]);
    if (!resultSet.goToNextRow()) {
      return 0;
    }
    return resultSet.getLong(resultSet.getColumnIndex('failures'));
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    throw new Error(`告警失败次数查询失败：${reason}`);
  } finally {
    if (resultSet) {
      resultSet.close();
    }
  }
}

/// 当日各渠道的投递明细（设置页复盘用，按时间倒序）
export interface DeliveryLog {
  ruleKey: string;
  channel: string;
  ok: boolean;
  error: string | null;
  deliveredAt: string;
}

export async function recentAlertDeliveries(day: string, limit: number): Promise<DeliveryLog[]> {
  let resultSet: relationalStore.ResultSet | null = null;
  try {
    resultSet = await requireDb().querySql(
      `SELECT rule_key, channel, ok, error, delivered_at
       FROM alert_delivery WHERE day=? ORDER BY id DESC LIMIT ?`,
      [day, limit]);
    const ruleIdx = resultSet.getColumnIndex('rule_key');
    const channelIdx = resultSet.getColumnIndex('channel');
    const okIdx = resultSet.getColumnIndex('ok');
    const errorIdx = resultSet.getColumnIndex('error');
    const atIdx = resultSet.getColumnIndex('delivered_at');
    const rows: DeliveryLog[] = [];
    while (resultSet.goToNextRow()) {
      rows.push({
        ruleKey: resultSet.getString(ruleIdx),
        channel: resultSet.getString(channelIdx),
        ok: resultSet.getLong(okIdx) === 1,
        error: resultSet.isColumnNull(errorIdx) ? null : resultSet.getString(errorIdx),
        deliveredAt: resultSet.getString(atIdx),
      });
    }
    return rows;
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    throw new Error(`投递明细查询失败：${reason}`);
  } finally {
    if (resultSet) {
      resultSet.close();
    }
  }
}

export async function saveSnapshot(row: SnapshotRow): Promise<void> {
  const values: relationalStore.ValuesBucket = {
    'saved_at': row.savedAt,
    'today_yuan': row.todayYuan,
    'total_tokens': row.totalTokens,
    'remaining': row.remaining,
    'used': row.used,
    'total': row.total,
    'request_count': row.requestCount,
  };
  // insert 异常时 reject；返回负值表示写入失败（如约束冲突），不静默吞错
  const rowId = await requireDb().insert('dashboard_snapshot', values);
  if (rowId < 0) {
    throw new Error('快照写入失败');
  }
}

// 查询区间内每个本地日的日末快照（同一天取 MAX(id) 最新一条）
// RdbPredicates 不支持子查询，改用 querySql 原生 SQL
// （文档查证：relationalStore.RdbStore.querySql(sql, bindArgs?): Promise<ResultSet>）
export async function getDailySnapshots(startDate: string, endDate: string): Promise<DailySnapshot[]> {
  let resultSet: relationalStore.ResultSet | null = null;
  try {
    resultSet = await requireDb().querySql(
      `SELECT date(saved_at, 'localtime') AS day, today_yuan, total_tokens, remaining, request_count
       FROM dashboard_snapshot
       WHERE id IN (SELECT MAX(id) FROM dashboard_snapshot
                    WHERE date(saved_at, 'localtime') BETWEEN ? AND ?
                    GROUP BY date(saved_at, 'localtime'))
       ORDER BY day`,
      [startDate, endDate]);
    // 列索引一次取出，避免逐行重复解析列名
    const dayIdx = resultSet.getColumnIndex('day');
    const yuanIdx = resultSet.getColumnIndex('today_yuan');
    const tokensIdx = resultSet.getColumnIndex('total_tokens');
    const remainingIdx = resultSet.getColumnIndex('remaining');
    const requestIdx = resultSet.getColumnIndex('request_count');
    const rows: DailySnapshot[] = [];
    while (resultSet.goToNextRow()) {
      rows.push({
        date: resultSet.getString(dayIdx),
        yuan: resultSet.getDouble(yuanIdx),
        tokens: resultSet.getLong(tokensIdx),
        remaining: resultSet.getDouble(remainingIdx),
        // request_count 列允许 NULL，先判空再取值
        requestCount: resultSet.isColumnNull(requestIdx) ? null : resultSet.getLong(requestIdx),
      });
    }
    return rows;
  } catch (error) {
    // 查询或取值异常统一包装为可读错误，不静默吞错
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    throw new Error(`快照查询失败：${reason}`);
  } finally {
    if (resultSet) {
      try {
        resultSet.close();
      } catch (closeError) {
        // 清理失败不影响主流程结果，仅记录原因
        const reason = closeError instanceof Error ? closeError.message : JSON.stringify(closeError);
        console.error(`关闭 ResultSet 失败：${reason}`);
      }
    }
  }
}

// 'YYYY-MM-DD' 加一天；格式非法返回空串，由调用方按"不相邻"处理
// 本地构造 new Date(y, m-1, d+1) 自动进位跨月/跨年，规避 UTC 解析陷阱
function nextDayString(dateStr: string): string {
  const match = dateStr.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!match) {
    return '';
  }
  const next = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]) + 1, 0, 0, 0, 0);
  const month = (next.getMonth() + 1).toString().padStart(2, '0');
  const day = next.getDate().toString().padStart(2, '0');
  return `${next.getFullYear()}-${month}-${day}`;
}

// 由快照序列派生每日请求数增量：首日、日期缺档（不相邻）、任一侧请求数为空均返回 null
export function deriveDailyRequests(snapshots: DailySnapshot[]): Array<number | null> {
  const result: Array<number | null> = [];
  for (let i = 0; i < snapshots.length; i++) {
    const cur = snapshots[i];
    if (i === 0) {
      result.push(null);
      continue;
    }
    const prev = snapshots[i - 1];
    if (prev.requestCount === null || cur.requestCount === null
      || nextDayString(prev.date) !== cur.date) {
      result.push(null);
    } else {
      result.push(cur.requestCount - prev.requestCount);
    }
  }
  return result;
}

// 本地快照区间汇总：avg 除有快照天数（与桌面版 db.rs summarize_local 同口径，
// 降级模式下日均不偏高）；peakDate 全 0 或空输入保持空串（同桌面版，UI 显示 '--'、导出空串）
export function summarizeLocal(snapshots: DailySnapshot[]): LocalSummary {
  let totalYuan = 0;
  let totalTokens = 0;
  let daysWithUsage = 0;
  let peakYuan = 0;
  // 全 0 时不回退快照日期，与桌面版 summarize_local 一致
  let peakDate = '';
  for (const snap of snapshots) {
    totalYuan += snap.yuan;
    totalTokens += snap.tokens;
    if (snap.yuan > 0 || snap.tokens > 0) {
      daysWithUsage++;
    }
    if (snap.yuan > peakYuan) {
      peakYuan = snap.yuan;
      peakDate = snap.date;
    }
  }
  // 分母为有快照天数而非有用量天数，空区间返回 0
  const avgYuan = snapshots.length > 0 ? totalYuan / snapshots.length : 0;
  return {
    totalYuan: totalYuan,
    avgYuan: avgYuan,
    peakYuan: peakYuan,
    peakDate: peakDate,
    totalTokens: totalTokens,
    daysWithData: snapshots.length,
    daysWithUsage: daysWithUsage,
  };
}
