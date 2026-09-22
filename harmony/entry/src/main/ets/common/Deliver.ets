// 告警投递层（手机端两个渠道：系统通知 + MeoW 推送；SMTP 邮件是桌面端专属）
// 与桌面端 src-tauri/src/deliver.rs 同一套收口语义：
// - 任一渠道成功即算这条告警送出去了（sent）
// - 全部永久失败也算送过（failed），否则每轮刷新都撞同一堵墙
// - 只有「还能重试」时不消耗当日额度（retryable），留给下一轮，并由
//   MAX_FAILED_DELIVERIES_PER_RULE 给失败行数封顶
// MeoW 推送端点按昵称路由、无鉴权：POST /{nickname}/{title}?msgType=markdown
// 本文件不碰数据库、不读配置，凭据与开关由 model/Alerts.ets 取好再传进来。
import { http } from '@kit.NetworkKit';
import { notificationManager } from '@kit.NotificationKit';
import { RULE_KEYS } from './Alert';

export const MEOW_BASE_URL: string = 'https://api.chuckfang.com';
/// 单规则单日的失败投递行上限（开了几个渠道就一次计几行）
export const MAX_FAILED_DELIVERIES_PER_RULE: number = 3;
/// 推送超时单独设小：后台延迟任务单次回调只有 2 分钟预算，
/// 已经要留给两次各 30 秒的官网请求，不该再被一条通知拖满
const MEOW_TIMEOUT_MS: number = 15_000;
/// 失败原因入库/上界面时截断，避免把整页 HTML 错误页存进账本
const ERROR_BRIEF_CHARS: number = 200;

export type Outcome = 'sent' | 'failed' | 'retryable';
export type Channel = 'notification' | 'meow';

/// 一次渠道投递的结果（对应桌面端 Receipt）
export interface Receipt {
  channel: Channel;
  outcome: Outcome;
  error: string | null;
}

function sent(channel: Channel): Receipt {
  return { channel: channel, outcome: 'sent', error: null };
}

function failed(channel: Channel, error: string): Receipt {
  return { channel: channel, outcome: 'failed', error: brief(error) };
}

function retryable(channel: Channel, error: string): Receipt {
  return { channel: channel, outcome: 'retryable', error: brief(error) };
}

function brief(text: string): string {
  return text.length > ERROR_BRIEF_CHARS ? `${text.slice(0, ERROR_BRIEF_CHARS)}…` : text;
}

/// 多渠道结果收口：任一成功即 sent，否则有可重试即 retryable，其余 failed
export function summarize(receipts: Receipt[]): Outcome {
  if (receipts.some((receipt: Receipt): boolean => receipt.outcome === 'sent')) {
    return 'sent';
  }
  if (receipts.some((receipt: Receipt): boolean => receipt.outcome === 'retryable')) {
    return 'retryable';
  }
  return 'failed';
}

/// 系统通知的固定 ID：同一条规则重复推送时覆盖旧通知（文档：ID 与标签相同即更新），
/// 三条规则各占一个 ID，互不顶掉
export function notificationId(rule: string): number {
  const index = RULE_KEYS.indexOf(rule);
  return index >= 0 ? index + 1 : RULE_KEYS.length + 1;
}

/// 系统通知。未授权与发送失败都记永久失败：重试不会让授权出现。
export async function sendNotification(rule: string, title: string, body: string): Promise<Receipt> {
  const channel: Channel = 'notification';
  try {
    // 未开启通知授权时 publish 抛 1600004（Notification disabled），先查状态给出可读原因
    if (!(await notificationManager.isNotificationEnabled())) {
      return failed(channel, '系统通知未授权，请在设置里允许本应用通知');
    }
    const request: notificationManager.NotificationRequest = {
      id: notificationId(rule),
      content: {
        notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
        normal: {
          title: title,
          text: body,
          additionalText: '',
        },
      },
    };
    await notificationManager.publish(request);
    return sent(channel);
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    return failed(channel, `系统通知发送失败: ${reason}`);
  }
}

/// MeoW 端点：昵称与标题都百分号编码（中文昵称、含 `/` 的昵称都会改写路径段）
export function meowUrl(nickname: string, title: string): string {
  return `${MEOW_BASE_URL}/${encodeURIComponent(nickname)}/${encodeURIComponent(title)}?msgType=markdown`;
}

interface MeowResponse {
  status?: number;
  data?: boolean;
}

/// 响应是否被受理：HTTP 2xx 且（`status == 200` 或 `data == true`）。
/// 2xx 但根本不是 JSON 时不猜业务失败，按 HTTP 状态算成功（与桌面版同口径）；
/// 能解析却不是对象（标量、数组）按未受理处理。
export function meowAccepted(body: string): boolean {
  const trimmed = body.trim();
  if (!trimmed.startsWith('{')) {
    try {
      JSON.parse(trimmed);
      return false;
    } catch (error) {
      return true;
    }
  }
  try {
    const value = JSON.parse(trimmed) as MeowResponse;
    return value.status === 200 || value.data === true;
  } catch (error) {
    return true;
  }
}

/// 响应分类：429 与 5xx 可重试；其余非 2xx 永久失败；2xx 看响应体
export function classifyMeow(status: number, body: string): Outcome {
  if (status === 429 || status >= 500) {
    return 'retryable';
  }
  if (status < 200 || status >= 300) {
    return 'failed';
  }
  return meowAccepted(body) ? 'sent' : 'failed';
}

function meowRequest(url: string, payload: string): Promise<http.HttpResponse> {
  return new Promise((resolve, reject) => {
    const req = http.createHttp();
    const header: Record<string, string> = { 'Content-Type': 'application/json' };
    req.request(url, {
      method: http.RequestMethod.POST,
      header: header,
      extraData: payload,
      connectTimeout: MEOW_TIMEOUT_MS,
      readTimeout: MEOW_TIMEOUT_MS,
    }, (err, data: http.HttpResponse) => {
      req.destroy();
      if (err) {
        reject(new Error(`MeoW 请求失败: ${err.message}`));
        return;
      }
      resolve(data);
    });
  });
}

/// MeoW 推送。官网 Cookie 与 API Key 一律不带——端点按昵称路由、无鉴权。
export async function sendMeow(nickname: string, title: string, body: string): Promise<Receipt> {
  const channel: Channel = 'meow';
  const payload = JSON.stringify({ msg: body, title: title });
  try {
    const response = await meowRequest(meowUrl(nickname, title), payload);
    const text = typeof response.result === 'string' ? response.result : JSON.stringify(response.result);
    const outcome = classifyMeow(response.responseCode, text);
    if (outcome === 'sent') {
      return sent(channel);
    }
    const detail = `MeoW 响应 ${response.responseCode}: ${text}`;
    return outcome === 'retryable' ? retryable(channel, detail) : failed(channel, detail);
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    return retryable(channel, reason);
  }
}

/// 按渠道发一条（测试投递与正式告警共用）。缺昵称等不可用情况由调用方先拦住。
export async function deliver(channel: Channel, rule: string, nickname: string,
  title: string, body: string): Promise<Receipt> {
  return channel === 'meow'
    ? sendMeow(nickname, title, body)
    : sendNotification(rule, title, body);
}
