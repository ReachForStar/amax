// 纯函数：Token 与金额格式化、Cookie 解析、HTTP 错误分级
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
