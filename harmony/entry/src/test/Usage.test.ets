import { describe, it, expect } from '@ohos/hypium';
import { aggregateUsage, dateRangeToUnix, DailyUsageRaw, ModelUsageRaw } from '../main/ets/model/Api';
import { deriveDailyRequests, summarizeLocal, DailySnapshot } from '../main/ets/model/Store';
import {
  buildStatsCsv, buildStatsJson, csvEscape, ExportData, ExportDaily, ExportModel, ExportRange, ExportSummary,
} from '../main/ets/common/Export';

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
    it('avgDividesBySnapshotDaysNotUsageDays', 0, () => {
      // 3 天快照仅 1 天消费 3 元：avg = 3/3 = 1.0（有快照天数口径，对齐桌面版）
      // 若误用"有用量天数"口径会得 3.0，本用例区分两种分母
      const snaps: DailySnapshot[] = [
        { date: '2026-08-01', yuan: 3, tokens: 100, remaining: 90, requestCount: 1 },
        { date: '2026-08-02', yuan: 0, tokens: 0, remaining: 89, requestCount: 1 },
        { date: '2026-08-03', yuan: 0, tokens: 0, remaining: 89, requestCount: 1 },
      ];
      const s = summarizeLocal(snaps);
      expect(s.avgYuan).assertClose(1.0, 0.000001);
      expect(s.daysWithUsage).assertEqual(1);
      expect(s.peakYuan).assertClose(3.0, 0.000001);
      expect(s.peakDate).assertEqual('2026-08-01');
    });
  });
  describe('dateRangeToUnix', () => {
    it('localTimezoneBoundaries', 0, () => {
      const r = dateRangeToUnix('2026-08-01', '2026-08-02');
      expect(r.end - r.start).assertEqual(2 * 86400 - 1); // 起 00:00:00 止 23:59:59
    });
  });
  describe('buildStatsCsv', () => {
    it('containsThreeSectionHeaders', 0, () => {
      const csv = buildStatsCsv(sampleExportData());
      const bom = String.fromCharCode(0xFEFF);
      // BOM 前缀 + 三节标题（节间空行分隔），表头与桌面版逐字一致
      expect(csv.startsWith(bom + '[区间汇总]')).assertTrue();
      expect(csv.includes('\r\n指标,值\r\n')).assertTrue();
      expect(csv.includes('\r\n\r\n[每日趋势]\r\ndate,yuan,tokens,input_tokens,output_tokens,new_requests,remaining\r\n')).assertTrue();
      expect(csv.includes('\r\n\r\n[模型分布]\r\nmodel,yuan,tokens,request_count,percent,percent_basis\r\n')).assertTrue();
      // 汇总节键值行与数据来源标注
      expect(csv.includes('累计费用(元),4.5\r\n')).assertTrue();
      expect(csv.includes('区间总请求数,14\r\n')).assertTrue();
      expect(csv.includes('数据来源,official\r\n')).assertTrue();
      // 每日趋势行含本地合并字段；缺值留空
      expect(csv.includes('2026-08-01,3,300,200,100,10,90\r\n')).assertTrue();
      expect(csv.includes('2026-08-02,1.5,100,60,40,,\r\n')).assertTrue();
      // 模型行尾列 percent_basis
      expect(csv.includes('m-a,3,300,10,75,quota\r\n')).assertTrue();
      // 行尾全为 CRLF（本夹具字段内无换行，剔除 CRLF 后不应残留 LF）
      expect(csv.substring(1).replace(/\r\n/g, '|').includes('\n')).assertFalse();
    });
    it('escapesCommaQuoteNewline', 0, () => {
      // csvEscape 单元规则：null 空串、含 , " 换行时双引号包裹 + 引号加倍
      expect(csvEscape(null)).assertEqual('');
      expect(csvEscape('plain')).assertEqual('plain');
      expect(csvEscape(12.5)).assertEqual('12.5');
      expect(csvEscape('a,b')).assertEqual('"a,b"');
      expect(csvEscape('a"b')).assertEqual('"a""b"');
      expect(csvEscape('a\nb')).assertEqual('"a\nb"');
      expect(csvEscape('a\r\nb')).assertEqual('"a\r\nb"');
      // 整表应用转义：模型名含逗号/引号/换行
      const data = sampleExportData();
      const weird: ExportModel[] = [{ model: 'we,ird"na\nme', yuan: 1, tokens: 2, request_count: 3, percent: 100 }];
      data.models = weird;
      const csv = buildStatsCsv(data);
      expect(csv.includes('"we,ird""na\nme",1,2,3,100,quota')).assertTrue();
    });
    it('emptyModelsOnlyHeader', 0, () => {
      const data = sampleExportData();
      data.models = [];
      const csv = buildStatsCsv(data);
      // 空 models：模型节仅表头，且为文件末尾（无尾随换行）
      expect(csv.endsWith('[模型分布]\r\nmodel,yuan,tokens,request_count,percent,percent_basis')).assertTrue();
    });
    it('localEstimateSkipsRequestCountRow', 0, () => {
      const data = sampleExportData();
      data.source = 'local_estimate';
      data.percent_basis = null;
      const s = sampleExportSummary();
      s.request_count = null;
      data.summary = s;
      data.models = [];
      const csv = buildStatsCsv(data);
      // 降级模式请求数不可派生：整行不输出
      expect(csv.includes('区间总请求数')).assertFalse();
      expect(csv.includes('数据来源,local_estimate\r\n')).assertTrue();
    });
  });
  describe('buildStatsJson', () => {
    it('keepsExportContractFields', 0, () => {
      const json = buildStatsJson(sampleExportData());
      // 导出契约字段（snake_case）与两空格缩进
      expect(json.startsWith('{\n  "app": "amax-dashboard"')).assertTrue();
      expect(json.includes('"start_date": "2026-08-01"')).assertTrue();
      expect(json.includes('"source": "official"')).assertTrue();
      expect(json.includes('"percent_basis": "quota"')).assertTrue();
      expect(json.includes('"new_requests": 10')).assertTrue();
    });
  });
}

// 导出夹具：官方模式汇总（request_count 可派生）
function sampleExportSummary(): ExportSummary {
  return {
    total_yuan: 4.5, avg_yuan: 2.25, peak_yuan: 3, peak_date: '2026-08-01',
    total_tokens: 400, request_count: 14, days_with_usage: 2,
  };
}

// 导出夹具：官方模式两日明细 + 两模型（第二日本地字段缺值留空）
function sampleExportData(): ExportData {
  const daily: ExportDaily[] = [
    { date: '2026-08-01', yuan: 3, tokens: 300, input_tokens: 200, output_tokens: 100, new_requests: 10, remaining: 90 },
    { date: '2026-08-02', yuan: 1.5, tokens: 100, input_tokens: 60, output_tokens: 40, new_requests: null, remaining: null },
  ];
  const models: ExportModel[] = [
    { model: 'm-a', yuan: 3, tokens: 300, request_count: 10, percent: 75 },
    { model: 'm-b', yuan: 1.5, tokens: 100, request_count: 4, percent: 25 },
  ];
  const range: ExportRange = { start_date: '2026-08-01', end_date: '2026-08-02' };
  return {
    app: 'amax-dashboard',
    exported_at: '2026-08-03T00:00:00.000Z',
    range: range,
    source: 'official',
    percent_basis: 'quota',
    summary: sampleExportSummary(),
    daily: daily,
    models: models,
  };
}
