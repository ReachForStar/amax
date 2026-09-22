import { describe, it, expect } from '@ohos/hypium';
import {
  AlertSettings, Baseline, Current, DayState, RuleCount, UsageDay,
  buildBaseline, baselineStartDay, dayStateFromCounts, defaultSettings, detect, paramRanges,
  resolveRulesEnabled, ruleTitle, sanitizeSettings,
} from '../main/ets/common/Alert';
import {
  classifyMeow, meowAccepted, meowUrl, notificationId, summarize, Receipt,
} from '../main/ets/common/Deliver';

function settings(overrides: Partial<AlertSettings> | null): AlertSettings {
  const base = defaultSettings();
  if (overrides === null) {
    return base;
  }
  const merged: AlertSettings = {
    enabled: overrides.enabled === undefined ? base.enabled : overrides.enabled,
    quotaPercent: overrides.quotaPercent === undefined ? base.quotaPercent : overrides.quotaPercent,
    runoutDays: overrides.runoutDays === undefined ? base.runoutDays : overrides.runoutDays,
    spikeMultiplier: overrides.spikeMultiplier === undefined ? base.spikeMultiplier
      : overrides.spikeMultiplier,
    maxFiresPerDay: overrides.maxFiresPerDay === undefined ? base.maxFiresPerDay
      : overrides.maxFiresPerDay,
  };
  return merged;
}

function current(percent: number, remaining: number, total: number, todayYuan: number): Current {
  return { percent: percent, remaining: remaining, total: total, todayYuan: todayYuan };
}

function baseline(avgYuan: number | null, medianYuan: number | null, sampleDays: number): Baseline {
  return { avgYuan: avgYuan, medianYuan: medianYuan, sampleDays: sampleDays };
}

function freshDayState(): DayState {
  return dayStateFromCounts([]);
}

function usage(date: string, yuan: number): UsageDay {
  return { date: date, yuan: yuan };
}

function receipt(channel: 'notification' | 'meow', outcome: 'sent' | 'failed' | 'retryable'): Receipt {
  return { channel: channel, outcome: outcome, error: outcome === 'sent' ? null : 'x' };
}

export default function alertTest() {
  describe('alertEngine', () => {
    it('sanitizeSettings_clamps_every_field', 0, () => {
      const saved = sanitizeSettings(settings({
        quotaPercent: 500, runoutDays: 1000, spikeMultiplier: 0.2, maxFiresPerDay: 99,
      }));
      expect(saved.quotaPercent).assertEqual(99);
      expect(saved.runoutDays).assertEqual(90);
      expect(saved.spikeMultiplier).assertEqual(1.5);
      expect(saved.maxFiresPerDay).assertEqual(10);
    });
    it('sanitizeSettings_falls_back_on_non_finite_and_rounds_ints', 0, () => {
      const saved = sanitizeSettings(settings({
        quotaPercent: Number.NaN, runoutDays: 4.6, spikeMultiplier: Number.POSITIVE_INFINITY,
        maxFiresPerDay: -3,
      }));
      expect(saved.quotaPercent).assertEqual(10);
      expect(saved.spikeMultiplier).assertEqual(3);
      expect(saved.runoutDays).assertEqual(5);
      expect(saved.maxFiresPerDay).assertEqual(0);
    });
    it('paramRanges_keys_match_settings', 0, () => {
      const keys = paramRanges().map((range): string => range.key);
      expect(keys.join(',')).assertEqual('quota_percent,runout_days,spike_multiplier,max_fires_per_day');
    });
    it('buildBaseline_drops_today_zero_days_and_out_of_window', 0, () => {
      const rows: UsageDay[] = [
        usage('2026-09-14', 99),   // 窗口外
        usage('2026-09-15', 10),   // 窗口首日，含
        usage('2026-09-16', 0),    // 空缺日：不能当成 ¥0 拉低日均
        usage('2026-09-21', 20),
        usage('2026-09-22', 88),   // 当日不参与基准
      ];
      const result = buildBaseline(rows, '2026-09-22', 7);
      expect(result.sampleDays).assertEqual(2);
      expect(result.avgYuan === null ? -1 : result.avgYuan).assertEqual(15);
      expect(result.medianYuan === null ? -1 : result.medianYuan).assertEqual(15);
    });
    it('buildBaseline_median_uses_even_average', 0, () => {
      const rows: UsageDay[] = [
        usage('2026-09-19', 4), usage('2026-09-20', 1), usage('2026-09-21', 3),
      ];
      const even = buildBaseline(rows, '2026-09-22', 7);
      expect(even.medianYuan === null ? -1 : even.medianYuan).assertEqual(3);
      const odd = buildBaseline(rows.concat([usage('2026-09-18', 2)]), '2026-09-22', 7);
      expect(odd.medianYuan === null ? -1 : odd.medianYuan).assertEqual(2.5);
    });
    it('buildBaseline_empty_window_is_no_baseline', 0, () => {
      const result = buildBaseline([usage('2026-09-22', 50)], '2026-09-22', 7);
      expect(result.sampleDays).assertEqual(0);
      expect(result.avgYuan).assertNull();
      expect(result.medianYuan).assertNull();
    });
    it('buildBaseline_rejects_unparsable_day', 0, () => {
      let threw = false;
      try {
        buildBaseline([usage('昨天', 5)], '2026-09-22', 7);
      } catch (error) {
        threw = true;
        expect((error as Error).message.indexOf('无法解析') >= 0).assertTrue();
      }
      expect(threw).assertTrue();
    });
    it('baselineStartDay_crosses_month_boundary', 0, () => {
      expect(baselineStartDay('2026-09-01', 7)).assertEqual('2026-08-25');
      expect(baselineStartDay('2026-01-03', 7)).assertEqual('2025-12-27');
    });
    it('detect_stays_silent_without_baseline', 0, () => {
      // 新装或一直没运行：percent 再低也不报，否则第一次刷新就会被轰炸
      const findings = detect(current(1, 1, 100, 0), baseline(null, null, 0), settings(null),
        ['quota_low', 'runout_soon', 'spike'], freshDayState());
      expect(findings.length).assertEqual(0);
    });
    it('detect_orders_rules_and_respects_daily_budget', 0, () => {
      const findings = detect(current(5, 20, 100, 90), baseline(10, 20, 5), settings(null),
        ['quota_low', 'runout_soon', 'spike'], freshDayState());
      expect(findings.length).assertEqual(2);   // 默认每日 2 次
      expect(findings[0].rule).assertEqual('quota_low');
      expect(findings[1].rule).assertEqual('runout_soon');
    });
    it('detect_skips_rules_already_fired_today', 0, () => {
      // 上限放到 3 是为了把「同规则当日去重」和「每日预算截断」分开验：预算不够时会先截断，看不出差异
      const counts: RuleCount[] = [{ key: 'quota_low', count: 1 }];
      const state = dayStateFromCounts(counts);
      const findings = detect(current(5, 20, 100, 90), baseline(10, 20, 5),
        settings({ maxFiresPerDay: 3 }), ['quota_low', 'runout_soon', 'spike'], state);
      expect(findings.length).assertEqual(2);
      expect(findings[0].rule).assertEqual('runout_soon');
      expect(findings[1].rule).assertEqual('spike');
      expect(state.total).assertEqual(1);
    });
    it('detect_honours_switches_and_cap', 0, () => {
      const off = detect(current(5, 20, 100, 90), baseline(10, 20, 5), settings({ enabled: false }),
        ['quota_low', 'runout_soon', 'spike'], freshDayState());
      expect(off.length).assertEqual(0);
      const muted = detect(current(5, 20, 100, 90), baseline(10, 20, 5), settings({ maxFiresPerDay: 0 }),
        ['quota_low'], freshDayState());
      expect(muted.length).assertEqual(0);
      // 关掉 quota_low 后同一条数据仍出两条，且顺序仍是规则声明序
      const ruleOff = detect(current(5, 20, 100, 90), baseline(10, 20, 5), settings(null),
        ['runout_soon', 'spike'], freshDayState());
      expect(ruleOff.length).assertEqual(2);
      expect(ruleOff[0].rule).assertEqual('runout_soon');
      expect(ruleOff[1].rule).assertEqual('spike');
    });
    it('detect_needs_samples_for_spike_and_avg_for_runout', 0, () => {
      // 样本不足 3 天：突增不判；日均缺失：燃尽不判
      const twoSamples = detect(current(90, 900, 1000, 90), baseline(10, 20, 2), settings({ quotaPercent: 1 }),
        ['spike'], freshDayState());
      expect(twoSamples.length).assertEqual(0);
      const noAvg = detect(current(90, 20, 1000, 0), baseline(null, null, 5), settings({ quotaPercent: 1 }),
        ['runout_soon'], freshDayState());
      expect(noAvg.length).assertEqual(0);
    });
    it('detect_messages_match_desktop_wording', 0, () => {
      const findings = detect(current(37.9, 634.56, 1674, 30), baseline(100, 10, 5), settings({ quotaPercent: 40 }),
        ['quota_low', 'spike'], freshDayState());
      expect(findings[0].message).assertEqual('剩余 ¥634.56 / ¥1674.00 (37.9%), 请及时充值');
      expect(findings[1].message).assertEqual('今日已花 ¥30.00，是近5日中位数 ¥10.00 的 3.0 倍');
      expect(findings[0].title).assertEqual('⚠️ AMAX 额度不足');
    });
    it('dayStateFromCounts_ignores_unknown_rules', 0, () => {
      const state = dayStateFromCounts([
        { key: 'quota_low', count: 1 }, { key: 'retired_rule', count: 7 },
      ]);
      expect(state.total).assertEqual(1);
      expect(state.perRule[0]).assertEqual(1);
      expect(state.perRule[1]).assertEqual(0);
    });
    it('resolveRulesEnabled_normalizes_and_rejects_unknown', 0, () => {
      expect(resolveRulesEnabled(['spike', 'quota_low'], null).join(','))
        .assertEqual('spike,quota_low');
      expect(resolveRulesEnabled(['quota_low'], ['spike', 'quota_low']).join(','))
        .assertEqual('quota_low,spike');
      let threw = false;
      try {
        resolveRulesEnabled([], ['nope']);
      } catch (error) {
        threw = true;
        expect((error as Error).message).assertEqual('未知告警规则: nope');
      }
      expect(threw).assertTrue();
    });
    it('ruleTitles_cover_every_rule_key', 0, () => {
      expect(ruleTitle('runout_soon')).assertEqual('⏳ AMAX 额度将耗尽');
      expect(ruleTitle('spike')).assertEqual('📈 AMAX 今日花费异常');
    });
  });

  describe('alertDeliver', () => {
    it('meowUrl_percent_encodes_both_segments', 0, () => {
      const url = meowUrl('张三/pika', '⚠️ AMAX 额度不足');
      expect(url.indexOf('https://api.chuckfang.com/')).assertEqual(0);
      expect(url.indexOf('%2F') >= 0).assertTrue();   // '/' 不编码就会打到别的昵称上
      expect(url.indexOf(' ') === -1).assertTrue();
      expect(url.endsWith('?msgType=markdown')).assertTrue();
      // https: + 空段 + host + 昵称段 + 标题段：昵称里的 '/' 若没编码就会多出一个空段
      expect(url.split('/').length).assertEqual(5);
    });
    it('classifyMeow_splits_retryable_and_permanent', 0, () => {
      expect(classifyMeow(429, 'slow down')).assertEqual('retryable');
      expect(classifyMeow(502, 'bad gateway')).assertEqual('retryable');
      expect(classifyMeow(404, 'not found')).assertEqual('failed');
      expect(classifyMeow(200, '{"status":200,"msg":"ok"}')).assertEqual('sent');
      expect(classifyMeow(200, '{"data":true}')).assertEqual('sent');
      expect(classifyMeow(200, '{"status":500,"msg":"no such user"}')).assertEqual('failed');
    });
    it('classifyMeow_treats_non_json_2xx_as_sent', 0, () => {
      expect(meowAccepted('not json')).assertTrue();
      expect(meowAccepted('123')).assertFalse();
      expect(meowAccepted('[]')).assertFalse();
      expect(classifyMeow(200, 'not json')).assertEqual('sent');
    });
    it('summarize_prefers_sent_then_retryable', 0, () => {
      expect(summarize([receipt('meow', 'failed'), receipt('notification', 'sent')]))
        .assertEqual('sent');
      expect(summarize([receipt('meow', 'retryable'), receipt('notification', 'failed')]))
        .assertEqual('retryable');
      expect(summarize([receipt('meow', 'failed')])).assertEqual('failed');
      // 一条渠道都没投成且没有可重试项 → 消耗额度，避免每轮刷新撞同一堵墙
      expect(summarize([])).assertEqual('failed');
    });
    it('notificationId_keeps_rules_apart', 0, () => {
      expect(notificationId('quota_low')).assertEqual(1);
      expect(notificationId('spike')).assertEqual(3);
      expect(notificationId('test')).assertEqual(4);   // 测试投递不占三条规则的位
    });
  });
}
