import { describe, it, expect } from '@ohos/hypium';
import {
  formatTokens,
  formatYuan,
  parseSessionCookie,
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
  });
}
