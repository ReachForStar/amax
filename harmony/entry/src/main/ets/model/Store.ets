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
