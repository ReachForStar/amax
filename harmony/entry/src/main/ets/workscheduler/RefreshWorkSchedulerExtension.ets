// 延迟任务（WorkScheduler）后台刷新扩展：应用退后台后由系统按条件调度，执行一次完整刷新
// （initStore → getCookie → fetchDashboard → saveSnapshot → checkLowQuota），积累每日快照。
//
// 明确约束（系统硬限制，文档查证 work-scheduler / js-apis-resourceschedule-workscheduler）：
// 1. 延迟任务触发时机由系统按条件调度（网络/充电/空闲、内存、功耗、温度、用户使用习惯等），
//    非精确周期——无法复刻桌面版 10 分钟精确刷新周期，仅用于机会性后台快照积累。
// 2. 执行频率按应用活跃分组分级管控：最小间隔 2 小时（活跃分组），低活跃应用间隔更长
//    （4/24/48 小时），受限或从未使用分组直接禁止调度。
// 3. 单次回调最长运行 2 分钟，超时系统终止 Extension 进程；本流程两次网络请求各 30s 超时，可完成。
// 4. WorkSchedulerExtensionAbility 运行在独立 Extension 进程，模块级状态（含 Store 单例）
//    与主进程隔离，须在回调内重新 initStore；不支持 backgroundTaskManager 等部分模块引用。
import { WorkSchedulerExtensionAbility, workScheduler } from '@kit.BackgroundTasksKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { initStore, getCookie, saveSnapshot } from '../model/Store';
import { fetchDashboard } from '../model/Api';
import { localRfc3339 } from '../common/Format';
import { checkLowQuota } from '../common/Notify';

const TAG = 'RefreshWorkScheduler';
// 任务唯一 ID：startWork/stopWork/getWorkStatus 均按此 ID 识别任务
const WORK_ID = 100;
const BUNDLE_NAME = 'com.amax.dashboard';
// 与 module.json5 extensionAbilities.name 一致
const ABILITY_NAME = 'RefreshWorkSchedulerExtension';

// 构造延迟任务申请参数。文档查证（WorkInfo）：workId/bundleName/abilityName 为必填项，
// 且至少设置一个触发条件——本任务依赖网络拉取数据，取 NETWORK_TYPE_ANY；
// 非循环、不持久化（重启后由下次退后台重新申请，避免残留无效任务）
function buildWorkInfo(): workScheduler.WorkInfo {
  const workInfo: workScheduler.WorkInfo = {
    workId: WORK_ID,
    bundleName: BUNDLE_NAME,
    abilityName: ABILITY_NAME,
    networkType: workScheduler.NetworkType.NETWORK_TYPE_ANY,
    isRepeat: false,
    isPersisted: false,
  };
  return workInfo;
}

// 申请一次后台延迟刷新：先 stopWork 移除可能残留的同 ID 任务防重复，再 startWork。
// 两处均同步接口、失败抛异常（401/9700004 等），分别 try/catch 仅记日志不向调用方抛出
export function scheduleBackgroundRefresh(): void {
  const workInfo = buildWorkInfo();
  try {
    // needCancel=true：停止并移除旧任务；首次申请时系统中无此任务，失败属预期，忽略继续
    workScheduler.stopWork(workInfo, true);
  } catch (error) {
    const err = error as BusinessError;
    console.warn(`${TAG} stopWork 失败（首次申请无旧任务属预期）：code=${err.code}, message=${err.message}`);
  }
  try {
    workScheduler.startWork(workInfo);
    console.info(`${TAG} startWork 成功，等待系统按条件调度后台刷新`);
  } catch (error) {
    const err = error as BusinessError;
    console.error(`${TAG} startWork 失败：code=${err.code}, message=${err.message}`);
  }
}

export default class RefreshWorkSchedulerExtension extends WorkSchedulerExtensionAbility {
  // 延迟任务开始回调：执行一次完整刷新，无论成败都重新申请下一次，形成机会性周期积累
  onWorkStart(workInfo: workScheduler.WorkInfo): void {
    console.info(`${TAG} onWorkStart，workId=${workInfo.workId}`);
    // 异步执行不阻塞回调返回；2 分钟时限内可完成（两次请求各 30s 超时 + 本地读写）
    this.runRefresh();
  }

  // 延迟任务结束回调（2 分钟超时或 stopWork 取消时触发）：无需清理资源，仅记日志
  onWorkStop(workInfo: workScheduler.WorkInfo): void {
    console.info(`${TAG} onWorkStop，workId=${workInfo.workId}`);
  }

  private async runRefresh(): Promise<void> {
    try {
      // this.context 为 WorkSchedulerExtensionContext（继承 ExtensionContext），可作 Context
      // 初始化 preferences + RDB；Extension 独立进程，Store 单例须在本进程重新初始化
      await initStore(this.context);
      // getCookie 在密文损坏时会 reject；未配置凭据返回 null，均跳过本次刷新
      let cookie: string | null = null;
      try {
        cookie = await getCookie();
      } catch (secretError) {
        const reason = secretError instanceof Error ? secretError.message : JSON.stringify(secretError);
        console.error(`${TAG} 凭据解密失败，跳过本次后台刷新：${reason}`);
        return;
      }
      if (cookie === null || cookie.length === 0) {
        console.info(`${TAG} 未配置凭据，跳过本次后台刷新`);
        return;
      }
      // 与前台刷新同口径：账户额度 + 当日汇总（汇总失败仍返回额度，Api 层已容错）
      const data = await fetchDashboard(cookie);
      await saveSnapshot({
        savedAt: localRfc3339(new Date()),
        todayYuan: data.todayYuan,
        totalTokens: data.todayTokens,
        remaining: data.remaining,
        used: data.used,
        total: data.total,
        requestCount: data.requestCount,
      });
      // 后台刷新同样触发低余额检查（Extension 进程内标志独立，与前台进程互不影响）
      await checkLowQuota(data.percent, data.remaining, data.total);
      console.info(`${TAG} 后台刷新完成：剩余 ¥${data.remaining.toFixed(2)}（${data.percent.toFixed(1)}%）`);
    } catch (error) {
      // 认证失败（cookie 过期）等所有异常仅记日志：Extension 进程无法导航回配置页，
      // 下次前台刷新会走既有认证错误路径；不静默吞错、不崩溃 Extension 进程
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`${TAG} 后台刷新失败：${reason}`);
    } finally {
      // 成功或失败都重新申请下一次延迟任务（一次性任务执行后即从队列移除）
      scheduleBackgroundRefresh();
    }
  }
}
