//! 配额/费用告警引擎 — 规则评估 + 按日去重 + 每日上限
//!
//! 三条规则各自回答一个问题，共用同一份「近 7 日有用量日子」的基准和同一套去重/上限逻辑：
//! - `quota_low`：剩余百分比低于水位（存量状态）
//! - `runout_soon`：剩余 ÷ 近 7 日日均花费 ≤ 天数（消耗趋势）
//! - `spike`：当日花费 ≥ 近 7 日中位数 × 倍数（当日异常，用中位数以免被极端日自己抬高门槛）
//!
//! 去重是「同规则同日一次」且跨进程持久（`alert_state` 表，见 `db.rs`），不随应用重启复位；
//! 每日上限是所有规则的总投递次数，0 表示当日全部静默。
//!
//! 宁可漏报不误报：基准窗口内完全没有用量记录时（新装、或一直没运行）三条规则一律不判，
//! 否则新装用户第一次刷新就会因为 `percent<10` 或「今日 ≥ 3×0」被轰炸。
//!
//! 与手机端 `common/Notify.ets` 的差异（手机端只有 percent<10 一条规则，防重发标志是模块级
//! 进程内状态、重启即复位）是有意为之，手机端对齐在后续阶段处理，不要在这里顺手统一。

use crate::db::DailySnapshot;
use crate::error::AppError;
use chrono::{DateTime, Days, NaiveDate};
use serde::Serialize;

/// 基准窗口天数：燃尽日均与突增中位数都只看最近这些天（不含当日）
pub const BASELINE_WINDOW_DAYS: i64 = 7;
/// 突增判定所需的最少有用量天数，样本不足直接不判
pub const MIN_SPIKE_SAMPLES: usize = 3;
/// 每日告警总数上限的可调上界
pub const MAX_DAILY_FIRES_LIMIT: i64 = 10;

/// 用户可调的四个参数 + 总开关，持久化在 `config` 表明文字段。
/// 前端按 camelCase 传字段（Tauri 只自动转换 command 参数名，不转换嵌套结构体字段），
/// 所以这里显式声明 `rename_all`。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertSettings {
    pub enabled: bool,
    /// 剩余额度水位（百分比），低于该值告警
    pub quota_percent: f64,
    /// 按日均花费还能撑多少天以内算告警
    pub runout_days: i64,
    /// 当日花费达到中位数的多少倍算异常
    pub spike_multiplier: f64,
    /// 所有规则合计的当日投递次数上限
    pub max_fires_per_day: i64,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            quota_percent: 10.0,
            runout_days: 5,
            spike_multiplier: 3.0,
            max_fires_per_day: 2,
        }
    }
}

impl AlertSettings {
    /// 逐字段夹到合法区间，非有限值取默认值。
    ///
    /// 刻意不返回 `Option`：任何输入都产出可用参数，调用方不必为「参数不合法」再写一套语义。
    pub fn sanitized(self) -> Self {
        Self {
            enabled: self.enabled,
            quota_percent: clamp_or(self.quota_percent, 1.0, 99.0, 10.0),
            runout_days: self.runout_days.clamp(1, 90),
            spike_multiplier: clamp_or(self.spike_multiplier, 1.5, 100.0, 3.0),
            max_fires_per_day: self.max_fires_per_day.clamp(0, MAX_DAILY_FIRES_LIMIT),
        }
    }
}

fn clamp_or(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// 可调参数的合法区间（前端据此做输入约束与提示，避免两处各写一份数字）
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ParamRange {
    pub key: &'static str,
    pub min: f64,
    pub max: f64,
}

/// 四个可调参数的区间，`key` 与 `AlertSettings` 字段名一致
pub fn param_ranges() -> Vec<ParamRange> {
    vec![
        ParamRange {
            key: "quota_percent",
            min: 1.0,
            max: 99.0,
        },
        ParamRange {
            key: "runout_days",
            min: 1.0,
            max: 90.0,
        },
        ParamRange {
            key: "spike_multiplier",
            min: 1.5,
            max: 100.0,
        },
        ParamRange {
            key: "max_fires_per_day",
            min: 0.0,
            max: MAX_DAILY_FIRES_LIMIT as f64,
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    QuotaLow,
    RunoutSoon,
    Spike,
}

impl Rule {
    /// 评估顺序即投递顺序：额度是存量、燃尽是趋势、突增是当日异常，
    /// 当日剩余次数不够时优先留下更靠前的一条。
    const ALL: [Self; 3] = [Self::QuotaLow, Self::RunoutSoon, Self::Spike];

    /// 去重主键（`alert_state.rule_key`）、配置键后缀与事件负载里的 `rule` 字段三处同源
    pub fn key(self) -> &'static str {
        match self {
            Self::QuotaLow => "quota_low",
            Self::RunoutSoon => "runout_soon",
            Self::Spike => "spike",
        }
    }

    /// 在 `ALL` / `DayState.per_rule` 里的下标，两者顺序必须一致
    fn index(self) -> usize {
        match self {
            Self::QuotaLow => 0,
            Self::RunoutSoon => 1,
            Self::Spike => 2,
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rule| rule.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::QuotaLow => "⚠️ AMAX 额度不足",
            Self::RunoutSoon => "⏳ AMAX 额度将耗尽",
            Self::Spike => "📈 AMAX 今日花费异常",
        }
    }

    /// 命中正文；`None` 表示这条规则此刻给不出可信的描述（例如日均为 0 无法算燃尽天数）
    fn body(self, current: &Current, baseline: &Baseline) -> Option<String> {
        match self {
            Self::QuotaLow => Some(format!(
                "剩余 ¥{:.2} / ¥{:.2} ({:.1}%), 请及时充值",
                current.remaining, current.total, current.percent
            )),
            Self::RunoutSoon => {
                let avg = baseline.avg_yuan.filter(|avg| *avg > 0.0)?;
                let days = current.remaining / avg;
                if !days.is_finite() {
                    return None;
                }
                Some(format!(
                    "剩余 ¥{:.2}，按近{}日日均 ¥{:.2} 约 {days:.1} 天耗尽",
                    current.remaining, BASELINE_WINDOW_DAYS, avg
                ))
            }
            Self::Spike => {
                if baseline.sample_days < MIN_SPIKE_SAMPLES {
                    return None;
                }
                let median = baseline.median_yuan.filter(|median| *median > 0.0)?;
                let ratio = current.today_yuan / median;
                if !ratio.is_finite() {
                    return None;
                }
                Some(format!(
                    "今日已花 ¥{:.2}，是近{}日中位数 ¥{:.2} 的 {ratio:.1} 倍",
                    current.today_yuan, baseline.sample_days, median
                ))
            }
        }
    }
}

/// 全部规则 key（设置页与校验共用）
pub fn rule_keys() -> Vec<&'static str> {
    Rule::ALL.iter().map(|rule| rule.key()).collect()
}

/// 看板刷新那一刻的账户状态
#[derive(Debug, Clone, Copy)]
pub struct Current {
    /// **剩余**百分比（`api.rs::assemble_dashboard` 取 quota/total×100）
    pub percent: f64,
    pub remaining: f64,
    pub total: f64,
    pub today_yuan: f64,
}

/// 近期用量基准（不含当日）
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Baseline {
    pub avg_yuan: Option<f64>,
    pub median_yuan: Option<f64>,
    /// 有用量的天数；0 表示窗口内完全无记录
    pub sample_days: usize,
}

/// 一次命中
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub title: String,
    pub message: String,
}

/// 当日已投递情况：`total` 是所有规则合计，`per_rule` 供逐规则去重
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DayState {
    pub total: i64,
    per_rule: [i64; 3],
}

impl DayState {
    /// 由 `alert_state` 行构造；未知 rule_key 忽略（规则增删后的历史行不该影响新判断）
    pub fn from_counts(counts: &[(String, i64)]) -> Self {
        let mut per_rule = [0; 3];
        let mut total = 0;
        for (key, count) in counts {
            if let Some(rule) = Rule::from_key(key) {
                per_rule[rule.index()] = *count;
                total += *count;
            }
        }
        Self { total, per_rule }
    }

    fn count_of(self, rule: Rule) -> i64 {
        self.per_rule[rule.index()]
    }
}

/// 单条规则是否命中（不含去重与上限）
fn breaches(rule: Rule, current: &Current, baseline: &Baseline, settings: &AlertSettings) -> bool {
    if baseline.sample_days == 0 {
        return false;
    }
    match rule {
        Rule::QuotaLow => current.percent < settings.quota_percent,
        Rule::RunoutSoon => match baseline.avg_yuan {
            Some(avg) if avg > 0.0 => current.remaining / avg <= settings.runout_days as f64,
            _ => false,
        },
        Rule::Spike => match baseline.median_yuan {
            Some(median) if median > 0.0 && baseline.sample_days >= MIN_SPIKE_SAMPLES => {
                current.today_yuan >= settings.spike_multiplier * median
            }
            _ => false,
        },
    }
}

/// 评估当日应投递的告警：命中 + 已启用 + 今日未投，按 [`Rule::ALL`] 顺序取到当日剩余额度为止。
pub fn detect(
    current: &Current,
    baseline: &Baseline,
    settings: &AlertSettings,
    rules_enabled: &[String],
    state: &DayState,
) -> Vec<Finding> {
    if !settings.enabled || state.total >= settings.max_fires_per_day {
        return Vec::new();
    }
    let budget = settings.max_fires_per_day - state.total;
    Rule::ALL
        .into_iter()
        .filter(|rule| rules_enabled.iter().any(|key| *key == rule.key()))
        .filter(|rule| state.count_of(*rule) == 0)
        .filter(|rule| breaches(*rule, current, baseline, settings))
        .filter_map(|rule| rule.body(current, baseline).map(|message| (rule, message)))
        .take(budget as usize)
        .map(|(rule, message)| Finding {
            rule: rule.key(),
            title: rule.title().to_string(),
            message,
        })
        .collect()
}

/// 日末快照序列 → 用量基准：排除当日与窗口外的日期，只取**有用量**（yuan > 0）的日子。
///
/// 空缺日（应用没运行）整行排除：当成 ¥0 会同时拉低日均（把正常一天判成突增）和抬高日均
/// （把快耗尽算成还能撑），两头都是误报。
pub fn build_baseline(
    snapshots: &[DailySnapshot],
    day: &str,
    window_days: i64,
) -> Result<Baseline, AppError> {
    let today = day_start_of(day)?;
    let start = today
        .checked_sub_days(Days::new(window_days.max(1) as u64))
        .ok_or_else(|| AppError::input("告警基准窗口天数超出可表示范围"))?;

    // f64 不是 Ord，没法用 BTreeSet<(日期, 金额)] 折叠；n ≤ 窗口天数，线性查重足够
    let mut usage: Vec<(NaiveDate, f64)> = Vec::new();
    for snapshot in snapshots {
        let date = day_start_of(&snapshot.date)?;
        if snapshot.yuan > 0.0 && date >= start && date < today {
            let row = (date, snapshot.yuan);
            if !usage.contains(&row) {
                usage.push(row);
            }
        }
    }
    usage.sort_by_key(|(date, _)| *date);

    let values: Vec<f64> = usage.iter().map(|(_, yuan)| *yuan).collect();
    let sample_days = values.len();
    if sample_days == 0 {
        return Ok(Baseline::default());
    }
    let sum: f64 = values.iter().sum();
    // values 已按 (date, yuan) 有序而非按金额有序，中位数要单独排序
    let mut sorted = values.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = if sorted.len().is_multiple_of(2) {
        let low_index = sorted.len() / 2 - 1;
        (sorted[low_index] + sorted[low_index + 1]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };
    Ok(Baseline {
        avg_yuan: Some(sum / sample_days as f64),
        median_yuan: Some(median),
        sample_days,
    })
}

/// 把 `YYYY-MM-DD` 或本地 RFC3339 归一为本地日界
fn day_start_of(value: &str) -> Result<NaiveDate, AppError> {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(date);
    }
    DateTime::parse_from_rfc3339(value)
        .map(|instant| instant.date_naive())
        .map_err(|_| AppError::input(format!("告警基准日期无法解析: {value}")))
}

/// 解析「哪些规则启用」的写入值：未传即保持现状，传了必须全在已知规则集合内。
///
/// 单独导出是为了让 command 层可测（不依赖 Tauri 状态）。返回值按 [`Rule::ALL`] 顺序规范化。
pub fn resolve_rules_enabled(
    current: &[String],
    requested: Option<Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let keys = rule_keys();
    let requested = match requested {
        None => return Ok(current.to_vec()),
        Some(requested) => requested,
    };
    let unknown: Vec<&str> = requested
        .iter()
        .map(String::as_str)
        .filter(|key| !keys.contains(key))
        .collect();
    if !unknown.is_empty() {
        return Err(AppError::input(format!(
            "未知告警规则: {}",
            unknown.join(", ")
        )));
    }
    Ok(keys
        .into_iter()
        .filter(|key| requested.iter().any(|requested| requested == *key))
        .map(String::from)
        .collect())
}

/// 读取告警参数；读失败按默认值返回——一次坏值不该挡住看板刷新
pub fn load_settings(db: &crate::db::Db) -> AlertSettings {
    db.get_alert_settings().unwrap_or_else(|error| {
        log::warn!("读取告警设置失败，按默认值处理: {error}");
        AlertSettings::default()
    })
}

/// 读取启用的规则；读失败按全开处理
pub fn load_rules_enabled(db: &crate::db::Db) -> Vec<String> {
    db.get_alert_rules_enabled(&rule_keys())
        .unwrap_or_else(|error| {
            log::warn!("读取告警规则开关失败，按全开处理: {error}");
            rule_keys().into_iter().map(String::from).collect()
        })
}

#[cfg(test)]
mod tests {
    use super::{
        AlertSettings, Baseline, Current, DayState, Finding, MAX_DAILY_FIRES_LIMIT, Rule,
        build_baseline, detect, param_ranges, resolve_rules_enabled, rule_keys,
    };
    use crate::db::DailySnapshot;

    fn settings() -> AlertSettings {
        AlertSettings::default()
    }

    fn current(percent: f64, remaining: f64, today_yuan: f64) -> Current {
        Current {
            percent,
            remaining,
            total: 100.0,
            today_yuan,
        }
    }

    fn baseline(avg: f64, median: f64, sample_days: usize) -> Baseline {
        Baseline {
            avg_yuan: (avg > 0.0).then_some(avg),
            median_yuan: (median > 0.0).then_some(median),
            sample_days,
        }
    }

    fn all_rules() -> Vec<String> {
        rule_keys().into_iter().map(String::from).collect()
    }

    fn rules(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|finding| finding.rule).collect()
    }

    fn detect_current(current: &Current, base: &Baseline) -> Vec<Finding> {
        detect(
            current,
            base,
            &settings(),
            &all_rules(),
            &DayState::default(),
        )
    }

    fn snap(date: &str, yuan: f64) -> DailySnapshot {
        DailySnapshot {
            date: date.to_string(),
            yuan,
            tokens: 0,
            remaining: 0.0,
            request_count: None,
        }
    }

    // ---------- quota_low ----------

    #[test]
    fn quota_low_fires_below_threshold() {
        let findings = detect_current(&current(9.9, 9.9, 0.0), &baseline(1.0, 1.0, 5));
        assert_eq!(rules(&findings), vec!["quota_low"]);
        assert!(
            findings[0].message.contains("(9.9%)"),
            "正文应带剩余百分比: {}",
            findings[0].message
        );
    }

    #[test]
    fn quota_low_at_threshold_does_not_fire() {
        let findings = detect_current(&current(10.0, 10.0, 0.0), &baseline(1.0, 1.0, 5));
        assert!(findings.is_empty(), "恰好等于阈值不算命中");
    }

    #[test]
    fn quota_low_respects_adjusted_threshold() {
        let settings = AlertSettings {
            quota_percent: 30.0,
            ..settings()
        };
        let findings = detect(
            &current(29.0, 29.0, 0.0),
            &baseline(1.0, 1.0, 5),
            &settings,
            &all_rules(),
            &DayState::default(),
        );
        assert_eq!(rules(&findings), vec!["quota_low"]);
    }

    // ---------- runout_soon ----------

    #[test]
    fn runout_soon_fires_within_days_budget() {
        // 剩余 10 / 日均 2 = 5 天，等于阈值即命中；水位 50% 不触发 quota_low
        let findings = detect_current(&current(50.0, 10.0, 0.0), &baseline(2.0, 2.0, 6));
        assert_eq!(rules(&findings), vec!["runout_soon"]);
        assert!(
            findings[0].message.contains("约 5.0 天耗尽"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn runout_soon_skips_beyond_days_budget() {
        let findings = detect_current(&current(50.0, 20.0, 0.0), &baseline(2.0, 2.0, 6));
        assert!(findings.is_empty(), "10 天 > 阈值 5 天不应命中");
    }

    #[test]
    fn runout_soon_skips_when_daily_average_is_zero() {
        let findings = detect_current(&current(50.0, 10.0, 0.0), &baseline(0.0, 0.0, 5));
        assert!(findings.is_empty(), "日均为 0 不可除，不判");
    }

    // ---------- spike ----------

    #[test]
    fn spike_fires_at_multiplier_boundary() {
        // 今日 30 = 3×中位数 10；日均 1 → 剩余 50 可撑 50 天，不触发燃尽
        let findings = detect_current(&current(50.0, 50.0, 30.0), &baseline(1.0, 10.0, 4));
        assert_eq!(rules(&findings), vec!["spike"]);
        assert!(
            findings[0].message.contains("的 3.0 倍"),
            "正文应带倍数: {}",
            findings[0].message
        );
    }

    #[test]
    fn spike_skips_below_multiplier() {
        let findings = detect_current(&current(50.0, 50.0, 29.9), &baseline(1.0, 10.0, 4));
        assert!(findings.is_empty());
    }

    #[test]
    fn spike_skips_when_samples_insufficient() {
        let findings = detect_current(&current(50.0, 50.0, 99.0), &baseline(1.0, 10.0, 2));
        assert!(findings.is_empty(), "样本 <3 天不判突增");
    }

    #[test]
    fn spike_uses_median_not_mean() {
        // 极端日把均值抬到 12，中位数仍是 1.0：按均值判会漏判
        let findings = detect_current(&current(90.0, 90.0, 6.0), &baseline(12.0, 1.0, 4));
        assert_eq!(rules(&findings), vec!["spike"]);
    }

    // ---------- 去重 / 上限 / 开关 ----------

    #[test]
    fn same_rule_fires_only_once_per_day() {
        // 只有 quota_low 命中（日均 0.05 → 剩余 5 元可撑 100 天，不触燃尽），它今日已投
        let state = DayState::from_counts(&[("quota_low".to_string(), 1)]);
        let findings = detect(
            &current(5.0, 5.0, 0.0),
            &baseline(0.05, 0.05, 5),
            &settings(),
            &all_rules(),
            &state,
        );
        assert!(findings.is_empty(), "同规则同日不重复");
    }

    #[test]
    fn another_rule_may_fire_while_budget_remains() {
        // quota_low 今日已投一次（total 1 < 上限 2），燃尽仍可投
        let state = DayState::from_counts(&[("quota_low".to_string(), 1)]);
        let findings = detect(
            &current(5.0, 5.0, 0.0),
            &baseline(2.0, 2.0, 5),
            &settings(),
            &all_rules(),
            &state,
        );
        assert_eq!(rules(&findings), vec!["runout_soon"]);
    }

    #[test]
    fn daily_cap_limits_how_many_rules_fire() {
        let findings = detect_current(&current(5.0, 100.0, 30.0), &baseline(0.05, 10.0, 5));
        assert_eq!(
            rules(&findings),
            vec!["quota_low", "spike"],
            "上限 2 → 按规则顺序取两条"
        );
    }

    #[test]
    fn cap_of_one_keeps_only_the_first_rule_in_declaration_order() {
        // 同一份数据下有两条命中，上限 1 时必须按顺序截断，不能随机留一条
        let settings = AlertSettings {
            max_fires_per_day: 1,
            ..settings()
        };
        let findings = detect(
            &current(5.0, 100.0, 30.0),
            &baseline(0.05, 10.0, 5),
            &settings,
            &all_rules(),
            &DayState::default(),
        );
        assert_eq!(rules(&findings), vec!["quota_low"]);
    }

    #[test]
    fn exhausted_cap_yields_nothing() {
        let state =
            DayState::from_counts(&[("quota_low".to_string(), 1), ("runout_soon".to_string(), 1)]);
        let findings = detect(
            &current(5.0, 100.0, 30.0),
            &baseline(0.05, 10.0, 5),
            &settings(),
            &all_rules(),
            &state,
        );
        assert!(findings.is_empty(), "合计已达上限，第三条也不再投");
    }

    #[test]
    fn zero_cap_silences_everything() {
        let settings = AlertSettings {
            max_fires_per_day: 0,
            ..settings()
        };
        let findings = detect(
            &current(5.0, 100.0, 30.0),
            &baseline(0.05, 10.0, 5),
            &settings,
            &all_rules(),
            &DayState::default(),
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn cap_resumes_next_day_because_state_is_keyed_by_day() {
        // 跨日复位由 alert_state 的 day 主键保证：新的一天查不到行 → DayState 归零
        let yesterday = DayState::from_counts(&[("quota_low".to_string(), 1)]);
        let today = DayState::from_counts(&[]);
        assert_eq!(yesterday.total, 1);
        assert_eq!(today, DayState::default());
    }

    #[test]
    fn day_state_ignores_unknown_rule_keys() {
        let state = DayState::from_counts(&[("legacy_rule".to_string(), 7)]);
        assert_eq!(state.total, 0, "已删除规则的历史行不该吃掉当日额度");
    }

    #[test]
    fn disabled_master_switch_yields_nothing() {
        let settings = AlertSettings {
            enabled: false,
            ..settings()
        };
        let findings = detect(
            &current(1.0, 1.0, 99.0),
            &baseline(1.0, 1.0, 5),
            &settings,
            &all_rules(),
            &DayState::default(),
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn disabled_rules_are_excluded() {
        let findings = detect(
            &current(5.0, 100.0, 30.0),
            &baseline(0.05, 10.0, 5),
            &settings(),
            &["spike".to_string()],
            &DayState::default(),
        );
        assert_eq!(
            rules(&findings),
            vec!["spike"],
            "未启用的 quota_low 不占额度"
        );
    }

    // ---------- 基准 ----------

    #[test]
    fn baseline_ignores_gap_days() {
        // 9-15 ~ 9-18 缺失（应用没运行），窗口内只有 9-19/9-20 两天有用量
        let snapshots = vec![
            snap("2026-09-19", 4.0),
            snap("2026-09-20", 6.0),
            snap("2026-09-21", 99.0),
        ];
        let base = build_baseline(&snapshots, "2026-09-21", super::BASELINE_WINDOW_DAYS)
            .expect("应可计算");
        assert_eq!(base.sample_days, 2, "当日不参与基准");
        assert_eq!(base.avg_yuan, Some(5.0));
        assert_eq!(base.median_yuan, Some(5.0));
    }

    #[test]
    fn baseline_drops_zero_spend_days() {
        let snapshots = vec![
            snap("2026-09-21", 99.0),
            snap("2026-09-20", 0.0),
            snap("2026-09-19", 3.0),
            snap("2026-09-18", 5.0),
            snap("2026-09-17", 7.0),
        ];
        let base = build_baseline(&snapshots, "2026-09-21", super::BASELINE_WINDOW_DAYS)
            .expect("应可计算");
        assert_eq!(base.sample_days, 3);
        assert_eq!(base.avg_yuan, Some(5.0));
        assert_eq!(base.median_yuan, Some(5.0));
    }

    #[test]
    fn baseline_median_averages_two_middle_values() {
        let snapshots = vec![
            snap("2026-09-20", 8.0),
            snap("2026-09-19", 2.0),
            snap("2026-09-18", 6.0),
            snap("2026-09-17", 4.0),
        ];
        let base = build_baseline(&snapshots, "2026-09-21", super::BASELINE_WINDOW_DAYS)
            .expect("应可计算");
        assert_eq!(base.sample_days, 4);
        assert_eq!(base.median_yuan, Some(5.0), "2/4/6/8 取中间两项平均");
    }

    #[test]
    fn baseline_window_excludes_older_days() {
        let snapshots = vec![
            snap("2026-09-20", 1.0),
            snap("2026-09-14", 2.0),   // 窗口首日（today 前 7 天），参与基准
            snap("2026-09-13", 999.0), // 窗口外，不得影响均值
        ];
        let base = build_baseline(&snapshots, "2026-09-21", super::BASELINE_WINDOW_DAYS)
            .expect("应可计算");
        assert_eq!(base.sample_days, 2);
        assert_eq!(base.avg_yuan, Some(1.5));
        assert_eq!(base.median_yuan, Some(1.5));
    }

    #[test]
    fn baseline_collapses_identical_rows() {
        let snapshots = vec![snap("2026-09-20", 3.0), snap("2026-09-20", 3.0)];
        let base = build_baseline(&snapshots, "2026-09-21", super::BASELINE_WINDOW_DAYS)
            .expect("应可计算");
        assert_eq!(base.sample_days, 1, "完全重复的同一日同值行只算一次");
    }

    #[test]
    fn baseline_empty_window_means_no_baseline() {
        let base =
            build_baseline(&[], "2026-09-21", super::BASELINE_WINDOW_DAYS).expect("空序列应可计算");
        assert_eq!(base, Baseline::default());
    }

    #[test]
    fn baseline_rejects_unparsable_day() {
        let error = build_baseline(&[], "not-a-date", super::BASELINE_WINDOW_DAYS)
            .expect_err("非法日期应报错");
        assert_eq!(error.code, crate::error::ErrorCode::Input);
    }

    #[test]
    fn baseline_accepts_rfc3339_dates() {
        let snapshots = vec![snap("2026-09-20T23:50:00+08:00", 6.0)];
        let base = build_baseline(&snapshots, "2026-09-21", super::BASELINE_WINDOW_DAYS)
            .expect("应可计算");
        assert_eq!(base.sample_days, 1);
    }

    #[test]
    fn no_baseline_means_no_rule_fires() {
        let findings = detect_current(&current(1.0, 1.0, 99.0), &Baseline::default());
        assert!(findings.is_empty(), "窗口内完全无记录时不做任何判定");
    }

    // ---------- 设置校验 ----------

    #[test]
    fn settings_are_clamped_into_range() {
        let settings = AlertSettings {
            enabled: true,
            quota_percent: 500.0,
            runout_days: 0,
            spike_multiplier: -1.0,
            max_fires_per_day: 999,
        }
        .sanitized();
        assert_eq!(settings.quota_percent, 99.0);
        assert_eq!(settings.runout_days, 1);
        assert_eq!(settings.spike_multiplier, 1.5);
        assert_eq!(settings.max_fires_per_day, MAX_DAILY_FIRES_LIMIT);
    }

    #[test]
    fn non_finite_settings_fall_back_to_default() {
        let settings = AlertSettings {
            quota_percent: f64::NAN,
            spike_multiplier: f64::INFINITY,
            ..settings()
        }
        .sanitized();
        assert_eq!(settings.quota_percent, 10.0);
        assert_eq!(settings.spike_multiplier, 3.0);
    }

    #[test]
    fn settings_wire_names_are_camel_case() {
        // 锁定与设置页的字段契约：Tauri 不转换嵌套结构体字段名
        let settings: AlertSettings = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "quotaPercent": 12.0,
            "runoutDays": 4,
            "spikeMultiplier": 2.5,
            "maxFiresPerDay": 3,
        }))
        .expect("设置页形态应可反序列化");
        assert_eq!(settings.quota_percent, 12.0);
        assert_eq!(settings.runout_days, 4);
        assert_eq!(settings.spike_multiplier, 2.5);
        assert_eq!(settings.max_fires_per_day, 3);
        let value = serde_json::to_value(settings).expect("应可序列化");
        assert_eq!(value["maxFiresPerDay"], 3);
        assert!(
            value.get("max_fires_per_day").is_none(),
            "不得同时输出两种命名"
        );
    }

    #[test]
    fn param_ranges_cover_every_tunable_field() {
        let ranges = param_ranges();
        assert_eq!(
            ranges.iter().map(|range| range.key).collect::<Vec<_>>(),
            vec![
                "quota_percent",
                "runout_days",
                "spike_multiplier",
                "max_fires_per_day"
            ]
        );
        assert!(ranges.iter().all(|range| range.min < range.max));
        // 上限 0 表示「当日全部静默」，设置页要能表达这个状态
        assert!(
            ranges
                .iter()
                .any(|range| range.key == "max_fires_per_day" && range.min == 0.0)
        );
    }

    #[test]
    fn rules_enabled_rejects_unknown_key() {
        let error = resolve_rules_enabled(&all_rules(), Some(vec!["nope".to_string()]))
            .expect_err("未知规则应拒绝");
        assert_eq!(error.code, crate::error::ErrorCode::Input);
        assert!(error.message.contains("nope"));
    }

    #[test]
    fn rules_enabled_accepts_subset_in_canonical_order() {
        let resolved = resolve_rules_enabled(
            &[],
            Some(vec!["spike".to_string(), "quota_low".to_string()]),
        )
        .expect("子集应接受");
        assert_eq!(resolved, vec!["quota_low".to_string(), "spike".to_string()]);
    }

    #[test]
    fn rules_enabled_none_keeps_current() {
        let current = vec!["quota_low".to_string()];
        assert_eq!(
            resolve_rules_enabled(&current, None).expect("未传应保留"),
            current
        );
    }

    #[test]
    fn rule_keys_are_unique_and_match_declaration_order() {
        let keys = rule_keys();
        assert_eq!(keys.len(), Rule::ALL.len());
        assert_eq!(keys, vec!["quota_low", "runout_soon", "spike"]);
        assert!(
            Rule::ALL.iter().all(|rule| !rule.title().is_empty()),
            "每条规则都要有标题"
        );
    }
}
