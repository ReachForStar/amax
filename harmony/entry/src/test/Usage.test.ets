import { describe, it, expect } from '@ohos/hypium';
import { aggregateUsage, dateRangeToUnix, DailyUsageRaw, ModelUsageRaw } from '../main/ets/model/Api';
import { deriveDailyRequests, summarizeLocal, DailySnapshot } from '../main/ets/model/Store';

// rawDaily 元素：{ date: 本地 0 点 Unix 秒, model, quota, total_tokens, input_tokens, output_tokens }
function ts(dateStr: string): number {
  const parts = dateStr.split('-');
  return Math.floor(new Date(Number(parts[0]), Number(parts[1]) - 1, Number(parts[2]), 0, 0, 0, 0).getTime() / 1000);
}

export default function usageTest() {
  describe('aggregateUsage', () => {
    it('fillsMissingDaysAndMergesModels', 0, () => {
      const d1 = ts('2026-08-01');
      const d3 = ts('2026-08-03');
      const rawDaily: DailyUsageRaw[] = [
        { date: d1, quota: 1_000_000, total_tokens: 200, input_tokens: 150, output_tokens: 50 },
        { date: d1, quota: 500_000, total_tokens: 100, input_tokens: 60, output_tokens: 40 },
        { date: d3, quota: 500_000, total_tokens: 100, input_tokens: 80, output_tokens: 20 },
      ];
      const rawModels: ModelUsageRaw[] = [
        { model: 'm-a', quota: 1_500_000, total_tokens: 300, request_count: 10 },
        { model: 'm-b', quota: 500_000, total_tokens: 100, request_count: 4 },
      ];
      const stats = aggregateUsage(rawDaily, rawModels, '2026-08-01', '2026-08-03');
      expect(stats.daily.length).assertEqual(3);
      expect(stats.daily[0].yuan).assertClose(3.0, 0.000001);   // 跨模型求和
      expect(stats.daily[0].tokens).assertEqual(300);
      expect(stats.daily[1].yuan).assertEqual(0);               // 补零
      expect(stats.summary.totalYuan).assertClose(4.0, 0.000001);
      expect(stats.summary.daysWithUsage).assertEqual(2);
      expect(stats.summary.avgYuan).assertClose(2.0, 0.000001);
      expect(stats.summary.peakYuan).assertClose(3.0, 0.000001);
      expect(stats.summary.peakDate).assertEqual('2026-08-01');
      expect(stats.summary.requestCount).assertEqual(14);
      expect(stats.percentBasis).assertEqual('quota');
      expect(stats.models[0].model).assertEqual('m-a');
      expect(stats.models[0].percent).assertClose(75.0, 0.01);
    });
    it('switchesToTokensBasisWhenQuotaZero', 0, () => {
      const rawDaily: DailyUsageRaw[] = [{ date: ts('2026-08-01'), quota: 0, total_tokens: 400, input_tokens: 0, output_tokens: 0 }];
      const rawModels: ModelUsageRaw[] = [
        { model: 'm-a', quota: 0, total_tokens: 300, request_count: 3 },
        { model: 'm-b', quota: 0, total_tokens: 100, request_count: 1 },
      ];
      const stats = aggregateUsage(rawDaily, rawModels, '2026-08-01', '2026-08-01');
      expect(stats.percentBasis).assertEqual('tokens');
      expect(stats.models[0].percent).assertClose(75.0, 0.01);
    });
    it('emptyRangeAllZero', 0, () => {
      const stats = aggregateUsage([], [], '2026-08-01', '2026-08-02');
      expect(stats.daily.length).assertEqual(2);
      expect(stats.summary.daysWithUsage).assertEqual(0);
      expect(stats.summary.peakDate).assertEqual('2026-08-02');
    });
  });
  describe('deriveDailyRequests', () => {
    it('adjacentGapAndNull', 0, () => {
      const snaps: DailySnapshot[] = [
        { date: '2026-08-01', yuan: 0, tokens: 0, remaining: 0, requestCount: 100 },
        { date: '2026-08-02', yuan: 0, tokens: 0, remaining: 0, requestCount: 130 },
        { date: '2026-08-04', yuan: 0, tokens: 0, remaining: 0, requestCount: 200 },
        { date: '2026-08-05', yuan: 0, tokens: 0, remaining: 0, requestCount: null },
      ];
      const diff = deriveDailyRequests(snaps);
      expect(diff[0]).assertNull();
      expect(diff[1]).assertEqual(30);
      expect(diff[2]).assertNull();   // 缺档
      expect(diff[3]).assertNull();   // 前值为 null
    });
  });
  describe('summarizeLocal', () => {
    it('totalsAndPeak', 0, () => {
      const snaps: DailySnapshot[] = [
        { date: '2026-08-01', yuan: 2.5, tokens: 100, remaining: 90, requestCount: 1 },
        { date: '2026-08-02', yuan: 4.5, tokens: 300, remaining: 80, requestCount: 2 },
      ];
      const s = summarizeLocal(snaps);
      expect(s.totalYuan).assertClose(7.0, 0.000001);
      expect(s.avgYuan).assertClose(3.5, 0.000001);
      expect(s.peakYuan).assertClose(4.5, 0.000001);
      expect(s.peakDate).assertEqual('2026-08-02');
      expect(s.totalTokens).assertEqual(400);
    });
  });
  describe('dateRangeToUnix', () => {
    it('localTimezoneBoundaries', 0, () => {
      const r = dateRangeToUnix('2026-08-01', '2026-08-02');
      expect(r.end - r.start).assertEqual(2 * 86400 - 1); // 起 00:00:00 止 23:59:59
    });
  });
}
