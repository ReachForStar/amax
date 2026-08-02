// 看板页：展示当日 Token 消耗、当日费用与剩余额度；下拉刷新 + onPageShow 节流自动刷新
// 偏差记录（台账先例与传递项）：
// 1. 路由一律使用 this.getUIContext().getRouter() 实例而非静态 router：静态接口自 API 18 废弃
//    （当前 targetSdk API 23），异步回调中调全局 router 会闪退（faqs-arkui-388），导航调用补 try/catch。
// 2. getCookie 在密文损坏时会 reject（decryptSecret 抛错）：启动路径（onPageShow → refresh）兜底为
//    无效凭据 → 与认证错误同路径导航回 ConfigPage，不允许启动路径崩溃。
import { Theme } from '../common/Theme';
import { GridBackground } from '../common/GridBackground';
import { formatTokens, formatYuan, ApiError } from '../common/Format';
import { getCookie, saveSnapshot } from '../model/Store';
import { fetchDashboard, DashboardData } from '../model/Api';

const AUTO_REFRESH_INTERVAL_MS = 60_000; // onPageShow 自动刷新节流

// 本地时区 RFC3339：动态计算偏移，任意时区正确（与桌面版 Local::now().to_rfc3339() 同口径）
function localRfc3339(date: Date): string {
  const offsetMin = -date.getTimezoneOffset();
  const sign = offsetMin >= 0 ? '+' : '-';
  const abs = Math.abs(offsetMin);
  const hh = String(Math.floor(abs / 60)).padStart(2, '0');
  const mm = String(abs % 60).padStart(2, '0');
  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}${sign}${hh}:${mm}`;
}

@Entry
@Component
struct DashboardPage {
  @StorageProp('currentBreakpoint') bp: string = 'sm';
  @State data: DashboardData | null = null;
  @State errorMsg: string = '';
  @State refreshing: boolean = false;
  @State lastUpdatedText: string = '等待同步';
  private lastRefreshAt: number = 0;
  // 刷新串行标志：三入口（onPageShow / onRefreshing / 顶栏 ⟳）并发时仅首个执行，对齐桌面版 refresh_lock，
  // 避免重复请求与重复快照；不能用 refreshing 判定——下拉入口进入时 Refresh 组件已把 $$this.refreshing 置 true，会误跳过
  private refreshInFlight: boolean = false;

  async onPageShow(): Promise<void> {
    const now = Date.now();
    if (now - this.lastRefreshAt >= AUTO_REFRESH_INTERVAL_MS) {
      await this.refresh();
    }
  }

  // 凭据无效统一路径：清空栈后替换为配置页（回退键直接退出应用）
  private async gotoConfig(): Promise<void> {
    try {
      const nav = this.getUIContext().getRouter();
      nav.clear();
      await nav.replaceUrl({ url: 'pages/ConfigPage' });
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`跳转配置页失败：${reason}`);
      // 导航失败降级：留在当前页并提示，避免静默卡死
      this.errorMsg = '凭据无效，且跳转配置页失败，请重试';
    }
  }

  // 顶栏 ⚙ 入口：压栈进入配置页（保留看板页以便返回）
  private async openConfig(): Promise<void> {
    try {
      await this.getUIContext().getRouter().pushUrl({ url: 'pages/ConfigPage' });
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`跳转配置页失败：${reason}`);
      this.errorMsg = '跳转配置页失败，请重试';
    }
  }

  async refresh(): Promise<void> {
    if (this.refreshInFlight) {
      return;
    }
    this.refreshInFlight = true;
    try {
      this.refreshing = true;
      this.errorMsg = '';
      try {
        // getCookie 在密文损坏时会 reject：兜底视为无效凭据，与认证错误同路径
        let cookie: string;
        try {
          cookie = (await getCookie()) ?? '';
        } catch (secretError) {
          const reason = secretError instanceof Error ? secretError.message : JSON.stringify(secretError);
          console.error(`凭据读取失败，视为无效凭据：${reason}`);
          await this.gotoConfig();
          return;
        }
        const data = await fetchDashboard(cookie);
        this.data = data;
        this.lastRefreshAt = Date.now();
        this.lastUpdatedText = '刚刚更新';
        await saveSnapshot({
          savedAt: localRfc3339(new Date()),
          todayYuan: data.todayYuan,
          totalTokens: data.todayTokens,
          remaining: data.remaining,
          used: data.used,
          total: data.total,
          requestCount: data.requestCount,
        });
      } catch (error) {
        if (error instanceof ApiError && error.isAuthError) {
          await this.gotoConfig();
          return;
        }
        // 其他错误：保留已有数据，显示重试信息
        this.errorMsg = error instanceof ApiError
          ? error.message + '（点击刷新重试）'
          : '刷新失败，请重试';
      } finally {
        this.refreshing = false;
      }
    } finally {
      this.refreshInFlight = false;
    }
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      GridBackground()
      Refresh({ refreshing: $$this.refreshing }) {
        Scroll() {
          Column({ space: 0 }) {
            // 顶栏
            Row() {
              Text('AMAX / DAILY')
                .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                .fontColor(Theme.accent).letterSpacing(2)
              Blank()
              Text(this.lastUpdatedText).fontSize(10).fontColor(Theme.muted)
              Text('⟳').fontSize(18).fontColor(Theme.muted).margin({ left: 12 })
                .onClick(() => this.refresh())
              Text('⚙').fontSize(16).fontColor(Theme.muted).margin({ left: 12 })
                .onClick(() => this.openConfig())
            }
            .width('100%').padding({ top: 14, bottom: 14 })

            if (this.errorMsg.length > 0) {
              Text(this.errorMsg)
                .fontSize(12).fontColor(Theme.error).backgroundColor(Theme.errorBg)
                .width('100%').padding(10).margin({ bottom: 14 })
            }

            if (this.data !== null) {
              // 账户头
              Text(this.data.displayName).fontSize(20).fontWeight(FontWeight.Bold)
                .fontColor(Theme.ink)
              Text(`${this.data.requestCount} 次请求`)
                .fontFamily(Theme.fontMono).fontSize(10).fontColor(Theme.muted)
                .margin({ top: 6 })

              // COST TODAY
              Column({ space: 8 }) {
                Text('COST TODAY')
                  .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                  .fontColor(Theme.muted).letterSpacing(2)
                Text(formatYuan(this.data.todayYuan))
                  .fontFamily(Theme.fontMono)
                  .fontSize(this.bp === 'sm' ? 40 : 46).fontWeight(600)
                  .fontColor(Theme.ink)
                Text(this.data.logsAvailable ? '' : '日志暂不可用，仅显示额度')
                  .fontSize(11).fontColor(Theme.muted)
              }
              .alignItems(HorizontalAlign.Start)
              .width('100%').padding({ top: 36, bottom: 28 })

              // TOKENS 网格
              Row() {
                Column({ space: 6 }) {
                  Text('TOKENS')
                    .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                    .fontColor(Theme.muted).letterSpacing(2)
                  Text(formatTokens(this.data.todayTokens))
                    .fontFamily(Theme.fontMono).fontSize(18).fontWeight(600).fontColor(Theme.ink)
                }
                .alignItems(HorizontalAlign.Start).layoutWeight(0.9)

                Column({ space: 6 }) {
                  Text('INPUT / OUTPUT')
                    .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                    .fontColor(Theme.muted).letterSpacing(2)
                  Text(`${formatTokens(this.data.todayInput)} / ${formatTokens(this.data.todayOutput)}`)
                    .fontFamily(Theme.fontMono).fontSize(18).fontWeight(600).fontColor(Theme.ink)
                }
                .alignItems(HorizontalAlign.Start).layoutWeight(1.35)
                .border({ width: { left: 1 }, color: Theme.line })
                .padding({ left: 18 })
              }
              .width('100%')
              .padding({ top: 16, bottom: 16 })
              .border({ width: { top: 1, bottom: 1 }, color: Theme.line })

              // 额度块
              Column({ space: 10 }) {
                Row() {
                  Column({ space: 4 }) {
                    Text('AVAILABLE QUOTA')
                      .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
                      .fontColor(Theme.muted).letterSpacing(2)
                    Text(`${this.data.percent.toFixed(1)}%`)
                      .fontFamily(Theme.fontMono).fontSize(28).fontWeight(600).fontColor(Theme.ink)
                  }
                  .alignItems(HorizontalAlign.Start)
                  Blank()
                  Text(`剩余 ${formatYuan(this.data.remaining)}`)
                    .fontFamily(Theme.fontMono).fontSize(11).fontColor(Theme.accent)
                }
                .width('100%')

                Progress({ value: this.data.percent, total: 100, type: ProgressType.Linear })
                  .width('100%').color(Theme.accent)

                Row() {
                  Text(`USED ${formatYuan(this.data.used)}`).fontSize(9).fontFamily(Theme.fontMono)
                    .fontColor(Theme.muted)
                  Blank()
                  Text(`TOTAL ${formatYuan(this.data.total)}`).fontSize(9)
                    .fontFamily(Theme.fontMono).fontColor(Theme.muted)
                }
                .width('100%')
              }
              .width('100%')
              .padding({ left: 16, top: 4, bottom: 4 })
              .border({ width: { left: 2 }, color: Theme.accent })
              .margin({ top: 30 })
            } else {
              Text('正在加载…').fontSize(13).fontColor(Theme.muted).margin({ top: 60 })
            }
          }
          .width('100%')
          .constraintSize({ maxWidth: Theme.contentMaxWidth(this.bp) })
          .padding({ left: Theme.pagePadding(this.bp), right: Theme.pagePadding(this.bp), bottom: 40 })
        }
        .width('100%').height('100%')
        .scrollBar(BarState.Auto)
        .align(Alignment.Top)
      }
      .width('100%').height('100%')
      .onRefreshing(() => this.refresh())
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
