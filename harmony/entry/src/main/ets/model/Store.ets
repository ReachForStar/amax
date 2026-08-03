// 存储层：preferences 库 config 存认证凭据（经 Secret 加密），RDB 库 amax_dashboard.db 存刷新快照
// schema 与桌面版 src-tauri/src/db.rs 的 dashboard_snapshot 表逐字段一致
// initStore 由 EntryAbility 启动时调用一次；未初始化即调用其余接口抛明确错误
import { preferences, relationalStore } from '@kit.ArkData';
import { encryptSecret, decryptSecret } from './Secret';

const CONFIG_STORE = 'config';
const DB_NAME = 'amax_dashboard.db';
const KEY_COOKIE = 'cookie';
const KEY_API_KEY = 'api_key';
const KEY_COOKIE_SOURCE = 'cookie_source';

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
}

export async function getCookie(): Promise<string | null> {
  const stored = await requireConfig().get(KEY_COOKIE, '') as string;
  if (stored.length === 0) {
    return null;
  }
  return decryptSecret(stored);
}

export async function saveCookie(plain: string, source: string): Promise<void> {
  const store = requireConfig();
  await store.put(KEY_COOKIE, await encryptSecret(plain));
  await store.put(KEY_COOKIE_SOURCE, source);
  await store.flush();
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
  await store.flush();
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
       WHERE id IN (SELECT MAX(id) FROM dashboard_snapshot GROUP BY date(saved_at, 'localtime'))
         AND day BETWEEN ? AND ?
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
