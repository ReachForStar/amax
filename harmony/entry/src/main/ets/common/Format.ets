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
