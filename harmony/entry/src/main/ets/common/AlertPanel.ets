// 设置页的告警区：规则参数 + 通知渠道 + 当日投递复盘
// 对应桌面端设置页的「告警规则」「通知渠道」两块，差别只在手机端没有 SMTP 渠道。
// 数值参数一律在发出保存前就校验：空框会被后端夹到区间下限、小数会毁掉整数参数，
// 静默夹取等于让用户亮出一个并不生效的数字。
import { Theme } from './Theme';
import { AlertSettings, RULE_KEYS, paramRanges } from './Alert';
import { Channel, deliver } from './Deliver';
import { AlertStatus, channelAvailability, readAlertStatus } from '../model/Alerts';
import {
  AlertChannels, DeliveryLog, getAlertChannels, getAlertRulesEnabled, getAlertSettings,
  recordAlertDelivery, setAlertChannels, setAlertRulesEnabled, setAlertSettings, recentAlertDeliveries,
} from '../model/Store';
import { localRfc3339, todayStr } from './Format';

/// 四个可调参数：key 与桌面端 param_ranges 一致，label 用于报错文案
interface ParamSpec {
  key: string;
  label: string;
  integer: boolean;
}

const PARAM_SPECS: ParamSpec[] = [
  { key: 'quota_percent', label: '剩余额度阈值（%）', integer: false },
  { key: 'runout_days', label: '可用天数（天）', integer: true },
  { key: 'spike_multiplier', label: '花费倍数（倍）', integer: false },
  { key: 'max_fires_per_day', label: '每日最多通知（次）', integer: true },
];

/// 规则行：rule 为空串的是「每日最多通知」，它没有独立开关
interface RuleRow {
  rule: string;
  label: string;
  paramIndex: number;
  unit: string;
}

const RULE_ROWS: RuleRow[] = [
  { rule: 'quota_low', label: '剩余额度低于', paramIndex: 0, unit: '%' },
  { rule: 'runout_soon', label: '按近期日均还能用不足', paramIndex: 1, unit: '天' },
  { rule: 'spike', label: '当日花费达近 7 日中位数的', paramIndex: 2, unit: '倍' },
  { rule: '', label: '每日最多通知', paramIndex: 3, unit: '次（0 为当日全部静默）' },
];

/// 渠道中文名：CHANNEL_LABELS(channel)
function channelLabel(channel: string): string {
  switch (channel) {
    case 'notification': return '系统通知';
    case 'meow': return 'MeoW 推送';
    default: return channel;
  }
}

/// 可用性文案：AVAILABILITY_LABELS(state)
function availabilityLabelText(state: string): string {
  switch (state) {
    case 'ready': return '可用';
    case 'incomplete': return '缺配置';
    case 'off': return '未开启';
    default: return state;
  }
}

/// 告警状态文案：ALERT_STATE_LABELS(state)
function alertStateLabel(state: string): string {
  switch (state) {
    case 'disabled': return '已停用';
    case 'armed': return '待触发';
    case 'cap_hit': return '今日已达上限';
    default: return state;
  }
}

/// 当日投递记录条数：只复盘最近几条，看板上不需要全量
const DELIVERY_LOG_LIMIT: number = 5;

@Component
export struct AlertPanel {
  @State private alertEnabled: boolean = true;
  @State private ruleOn: boolean[] = [true, true, true];
  @State private texts: string[] = ['10', '5', '3', '2'];
  @State private notificationOn: boolean = true;
  @State private meowOn: boolean = false;
  @State private nickname: string = '';
  @State private meowAvailable: string = '缺配置';
  @State private notificationAvailable: string = '可用';
  @State private progress: string = '--';
  @State private deliveries: DeliveryLog[] = [];
  @State private statusText: string = '';
  @State private statusIsError: boolean = false;
  @State private busy: boolean = false;
  private readonly ranges = paramRanges();

  async aboutToAppear(): Promise<void> {
    await this.reload();
  }

  private setStatus(text: string, isError: boolean): void {
    this.statusText = text;
    this.statusIsError = isError;
  }

  private setText(index: number, value: string): void {
    const next = this.texts.slice();
    next[index] = value;
    this.texts = next;
  }

  private setRuleOn(index: number, value: boolean): void {
    const next = this.ruleOn.slice();
    next[index] = value;
    this.ruleOn = next;
  }

  /// 重新读一遍本机配置与账本：保存后也走这里，界面显示的是存进去的值而不是提交值
  private async reload(): Promise<void> {
    try {
      const settings = await getAlertSettings();
      const rules = await getAlertRulesEnabled();
      const channels = await getAlertChannels();
      this.alertEnabled = settings.enabled;
      this.ruleOn = RULE_KEYS.map((key: string): boolean => rules.includes(key));
      this.texts = [
        String(settings.quotaPercent),
        String(settings.runoutDays),
        String(settings.spikeMultiplier),
        String(settings.maxFiresPerDay),
      ];
      this.notificationOn = channels.notification;
      this.meowOn = channels.meow;
      this.nickname = channels.meowNickname === null ? '' : channels.meowNickname;
      this.applyAvailability(channels);
      await this.reloadStatus();
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      this.setStatus(`读取告警设置失败：${reason}`, true);
    }
  }

  private applyAvailability(channels: AlertChannels): void {
    this.notificationAvailable = this.availabilityLabel(channels, 'notification');
    this.meowAvailable = this.availabilityLabel(channels, 'meow');
  }

  private availabilityLabel(channels: AlertChannels, channel: Channel): string {
    return availabilityLabelText(channelAvailability(channels, channel));
  }

  private async reloadStatus(): Promise<void> {
    const status: AlertStatus | null = await readAlertStatus();
    this.progress = status === null ? '今日次数不可读'
      : `${alertStateLabel(status.state)} · 今天 ${status.firedToday}/${status.cap} 次`;
    try {
      this.deliveries = await recentAlertDeliveries(todayStr(), DELIVERY_LOG_LIMIT);
    } catch (error) {
      // 明细只是复盘，读不到不该挡住规则与渠道的编辑
      this.deliveries = [];
    }
  }

  /// 把四个输入框读成参数；不合法时直接把话说清楚并返回 null
  private readParams(): AlertSettings | null {
    const missing: string[] = [];
    const invalid: string[] = [];
    const values: number[] = [0, 0, 0, 0];
    for (let i = 0; i < PARAM_SPECS.length; i++) {
      const raw = this.texts[i].trim();
      const spec = PARAM_SPECS[i];
      if (raw.length === 0) {
        missing.push(spec.label);
        continue;
      }
      const value = Number(raw);
      if (!Number.isFinite(value)) {
        invalid.push(`${spec.label}不是数字`);
        continue;
      }
      if (spec.integer && !Number.isInteger(value)) {
        invalid.push(`${spec.label}需为整数`);
        continue;
      }
      values[i] = value;
    }
    if (missing.length > 0 || invalid.length > 0) {
      const parts: string[] = [];
      if (missing.length > 0) {
        parts.push(`请填写：${missing.join('、')}`);
      }
      if (invalid.length > 0) {
        parts.push(invalid.join('；'));
      }
      this.setStatus(parts.join('；'), true);
      return null;
    }
    const settings: AlertSettings = {
      enabled: this.alertEnabled,
      quotaPercent: values[0],
      runoutDays: values[1],
      spikeMultiplier: values[2],
      maxFiresPerDay: values[3],
    };
    return settings;
  }

  private enabledRuleKeys(): string[] {
    return RULE_KEYS.filter((key: string, index: number): boolean => this.ruleOn[index]);
  }

  async saveAlertSettings(): Promise<void> {
    const settings = this.readParams();
    if (settings === null) {
      return;
    }
    this.busy = true;
    try {
      const saved = await setAlertSettings(settings);
      const rules = await setAlertRulesEnabled(this.enabledRuleKeys());
      this.alertEnabled = saved.enabled;
      this.ruleOn = RULE_KEYS.map((key: string): boolean => rules.includes(key));
      this.texts = [
        String(saved.quotaPercent),
        String(saved.runoutDays),
        String(saved.spikeMultiplier),
        String(saved.maxFiresPerDay),
      ];
      await this.reloadStatus();
      this.setStatus('已保存，下一轮刷新生效', false);
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      this.setStatus(`保存失败：${reason}`, true);
    } finally {
      this.busy = false;
    }
  }

  async saveChannels(): Promise<void> {
    this.busy = true;
    try {
      const saved = await setAlertChannels({
        notification: this.notificationOn,
        meow: this.meowOn,
        meowNickname: this.nickname,
      });
      this.notificationOn = saved.notification;
      this.meowOn = saved.meow;
      this.nickname = saved.meowNickname === null ? '' : saved.meowNickname;
      this.applyAvailability(saved);
      await this.reloadStatus();
      this.setStatus('已保存通知渠道', false);
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      this.setStatus(`保存失败：${reason}`, true);
    } finally {
      this.busy = false;
    }
  }

  /// 测试投递：走真实渠道但不消耗任何规则的当日额度（账本记在 rule='test' 下）
  async testChannel(channel: Channel): Promise<void> {
    const nickname = this.nickname.trim();
    if (channel === 'meow' && nickname.length === 0) {
      this.setStatus('MeoW 需要先填写昵称', true);
      return;
    }
    this.busy = true;
    try {
      const receipt = await deliver(channel, 'test', nickname, '✅ AMAX 测试投递',
        '这是一条来自 AMAX Dashboard 的测试消息。');
      await recordAlertDelivery('test', todayStr(), channel, receipt.outcome === 'sent',
        receipt.error, localRfc3339(new Date()));
      await this.reloadStatus();
      const label = channelLabel(channel);
      if (receipt.outcome === 'sent') {
        this.setStatus(`${label}：已投递`, false);
      } else {
        this.setStatus(`${label}：${receipt.error === null ? '投递失败' : receipt.error}`, true);
      }
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      this.setStatus(`测试投递失败：${reason}`, true);
    } finally {
      this.busy = false;
    }
  }

  private rangeOf(index: number): string {
    const range = this.ranges[index];
    return `${range.min}~${range.max}`;
  }

  build() {
    Column({ space: 18 }) {
      // ── 告警规则 ──
      Column({ space: 10 }) {
        Row() {
          Text('告警规则')
            .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
            .fontColor(Theme.accent).letterSpacing(2)
          Blank()
          Text(this.progress).fontSize(10).fontFamily(Theme.fontMono).fontColor(Theme.muted)
        }
        .width('100%')

        Text('每条规则同日只通知一次，三条共用「每日最多通知次数」；改完点保存才对下一轮刷新生效。')
          .fontSize(11).fontColor(Theme.muted)

        ForEach(RULE_ROWS, (row: RuleRow) => {
          Row({ space: 8 }) {
            if (row.rule.length > 0) {
              Toggle({ type: ToggleType.Checkbox, isOn: this.ruleOn[RULE_KEYS.indexOf(row.rule)] })
                .width(18).height(18).selectedColor(Theme.accent)
                .onChange((isOn: boolean) => {
                  this.setRuleOn(RULE_KEYS.indexOf(row.rule), isOn);
                })
            }
            Text(row.label).fontSize(13).fontColor(Theme.ink)
            TextInput({ text: this.texts[row.paramIndex] })
              .type(row.paramIndex === 1 || row.paramIndex === 3
                ? InputType.Number : InputType.NUMBER_DECIMAL)
              .width(64).height(32).fontSize(12).fontFamily(Theme.fontMono)
              .backgroundColor(Theme.surfaceRaised)
              .onChange((value: string) => {
                this.setText(row.paramIndex, value);
              })
            Text(row.unit).fontSize(11).fontColor(Theme.muted)
            Blank()
          }
          .width('100%')
        }, (row: RuleRow) => `${row.rule}-${row.paramIndex}`)

        Row({ space: 8 }) {
          Toggle({ type: ToggleType.Switch, isOn: this.alertEnabled })
            .selectedColor(Theme.accent)
            .onChange((isOn: boolean) => {
              this.alertEnabled = isOn;
            })
          Text('启用额度告警').fontSize(13).fontColor(Theme.ink)
        }
        .width('100%')

        Text(`取值区间：${PARAM_SPECS[0].label} ${this.rangeOf(0)}、${PARAM_SPECS[1].label} ${this.rangeOf(1)}、` +
          `${PARAM_SPECS[2].label} ${this.rangeOf(2)}、${PARAM_SPECS[3].label} ${this.rangeOf(3)}`)
          .fontSize(10).fontColor(Theme.muted)

        Button('保存告警设置')
          .width('100%').height(44)
          .backgroundColor(Theme.accent).fontColor('#f7faf7')
          .borderRadius(4).enabled(!this.busy)
          .onClick(() => {
            this.saveAlertSettings();
          })
      }
      .width('100%')
      .padding(16)
      .border({ width: 1, color: Theme.line, radius: 6 })

      // ── 通知渠道 ──
      Column({ space: 10 }) {
        Text('通知渠道')
          .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
          .fontColor(Theme.accent).letterSpacing(2)
        Text('任一渠道成功即算这条告警送出去了；全部失败也记一次，避免每轮刷新撞同一堵墙。')
          .fontSize(11).fontColor(Theme.muted)

        Row({ space: 8 }) {
          Toggle({ type: ToggleType.Switch, isOn: this.notificationOn })
            .selectedColor(Theme.accent)
            .onChange((isOn: boolean) => {
              this.notificationOn = isOn;
            })
          Text('系统通知').fontSize(13).fontColor(Theme.ink)
          Blank()
          Text(this.notificationAvailable).fontSize(10).fontFamily(Theme.fontMono).fontColor(Theme.muted)
          Button('测试').height(30).fontSize(12)
            .backgroundColor(Theme.surfaceRaised).fontColor(Theme.accent)
            .enabled(!this.busy)
            .onClick(() => {
              this.testChannel('notification');
            })
        }
        .width('100%')

        Row({ space: 8 }) {
          Toggle({ type: ToggleType.Switch, isOn: this.meowOn })
            .selectedColor(Theme.accent)
            .onChange((isOn: boolean) => {
              this.meowOn = isOn;
            })
          Text('MeoW 推送').fontSize(13).fontColor(Theme.ink)
          Blank()
          Text(this.meowAvailable).fontSize(10).fontFamily(Theme.fontMono).fontColor(Theme.muted)
          Button('测试').height(30).fontSize(12)
            .backgroundColor(Theme.surfaceRaised).fontColor(Theme.accent)
            .enabled(!this.busy)
            .onClick(() => {
              this.testChannel('meow');
            })
        }
        .width('100%')

        TextInput({ placeholder: '如 小明', text: this.nickname })
          .height(44).fontSize(12).backgroundColor(Theme.surfaceRaised)
          .onChange((value: string) => {
            this.nickname = value;
          })
        Text('推送按昵称路由到你的手机；昵称明文只存本机，清空并保存即删除。')
          .fontSize(11).fontColor(Theme.muted)

        Button('保存通知渠道')
          .width('100%').height(44)
          .backgroundColor(Theme.accent).fontColor('#f7faf7')
          .borderRadius(4).enabled(!this.busy)
          .onClick(() => {
            this.saveChannels();
          })
      }
      .width('100%')
      .padding(16)
      .border({ width: 1, color: Theme.line, radius: 6 })

      // ── 当日投递 ──
      Column({ space: 6 }) {
        Text('当日投递')
          .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
          .fontColor(Theme.accent).letterSpacing(2)
        if (this.deliveries.length === 0) {
          Text('今天还没有投递记录').fontSize(11).fontColor(Theme.muted)
        }
        ForEach(this.deliveries, (log: DeliveryLog) => {
          Text(`${log.deliveredAt.slice(11, 16)} ${channelLabel(log.channel)} ` +
            `${log.ok ? '成功' : `失败：${log.error ?? '未知原因'}`}`)
            .fontSize(11).fontColor(log.ok ? Theme.muted : Theme.error)
            .width('100%')
        }, (log: DeliveryLog) => `${log.deliveredAt}-${log.ruleKey}-${log.channel}`)
      }
      .width('100%')
      .padding(16)
      .border({ width: 1, color: Theme.line, radius: 6 })

      if (this.statusText.length > 0) {
        Text(this.statusText)
          .fontSize(12).fontColor(this.statusIsError ? Theme.error : Theme.accent)
          .backgroundColor(this.statusIsError ? Theme.errorBg : Theme.accentSoft)
          .width('100%').padding(10).borderRadius(2)
      }
    }
    .width('100%')
  }
}
