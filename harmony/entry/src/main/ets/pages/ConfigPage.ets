// 配置页：WebView 登录入口（推荐）+ 手动粘贴 Session Cookie / API Key 并保存验证
// 保存后即时调用 fetchDashboard 验证凭据；成功则清空页面栈并替换为 Dashboard，避免回退到配置页
// 偏差记录（docs 复核）：路由使用 this.getUIContext().getRouter() 实例而非静态 router
// （静态接口自 API 18 废弃，当前 targetSdk API 23；官方推荐 UIContext 实例）
import { Theme } from '../common/Theme';
import { GridBackground } from '../common/GridBackground';
import { ApiError } from '../common/Format';
import { getCookie, saveCookie, saveApiKey, clearCredentials } from '../model/Store';
import { fetchDashboard } from '../model/Api';

@Entry
@Component
struct ConfigPage {
  @StorageProp('currentBreakpoint') bp: string = 'sm';
  @State cookieInput: string = '';
  @State apiKeyInput: string = '';
  @State statusText: string = '';
  @State statusIsError: boolean = false;
  @State busy: boolean = false;
  @State hasCookie: boolean = false;

  async aboutToAppear(): Promise<void> {
    this.hasCookie = (await getCookie()) !== null;
  }

  setStatus(text: string, isError: boolean): void {
    this.statusText = text;
    this.statusIsError = isError;
  }

  // 清空栈后替换为 Dashboard：回退键直接退出应用，不回配置/登录页
  async gotoDashboard(): Promise<void> {
    try {
      const nav = this.getUIContext().getRouter();
      nav.clear();
      await nav.replaceUrl({ url: 'pages/DashboardPage' });
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`跳转看板失败：${reason}`);
      this.setStatus('跳转看板失败，请重试', true);
      this.busy = false;
    }
  }

  async gotoLogin(): Promise<void> {
    try {
      await this.getUIContext().getRouter().pushUrl({ url: 'pages/LoginPage' });
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`跳转登录页失败：${reason}`);
      this.setStatus('跳转登录页失败，请重试', true);
    }
  }

  async saveManual(): Promise<void> {
    const cookie = this.cookieInput.trim();
    const apiKey = this.apiKeyInput.trim();
    if (cookie.length === 0 && apiKey.length === 0) {
      this.setStatus('请至少填写一项认证信息', true);
      return;
    }
    this.busy = true;
    try {
      if (cookie.length > 0) {
        await saveCookie(cookie, 'manual');
      }
      if (apiKey.length > 0) {
        await saveApiKey(apiKey);
      }
      // 保存后立即验证：仅凭 API Key 时 getCookie 为 null，fetchDashboard 会按认证错误提示
      const stored = await getCookie();
      await fetchDashboard(stored === null ? '' : stored);
      await this.gotoDashboard();
    } catch (error) {
      const message = error instanceof ApiError ? error.message : '保存失败，请重试';
      this.setStatus(message, true);
      this.busy = false;
    }
  }

  async relogin(): Promise<void> {
    await clearCredentials();
    this.hasCookie = false;
    this.setStatus('', false);
    await this.gotoLogin();
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      GridBackground()
      Scroll() {
        Column({ space: 22 }) {
          Text('AMAX / ACCESS')
            .fontFamily(Theme.fontMono).fontSize(10).fontWeight(FontWeight.Bold)
            .fontColor(Theme.accent).letterSpacing(2)
          Text('连接你的 AMAX 账户').fontSize(26).fontWeight(FontWeight.Bold)
            .fontColor(Theme.ink).margin({ top: 10 })
          Text('凭据只保存在当前设备的本地应用数据中（HUKS 加密）。')
            .fontSize(13).fontColor(Theme.muted)

          if (this.hasCookie) {
            Button('重新登录（清除现有凭据）')
              .width('100%').height(44)
              .backgroundColor(Theme.surfaceRaised).fontColor(Theme.accent)
              .border({ width: 1, color: Theme.accent, radius: 4 })
              .onClick(() => this.relogin())
          }

          Button('前往登录（推荐）')
            .width('100%').height(48)
            .backgroundColor(Theme.accent).fontColor('#f7faf7')
            .fontSize(15).fontWeight(FontWeight.Medium).borderRadius(4)
            .enabled(!this.busy)
            .onClick(() => this.gotoLogin())

          Column({ space: 10 }) {
            Text('或手动粘贴 Session Cookie')
              .fontSize(13).fontWeight(FontWeight.Medium).fontColor(Theme.ink)
            TextArea({ placeholder: 'session=MTc4MzQyOTkyN3xE...', text: this.cookieInput })
              .height(80).fontSize(12).fontFamily(Theme.fontMono)
              .backgroundColor(Theme.surfaceRaised)
              .onChange((value: string) => {
                this.cookieInput = value;
              })
            Text('从桌面浏览器 F12 → Application → Cookies 复制 session 值')
              .fontSize(11).fontColor(Theme.muted)
            TextInput({ placeholder: 'API Key（可选）', text: this.apiKeyInput })
              .height(44).fontSize(12).fontFamily(Theme.fontMono)
              .backgroundColor(Theme.surfaceRaised)
              .onChange((value: string) => {
                this.apiKeyInput = value;
              })
            Button('保存并验证')
              .width('100%').height(44)
              .backgroundColor(Theme.accent).fontColor('#f7faf7')
              .borderRadius(4).enabled(!this.busy)
              .onClick(() => this.saveManual())
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
        .constraintSize({ maxWidth: Theme.contentMaxWidth(this.bp) })
        .padding({
          left: Theme.pagePadding(this.bp),
          right: Theme.pagePadding(this.bp),
          top: 48,
          bottom: 48
        })
      }
      .width('100%').height('100%')
      .scrollBar(BarState.Auto)
      .align(Alignment.Top)
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
