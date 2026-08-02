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

interface UsageSummary {
  quota: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
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

// 本地时区当日起止 Unix 秒（与桌面版 api.rs 同口径）
function todayRange(): { start: number; end: number } {
  const now = new Date();
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 0, 0, 0, 0);
  const end = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 23, 59, 59, 999);
  return { start: Math.floor(start.getTime() / 1000), end: Math.floor(end.getTime() / 1000) };
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
    user = (parseBody<{ data: UserInfo }>(userResp)).data;
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
  let summary: UsageSummary = { quota: 0, total_tokens: 0, input_tokens: 0, output_tokens: 0 };
  let logsAvailable = false;
  try {
    const range = todayRange();
    const logsResp = await request(`${BASE_URL}/v1/logs/token-usage/by-model`, cookie,
      http.RequestMethod.POST, {
        start_time: range.start,
        end_time: range.end,
        user_id: String(user.id),
        status: 'success',
      });
    if (logsResp.responseCode === 200) {
      const payload = parseBody<{ summary: UsageSummary }>(logsResp);
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
