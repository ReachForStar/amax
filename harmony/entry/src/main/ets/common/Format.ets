// 纯函数：Token 与金额格式化、Cookie 解析、HTTP 错误分级、日期字符串工具
export function formatTokens(n: number): string {
  if (n >= 1e6) {
    return (n / 1e6).toFixed(2) + 'M';
  }
  if (n >= 1e3) {
    return (n / 1e3).toFixed(1) + 'K';
  }
  return String(n);
}

export function formatYuan(n: number): string {
  return '¥' + n.toFixed(6);
}

// 从整串 Cookie（"a=1; session=xxx; b=2"）提取 session 值
export function parseSessionCookie(cookieStr: string): string | null {
  if (!cookieStr) {
    return null;
  }
  const match = cookieStr.match(/(?:^|;\s*)session=([^;]+)/);
  if (!match || match[1].length === 0) {
    return null;
  }
  return match[1];
}

export function classifyHttpError(code: number): string {
  if (code === 401 || code === 403) {
    return 'Cookie 无效或已过期，请重新登录';
  }
  if (code === 429) {
    return '请求过于频繁，请稍后重试';
  }
  return `服务异常（HTTP ${code}）`;
}

// API 层统一错误：message 为分级后的用户可见文案
export class ApiError extends Error {
  readonly isAuthError: boolean;

  constructor(message: string, isAuthError: boolean = false) {
    super(message);
    this.isAuthError = isAuthError;
  }
}

// 本地时区 RFC3339：动态计算偏移，任意时区正确（与桌面版 Local::now().to_rfc3339() 同口径）
// 快照 saved_at 的统一生成入口：前台刷新（DashboardPage）与后台延迟刷新（WorkScheduler 扩展）共用，
// 保证两条路径写入的快照时间串格式完全一致（getDailySnapshots 依赖 date(saved_at,'localtime') 解析）
export function localRfc3339(date: Date): string {
  const offsetMin = -date.getTimezoneOffset();
  const sign = offsetMin >= 0 ? '+' : '-';
  const abs = Math.abs(offsetMin);
  const hh = String(Math.floor(abs / 60)).padStart(2, '0');
  const mm = String(abs % 60).padStart(2, '0');
  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}${sign}${hh}:${mm}`;
}

// Cookie 到期时间戳解析：ArkWeb 的 WebHttpCookie.expiresDate 为 RFC-7601 GMT 串
// （"Wdy, DD Mon YYYY HH:MM:SS GMT"），未设 Expires 的 Cookie 给 "-1"，故数字分支同时兜住 -1/0（视为无到期），
// 日期串交 Date.parse；无法识别时返回 null，由调用方按"官网未下发有效期"处理，不做任何本地推断。
function parseExpiryMillis(raw: string): number | null {
  const value = raw.trim();
  if (/^-?\d+(\.\d+)?$/.test(value)) {
    const seconds = Number(value);
    if (seconds <= 0) {
      return null;
    }
    // 秒级时间戳 10 位、毫秒级 13 位；量级异常时按毫秒解释，避免乘出 5 万年后的假日期
    return seconds < 1e11 ? seconds * 1000 : seconds;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

// 凭据有效期可见性判定：仅接受"属性真实存在且尚未到期"的值。
// 会话 Cookie（浏览器关闭即失效，官网未定过期）、空值、格式不可识别、已过期一律返回 null，
// 有效期只用于展示，绝不作为本地倒计时拦截依据（拦截一律以服务端 401/403 为准）
export function parseCookieExpiry(expiresDate: string, isSessionCookie: boolean, now: Date): string | null {
  if (isSessionCookie || expiresDate.length === 0) {
    return null;
  }
  const millis = parseExpiryMillis(expiresDate);
  if (millis === null || millis <= now.getTime()) {
    return null;
  }
  return localRfc3339(new Date(millis));
}

// 展示用本地时间 'YYYY-MM-DD HH:MM'：有效期只精确到分钟，避免伪精确
export function formatLocalDateTime(date: Date): string {
  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    ` ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

// ---- 日期字符串工具（统计页区间选择用，与桌面版 app.js toDateStr/todayStr/daysAgoStr 同口径） ----

// Date → 本地时区 'YYYY-MM-DD'
function toDateStr(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${date.getFullYear()}-${month}-${day}`;
}

// 本地时区今天 'YYYY-MM-DD'
export function todayStr(): string {
  return toDateStr(new Date());
}

// 含当日的 N 天区间起始日：7 天即今天往前 6 天（与桌面版 daysAgoStr 一致）
export function daysAgoStr(days: number): string {
  const date = new Date();
  date.setDate(date.getDate() - (days - 1));
  return toDateStr(date);
}
