import { http } from '@kit.NetworkKit';
import { ApiError, classifyHttpError } from '../common/Format';

const BASE_URL = 'https://ai.amaxsmp.com';
const QUOTA_PER_YUAN = 500_000;
const TIMEOUT_MS = 30_000;

export interface DashboardData {
  displayName: string;
  requestCount: number;
  todayYuan: number;
  todayTokens: number;
  todayInput: number;
  todayOutput: number;
  remaining: number;
  used: number;
  total: number;
  percent: number;
  logsAvailable: boolean;
}

interface UserInfo {
  id: number;
  quota: number;
  used_quota: number;
  request_count: number;
  display_name: string;
}

// by-model 响应顶层 summary（与 user/self 的 data 包裹不同，顶层即 { summary, models, daily }）
interface UsageSummaryRaw {
  quota: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
}

// ArkTS 禁止对象字面量类型（arkts-no-obj-literals-as-types），泛型实参与返回值均须用显式接口
interface UserSelfPayload {
  data: UserInfo;
}

interface UsagePayload {
  summary: UsageSummaryRaw;
}

interface UsageQuery {
  start_time: number;
  end_time: number;
  user_id: string;
  status: string;
}

interface TodayRange {
  start: number;
  end: number;
}

// ---- 统计页区间聚合（与桌面版 src-tauri/src/api.rs 同口径） ----

// by-model 响应的按日明细原始行（每模型每天一条，date 为本地时区 0 点 Unix 秒）
export interface DailyUsageRaw {
  date: number;
  model?: string;
  quota: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
}

// by-model 响应的模型维度明细原始行
export interface ModelUsageRaw {
  model: string;
  quota: number;
  total_tokens: number;
  request_count: number;
}

// 统计页每日消耗（已按区间补零）
export interface UsageDaily {
  date: string;
  yuan: number;
  tokens: number;
  inputTokens: number;
  outputTokens: number;
}

// 统计页模型分布项
export interface UsageModel {
  model: string;
  yuan: number;
  tokens: number;
  requestCount: number;
  percent: number;
}

// 统计页区间汇总
export interface UsageSummary {
  totalYuan: number;
  avgYuan: number;
  peakYuan: number;
  peakDate: string;
  totalTokens: number;
  requestCount: number;
  daysWithUsage: number;
}

// 统计页官方数据聚合结果
export interface UsageStats {
  daily: UsageDaily[];
  models: UsageModel[];
  summary: UsageSummary;
  percentBasis: string;
}

// by-model 完整响应：顶层即 { summary, models, daily }，无 data 包裹（与 user/self 不同）
// models/daily 可能缺失（接口契约变更时），声明为可选由调用方兜底空数组
interface UsageStatsPayload {
  summary: UsageSummaryRaw;
  models?: ModelUsageRaw[];
  daily?: DailyUsageRaw[];
}

function commonHeaders(cookie: string): Record<string, string> {
  return {
    'User-Agent': 'Mozilla/5.0',
    'Accept': 'application/json',
    'x-company': 'AMAX',
    'Cookie': cookie,
  };
}

// 单次请求封装：http 对象一次性使用，完成即 destroy
function request(url: string, cookie: string, method: http.RequestMethod,
  body?: object): Promise<http.HttpResponse> {
  return new Promise((resolve, reject) => {
    const req = http.createHttp();
    req.request(url, {
      method: method,
      header: commonHeaders(cookie),
      extraData: body === undefined ? undefined : JSON.stringify(body),
      connectTimeout: TIMEOUT_MS,
      readTimeout: TIMEOUT_MS,
    }, (err, data: http.HttpResponse) => {
      req.destroy();
      if (err) {
        reject(new ApiError('网络连接失败，请检查网络'));
        return;
      }
      resolve(data);
    });
  });
}

function parseBody<T>(resp: http.HttpResponse): T {
  const text = typeof resp.result === 'string' ? resp.result : JSON.stringify(resp.result);
  return JSON.parse(text) as T;
}

// 区间上限（天），超出视为非法查询；约 3 年，与统计页最大可选区间对应
const MAX_RANGE_DAYS = 1096;

// 解析 'YYYY-MM-DD' 为本地日期。禁止 new Date('YYYY-MM-DD')（ArkTS 按 UTC 解析的陷阱），
// 必须用 new Date(y, m-1, d) 本地构造；同时拒绝 2026-02-30 这类滚动溢出的非法日期
function parseLocalDate(dateStr: string): Date {
  const match = dateStr.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!match) {
    throw new ApiError('日期格式错误，应为 YYYY-MM-DD');
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day, 0, 0, 0, 0);
  // 非法日期会被 Date 滚动进位（如 02-30 → 03-02），回读字段做往返校验
  if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day) {
    throw new ApiError(`日期不存在：${dateStr}`);
  }
  return date;
}

function formatLocalDate(date: Date): string {
  const month = (date.getMonth() + 1).toString().padStart(2, '0');
  const day = date.getDate().toString().padStart(2, '0');
  return `${date.getFullYear()}-${month}-${day}`;
}

// 区间校验：起始不晚于结束，且跨度不超过 MAX_RANGE_DAYS 天
function assertValidRange(start: Date, end: Date): void {
  if (start.getTime() > end.getTime()) {
    throw new ApiError('起始日期不能晚于结束日期');
  }
  const spanDays = Math.round((end.getTime() - start.getTime()) / 86_400_000);
  if (spanDays > MAX_RANGE_DAYS) {
    throw new ApiError(`查询区间不能超过 ${MAX_RANGE_DAYS} 天`);
  }
}

// 本地时区区间起止 Unix 秒：起 00:00:00、止 23:59:59（与桌面版 api.rs 同口径）
export function dateRangeToUnix(startDate: string, endDate: string): TodayRange {
  const start = parseLocalDate(startDate);
  const end = parseLocalDate(endDate);
  assertValidRange(start, end);
  const endLast = new Date(end.getFullYear(), end.getMonth(), end.getDate(), 23, 59, 59, 0);
  return { start: Math.floor(start.getTime() / 1000), end: Math.floor(endLast.getTime() / 1000) };
}

// 本地时区当日起止 Unix 秒；todayRange 重构为 dateRangeToUnix 复用，保留原调用兼容
function todayRange(): TodayRange {
  const now = new Date();
  const today = formatLocalDate(now);
  return dateRangeToUnix(today, today);
}

// 聚合官网 by-model 响应：按日跨模型求和、补零、派生汇总与占比（移植桌面版 aggregate_usage）
export function aggregateUsage(rawDaily: DailyUsageRaw[], rawModels: ModelUsageRaw[],
  startDate: string, endDate: string): UsageStats {
  const start = parseLocalDate(startDate);
  const end = parseLocalDate(endDate);
  assertValidRange(start, end);

  // 按本地日期分组求和：时间戳经 new Date(ts*1000) 取本地 y/m/d；非法时间戳跳过
  const byDay = new Map<string, UsageDaily>();
  const seen = new Set<string>();
  for (const raw of rawDaily) {
    if (!Number.isFinite(raw.date)) {
      continue;
    }
    const dateStr = formatLocalDate(new Date(raw.date * 1000));
    seen.add(dateStr);
    let entry = byDay.get(dateStr);
    if (!entry) {
      entry = { date: dateStr, yuan: 0, tokens: 0, inputTokens: 0, outputTokens: 0 };
      byDay.set(dateStr, entry);
    }
    entry.yuan += (raw.quota ?? 0) / QUOTA_PER_YUAN;
    entry.tokens += raw.total_tokens ?? 0;
    entry.inputTokens += raw.input_tokens ?? 0;
    entry.outputTokens += raw.output_tokens ?? 0;
  }
  const daysWithUsage = seen.size;

  // 逐日补齐区间，无使用日填 0
  const daily: UsageDaily[] = [];
  let cursor = start;
  while (cursor.getTime() <= end.getTime()) {
    const key = formatLocalDate(cursor);
    const row = byDay.get(key);
    daily.push(row ?? { date: key, yuan: 0, tokens: 0, inputTokens: 0, outputTokens: 0 });
    cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate() + 1, 0, 0, 0, 0);
  }

  // 汇总派生：avg 除有使用日天数；peak 全 0 回退 endDate；requestCount = Σ models
  let totalYuan = 0;
  let totalTokens = 0;
  for (const row of daily) {
    totalYuan += row.yuan;
    totalTokens += row.tokens;
  }
  const avgYuan = daysWithUsage > 0 ? totalYuan / daysWithUsage : 0;
  let peakYuan = 0;
  let peakFound = false;
  let peakDate = endDate;
  for (const row of daily) {
    if (!peakFound || row.yuan > peakYuan) {
      peakYuan = row.yuan;
      peakFound = true;
      if (row.yuan > 0) {
        peakDate = row.date;
      }
    }
  }
  let requestCount = 0;
  for (const m of rawModels) {
    requestCount += m.request_count ?? 0;
  }

  // 模型占比：Σquota <= 0 时切换 tokens 口径；percent 保留 1 位小数，按主指标降序
  let totalQuota = 0;
  for (const m of rawModels) {
    totalQuota += m.quota ?? 0;
  }
  const useTokensBasis = totalQuota <= 0;
  const percentBasis = useTokensBasis ? 'tokens' : 'quota';
  let basisTotal = 0;
  for (const m of rawModels) {
    basisTotal += useTokensBasis ? (m.total_tokens ?? 0) : (m.quota ?? 0);
  }
  const models: UsageModel[] = [];
  for (const m of rawModels) {
    const basisValue = useTokensBasis ? (m.total_tokens ?? 0) : (m.quota ?? 0);
    const percent = basisTotal > 0 ? Math.round(basisValue / basisTotal * 1000) / 10 : 0;
    models.push({
      model: m.model,
      yuan: (m.quota ?? 0) / QUOTA_PER_YUAN,
      tokens: m.total_tokens ?? 0,
      requestCount: m.request_count ?? 0,
      percent: percent,
    });
  }
  models.sort((a: UsageModel, b: UsageModel): number => {
    const av = useTokensBasis ? a.tokens : a.yuan;
    const bv = useTokensBasis ? b.tokens : b.yuan;
    return bv - av;
  });

  const statsSummary: UsageSummary = {
    totalYuan: totalYuan,
    avgYuan: avgYuan,
    peakYuan: peakYuan,
    peakDate: peakDate,
    totalTokens: totalTokens,
    requestCount: requestCount,
    daysWithUsage: daysWithUsage,
  };
  return { daily: daily, models: models, summary: statsSummary, percentBasis: percentBasis };
}

// 拉取区间用量并聚合为统计页数据：复用现有 request 封装与错误分级
export async function fetchUsageStats(cookie: string, startDate: string,
  endDate: string): Promise<UsageStats> {
  if (cookie.length === 0) {
    throw new ApiError('Cookie 无效或已过期，请重新登录', true);
  }
  // 先校验区间，非法日期/跨度直接抛错，避免浪费网络请求
  const range = dateRangeToUnix(startDate, endDate);

  // by-model 必须携带字符串形式的 user_id，先取账户信息（与 fetchDashboard 相同路径）
  const userResp = await request(`${BASE_URL}/api/user/self`, cookie, http.RequestMethod.GET);
  if (userResp.responseCode !== 200) {
    throw new ApiError(classifyHttpError(userResp.responseCode),
      userResp.responseCode === 401 || userResp.responseCode === 403);
  }
  let user: UserInfo;
  try {
    user = (parseBody<UserSelfPayload>(userResp)).data;
  } catch (error) {
    throw new ApiError('数据解析失败');
  }
  if (!user) {
    throw new ApiError('数据解析失败');
  }

  const query: UsageQuery = {
    start_time: range.start,
    end_time: range.end,
    user_id: String(user.id),
    status: 'success',
  };
  const logsResp = await request(`${BASE_URL}/v1/logs/token-usage/by-model`, cookie,
    http.RequestMethod.POST, query);
  if (logsResp.responseCode !== 200) {
    throw new ApiError(classifyHttpError(logsResp.responseCode),
      logsResp.responseCode === 401 || logsResp.responseCode === 403);
  }
  // 官网响应顶层即 { summary, models, daily }，无 data 包裹
  let payload: UsageStatsPayload;
  try {
    payload = parseBody<UsageStatsPayload>(logsResp);
  } catch (error) {
    throw new ApiError('用量数据解析失败');
  }
  return aggregateUsage(payload.daily ?? [], payload.models ?? [], startDate, endDate);
}

export async function fetchDashboard(cookie: string): Promise<DashboardData> {
  if (cookie.length === 0) {
    throw new ApiError('Cookie 无效或已过期，请重新登录', true);
  }
  // 1. 账户信息
  const userResp = await request(`${BASE_URL}/api/user/self`, cookie, http.RequestMethod.GET);
  if (userResp.responseCode !== 200) {
    throw new ApiError(classifyHttpError(userResp.responseCode),
      userResp.responseCode === 401 || userResp.responseCode === 403);
  }
  // HTTP 200 但响应体非 JSON 时 parseBody 抛原生 SyntaxError，统一收敛为 ApiError（对齐桌面版"数据解析失败"）
  let user: UserInfo;
  try {
    user = (parseBody<UserSelfPayload>(userResp)).data;
  } catch (error) {
    throw new ApiError('数据解析失败');
  }
  const totalQuota = user.quota + user.used_quota;
  const remaining = user.quota / QUOTA_PER_YUAN;
  const used = user.used_quota / QUOTA_PER_YUAN;
  const total = totalQuota / QUOTA_PER_YUAN;
  const percent = totalQuota > 0
    ? Math.min(100, Math.max(0, user.quota / totalQuota * 100))
    : 0;

  // 2. 当日用量汇总：失败仍返回额度（logsAvailable=false），与桌面版容错一致
  let summary: UsageSummaryRaw = { quota: 0, total_tokens: 0, input_tokens: 0, output_tokens: 0 };
  let logsAvailable = false;
  try {
    const range = todayRange();
    const query: UsageQuery = {
      start_time: range.start,
      end_time: range.end,
      user_id: String(user.id),
      status: 'success',
    };
    const logsResp = await request(`${BASE_URL}/v1/logs/token-usage/by-model`, cookie,
      http.RequestMethod.POST, query);
    if (logsResp.responseCode === 200) {
      const payload = parseBody<UsagePayload>(logsResp);
      if (payload.summary) {
        summary = payload.summary;
        logsAvailable = true;
      }
    } else {
      console.error(`日志汇总异常：HTTP ${logsResp.responseCode}，保留账户额度数据`);
    }
  } catch (error) {
    // Error 实例 JSON.stringify 得 {}，取 message 才能保留失败原因
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    console.error(`日志汇总失败，保留账户额度数据：${reason}`);
  }

  return {
    displayName: user.display_name,
    requestCount: user.request_count,
    todayYuan: summary.quota / QUOTA_PER_YUAN,
    todayTokens: summary.total_tokens,
    todayInput: summary.input_tokens,
    todayOutput: summary.output_tokens,
    remaining,
    used,
    total,
    percent,
    logsAvailable,
  };
}
