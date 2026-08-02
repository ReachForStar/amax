// WebView 登录页：加载官网登录页，捕获登录后到 /dashboard 的重定向，提取完整 Cookie 串保存
// 偏差记录（docs 复核）：
// 1. 路由使用 this.getUIContext().getRouter() 实例而非静态 router：静态接口自 API 18 废弃（当前 targetSdk API 23），
//    且官方 FAQ 提示在异步回调中调用全局 router 会因追溯不到 UI 上下文而崩溃。
// 2. fetchCookieSync 传入 onLoadIntercept 捕获的完整 URL 而非站点根：文档注明"需传入完整链接"才能取到可用 Cookie。
import { webview } from '@kit.ArkWeb';
import { Theme } from '../common/Theme';
import { parseSessionCookie } from '../common/Format';
import { saveCookie } from '../model/Store';

const LOGIN_URL = 'https://ai.amaxsmp.com/login';
const DASHBOARD_PREFIX = 'https://ai.amaxsmp.com/dashboard';

@Entry
@Component
struct LoginPage {
  private controller: webview.WebviewController = new webview.WebviewController();
  @State notice: string = '';
  // onLoadIntercept 对主框架与 iframe 导航均会触发，标记防重入
  private handled: boolean = false;

  async onLoginSuccess(url: string): Promise<void> {
    try {
      const cookieStr = webview.WebCookieManager.fetchCookieSync(url);
      const session = parseSessionCookie(cookieStr);
      if (session === null) {
        // 已重定向到 dashboard 但未提取到 session：视为登录未完成，放行重试
        this.notice = '未能提取登录凭据，请重试或返回手动粘贴';
        this.handled = false;
        return;
      }
      // 存储完整 cookie 串（请求头需要完整串）；parseSessionCookie 仅用于判定登录成功
      await saveCookie(cookieStr, 'webview');
      const nav = this.getUIContext().getRouter();
      nav.clear();
      await nav.replaceUrl({ url: 'pages/DashboardPage' });
    } catch (error) {
      // Cookie 读取失败、存储失败或导航失败：提示并重置标记，允许重试
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`登录处理失败：${reason}`);
      this.notice = '登录处理失败，请重试或返回手动粘贴';
      this.handled = false;
    }
  }

  build() {
    Column() {
      Row() {
        Text('← 返回').fontSize(15).fontColor(Theme.muted)
          .onClick(() => this.getUIContext().getRouter().back())
          .padding({ right: 12 })
        Text('登录 AMAX 账户').fontSize(15).fontWeight(FontWeight.Medium).fontColor(Theme.ink)
        Blank()
      }
      .width('100%')
      .padding({ left: 16, right: 16, top: 12, bottom: 12 })

      if (this.notice.length > 0) {
        Text(this.notice)
          .fontSize(12).fontColor(Theme.error)
          .backgroundColor(Theme.errorBg)
          .width('100%').padding(10)
      }

      Web({ src: LOGIN_URL, controller: this.controller })
        .width('100%')
        .layoutWeight(1)
        .onLoadIntercept((event) => {
          // 每次 URL 加载前触发（含 iframe 导航）；仅主框架到 /dashboard 的重定向视为登录成功
          const url = event.data.getRequestUrl();
          if (!this.handled && event.data.isMainFrame() && url.startsWith(DASHBOARD_PREFIX)) {
            this.handled = true;
            this.onLoginSuccess(url);
          }
          return false; // 放行导航
        })
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
