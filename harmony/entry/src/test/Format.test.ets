import { describe, it, expect } from '@ohos/hypium';
import {
  formatTokens,
  formatYuan,
  parseSessionCookie,
  parseCookieExpiry,
  formatLocalDateTime,
  localRfc3339,
  classifyHttpError,
  ApiError
} from '../main/ets/common/Format';

export default function formatTest() {
  describe('formatTest', () => {
    it('formatTokens_millions', 0, () => {
      expect(formatTokens(89936759)).assertEqual('89.94M');
      expect(formatTokens(1000000)).assertEqual('1.00M');
    });
    it('formatTokens_thousands', 0, () => {
      expect(formatTokens(26072)).assertEqual('26.1K');
      expect(formatTokens(1000)).assertEqual('1.0K');
    });
    it('formatTokens_plain', 0, () => {
      expect(formatTokens(999)).assertEqual('999');
      expect(formatTokens(0)).assertEqual('0');
    });
    it('formatYuan_six_decimals', 0, () => {
      expect(formatYuan(0.014014)).assertEqual('¥0.014014');
      expect(formatYuan(0)).assertEqual('¥0.000000');
    });
    it('parseSessionCookie_extracts_value', 0, () => {
      expect(parseSessionCookie('a=1; session=MTc4MzQy; b=2')).assertEqual('MTc4MzQy');
      expect(parseSessionCookie('session=only')).assertEqual('only');
    });
    it('parseSessionCookie_missing_or_empty', 0, () => {
      expect(parseSessionCookie('a=1; b=2')).assertNull();
      expect(parseSessionCookie('')).assertNull();
      expect(parseSessionCookie('session=')).assertNull();
    });
    it('classifyHttpError_tiers', 0, () => {
      expect(classifyHttpError(401)).assertEqual('Cookie 无效或已过期，请重新登录');
      expect(classifyHttpError(403)).assertEqual('Cookie 无效或已过期，请重新登录');
      expect(classifyHttpError(429)).assertEqual('请求过于频繁，请稍后重试');
      expect(classifyHttpError(500)).assertEqual('服务异常（HTTP 500）');
    });
    it('apiError_is_error_with_message', 0, () => {
      const error = new ApiError('msg');
      expect(error instanceof Error).assertTrue();
      expect(error.message).assertEqual('msg');
    });
    it('apiError_auth_flag_default_false', 0, () => {
      expect(new ApiError('msg').isAuthError).assertFalse();
    });
    it('apiError_auth_flag_explicit_true', 0, () => {
      expect(new ApiError('msg', true).isAuthError).assertTrue();
    });
    it('parseCookieExpiry_session_cookie_has_no_expiry', 0, () => {
      const now = new Date('2026-09-20T00:00:00Z');
      expect(parseCookieExpiry('Thu, 31 Dec 2099 23:59:59 GMT', true, now)).assertNull();
    });
    it('parseCookieExpiry_empty_and_sentinel_values', 0, () => {
      const now = new Date('2026-09-20T00:00:00Z');
      // 未设 Expires 的 Cookie 在 Chromium 侧为 -1（ArkWeb 以字符串透出），0 视为无效
      expect(parseCookieExpiry('', false, now)).assertNull();
      expect(parseCookieExpiry('-1', false, now)).assertNull();
      expect(parseCookieExpiry('0', false, now)).assertNull();
      expect(parseCookieExpiry('not-a-date', false, now)).assertNull();
    });
    it('parseCookieExpiry_past_is_null', 0, () => {
      const now = new Date('2026-09-20T00:00:00Z');
      expect(parseCookieExpiry('Thu, 01 Jan 2026 00:00:00 GMT', false, now)).assertNull();
    });
    it('parseCookieExpiry_rfc7601_and_epoch_use_same_instant', 0, () => {
      const now = new Date('2026-09-20T00:00:00Z');
      // 文档口径：expiresDate 为 'Wdy, DD Mon YYYY HH:MM:SS GMT'；数字串按秒/毫秒兜底
      const fromText = parseCookieExpiry('Thu, 31 Dec 2099 23:59:59 GMT', false, now);
      const fromSeconds = parseCookieExpiry('4102444799', false, now);
      const fromMillis = parseCookieExpiry('4102444799000', false, now);
      const expected = localRfc3339(new Date('Thu, 31 Dec 2099 23:59:59 GMT'));
      expect(fromText).assertEqual(expected);
      expect(fromSeconds).assertEqual(expected);
      expect(fromMillis).assertEqual(expected);
    });
    it('formatLocalDateTime_precision_to_minute', 0, () => {
      expect(formatLocalDateTime(new Date(2026, 8, 20, 7, 5, 0))).assertEqual('2026-09-20 07:05');
    });
  });
}
