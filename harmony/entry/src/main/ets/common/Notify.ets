// 低余额通知：与桌面版 lib.rs 同行为——剩余额度 percent < 10 时发一次系统通知，
// 模块级标志保证进程生命周期内只发一次；percent 回升到阈值及以上时重置，允许下次再发。
// API 经 devecocli docs 查证（js-apis-notificationmanager）：
// - notificationManager.isNotificationEnabled(): Promise<boolean>（API 11+，查询应用通知授权状态）
// - notificationManager.publish(request: NotificationRequest): Promise<void>
// - NotificationRequest.content.normal 含 title / text / additionalText（基础文本类型）
import { notificationManager } from '@kit.NotificationKit';

// 与桌面版一致的阈值：剩余额度百分比低于 10 触发
const LOW_QUOTA_THRESHOLD = 10.0;
// 固定通知 ID：重复发布时新通知覆盖旧通知（文档：ID 与标签相同即更新）
const LOW_QUOTA_NOTIFY_ID = 1;

// 启动周期内只发一次；percent 回升到阈值及以上时重置
let lowQuotaNotified = false;

export async function checkLowQuota(percent: number, remaining: number, total: number): Promise<void> {
  if (percent >= LOW_QUOTA_THRESHOLD) {
    // 额度回升（如充值后），重置标志允许后续再次告警
    lowQuotaNotified = false;
    return;
  }
  if (lowQuotaNotified) {
    return;
  }
  try {
    // 未开启通知授权静默跳过，避免 publish 抛 1600004（Notification disabled）
    const enabled = await notificationManager.isNotificationEnabled();
    if (!enabled) {
      return;
    }
    const request: notificationManager.NotificationRequest = {
      id: LOW_QUOTA_NOTIFY_ID,
      content: {
        notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
        normal: {
          title: 'AMAX 额度不足',
          text: `剩余 ¥${remaining.toFixed(2)} / ¥${total.toFixed(2)} (${percent.toFixed(1)}%)，请及时充值`,
          additionalText: '',
        },
      },
    };
    await notificationManager.publish(request);
    lowQuotaNotified = true;
  } catch (error) {
    // 通知失败不影响主刷新流程，仅记日志不抛出
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    console.error(`低余额通知失败：${reason}`);
  }
}
