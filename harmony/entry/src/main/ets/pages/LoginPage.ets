// WebView 登录页：加载官网登录页，通过 session Cookie 出现判定登录成功，提取完整 Cookie 串保存
// 偏差记录（docs 复核）：
// 1. 路由使用 this.getUIContext().getRouter() 实例而非静态 router：静态接口自 API 18 废弃（当前 targetSdk API 23），
//    且官方 FAQ 提示在异步回调中调用全局 router 会因追溯不到 UI 上下文而崩溃。
// 2. fetchCookieSync 传入站点 URL：文档注明"需传入完整链接"才能取到可用 Cookie。
// 3. 自定义 UA 在 onControllerAttached 中经 controller.setCustomUserAgent 设置：组件属性 .userAgent() 自 API 10 废弃；
//    文档明确 src 已设置 URL 时必须在 onControllerAttached 设置（onLoadIntercept/onPageEnd 中设置会概率性失败或触发重载）。
//    官网 Next.js 对 ArkWeb 默认 UA（含 OpenHarmony/ArkWeb 标识）走错代码路径抛客户端异常，故伪装为桌面 Chrome。
// 4. metaViewport(true)：UA 不含 "Mobile" 时 ArkWeb 默认关闭页面 viewport meta，显式开启保证登录表单按响应式宽度渲染。
// 5. 登录成功判据以 session Cookie 出现为准（轮询 + URL 快路径双通道）：实测官网登录成功后不发生
//    /dashboard 重定向（Next.js 客户端路由或停留原页），仅监听 URL 跳转无法捕获。
// 6. 有效期经 WebCookieManager.fetchAllCookies(false) 读取：fetchCookieSync 只返回 name=value 串，看不到 Expires，
//    这条 @since 23 的接口是手机端唯一能读到 session Cookie 到期属性的通道；取不到一律按"官网未下发"处理。
import { webview } from '@kit.ArkWeb';
import { Theme } from '../common/Theme';
import { parseCookieExpiry, parseSessionCookie } from '../common/Format';
import { saveCookie } from '../model/Store';

const LOGIN_URL = 'https://ai.amaxsmp.com/login';
const SITE_URL = 'https://ai.amaxsmp.com';
const DASHBOARD_PREFIX = 'https://ai.amaxsmp.com/dashboard';
// 与桌面用户一致的 Chrome UA，规避官网对 OpenHarmony/ArkWeb UA 的异常分支
const DESKTOP_CHROME_UA =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';
const LOGIN_POLL_INTERVAL_MS = 800;
// 登录等待上限：与桌面版 login.rs 的 LOGIN_TIMEOUT 同口径，超时关页并把提示交给配置页
const LOGIN_TIMEOUT_MS = 10 * 60 * 1000;
const LOGIN_TIMEOUT_NOTICE = '登录超时，请重新点击「前往登录（推荐）」或改为手动粘贴';
// session Cookie 名与站点域：fetchAllCookies 返回全部域名的 Cookie，需自行按名+域筛选
const SESSION_COOKIE = 'session';
const SITE_DOMAIN = 'amaxsmp.com';

@Entry
@Component
struct LoginPage {
  private controller: webview.WebviewController = new webview.WebviewController();
  @State notice: string = '';
  // 登录成功处理中标志：轮询与 URL 快路径共用，防重入
  private handled: boolean = false;
  private pollTimer: number = -1;
  private timeoutTimer: number = -1;

  aboutToAppear(): void {
    this.startLoginTimeout();
  }

  aboutToDisappear(): void {
    this.stopPolling();
    this.stopLoginTimeout();
  }

  private stopPolling(): void {
    if (this.pollTimer !== -1) {
      clearInterval(this.pollTimer);
      this.pollTimer = -1;
    }
  }

  private startLoginTimeout(): void {
    if (this.timeoutTimer !== -1) {
      return;
    }
    this.timeoutTimer = setTimeout(() => {
      this.onLoginTimeout();
    }, LOGIN_TIMEOUT_MS);
  }

  private stopLoginTimeout(): void {
    if (this.timeoutTimer !== -1) {
      clearTimeout(this.timeoutTimer);
      this.timeoutTimer = -1;
    }
  }

  // 超时后本页即将弹出、无处展示提示：经 AppStorage 'authNotice' 交给配置页消费
  private onLoginTimeout(): void {
    this.timeoutTimer = -1;
    if (this.handled) {
      return;
    }
    this.handled = true;
    this.stopPolling();
    AppStorage.setOrCreate('authNotice', LOGIN_TIMEOUT_NOTICE);
    try {
      this.getUIContext().getRouter().back();
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`登录超时后返回失败：${reason}`);
      this.notice = LOGIN_TIMEOUT_NOTICE;
    }
  }

  private startLoginPolling(): void {
    if (this.pollTimer !== -1) {
      return;
    }
    this.pollTimer = setInterval(() => {
      this.checkLoginByCookie();
    }, LOGIN_POLL_INTERVAL_MS);
  }

  // 轮询判据：官网写入 session Cookie 即视为登录成功（不依赖页面跳转）
  private async checkLoginByCookie(): Promise<void> {
    if (this.handled) {
      return;
    }
    try {
      const cookieStr = webview.WebCookieManager.fetchCookieSync(SITE_URL);
      if (parseSessionCookie(cookieStr) !== null) {
        this.handled = true;
        this.stopPolling();
        await this.onLoginSuccess(cookieStr);
      }
    } catch (error) {
      // controller 未就绪等瞬态错误：忽略，下一轮轮询重试
    }
  }

  // session Cookie 到期时间（本地时区 RFC3339）：会话 Cookie、-1、接口异常均返回 null，
  // 不阻断登录主流程——有效期仅用于展示，拦截一律以服务端 401/403 为准
  private async readSessionExpiry(): Promise<string | null> {
    try {
      const cookies = await webview.WebCookieManager.fetchAllCookies(false);
      const session = cookies.find((cookie: webview.WebHttpCookie) =>
        cookie.name === SESSION_COOKIE && cookie.domain.endsWith(SITE_DOMAIN));
      if (!session) {
        return null;
      }
      return parseCookieExpiry(session.expiresDate, session.isSessionCookie, new Date());
    } catch (error) {
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`读取 Cookie 有效期失败，按未下发处理：${reason}`);
      return null;
    }
  }

  async onLoginSuccess(cookieStr: string): Promise<void> {
    try {
      this.stopLoginTimeout();
      const expiresAt = await this.readSessionExpiry();
      // 存储完整 cookie 串（请求头需要完整串）；parseSessionCookie 仅用于判定登录成功
      await saveCookie(cookieStr, 'webview', expiresAt);
      const nav = this.getUIContext().getRouter();
      nav.clear();
      await nav.replaceUrl({ url: 'pages/DashboardPage' });
    } catch (error) {
      // 存储失败或导航失败：提示并重置标记，允许重试
      const reason = error instanceof Error ? error.message : JSON.stringify(error);
      console.error(`登录处理失败：${reason}`);
      this.notice = '登录处理失败，请重试或返回手动粘贴';
      this.handled = false;
      this.startLoginPolling();
      this.startLoginTimeout();
    }
  }

  build() {
    Column() {
      Row() {
        Text('← 返回').fontSize(15).fontColor(Theme.muted)
          .onClick(() => {
            this.stopPolling();
            this.getUIContext().getRouter().back();
          })
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
        // 桌面 UA 不含 "Mobile"，ArkWeb 会默认关闭页面 viewport meta，显式开启避免登录页按 980px 缩放渲染
        .metaViewport(true)
        // ArkWeb 默认禁用 DOM storage，官网 Next.js 使用 localStorage 会抛
        // "Cannot read properties of null (reading 'getItem')"，显式开启
        .domStorageAccess(true)
        .onControllerAttached(() => {
          // controller 绑定完成后、首次加载前设置 UA；此处设置才能保证首请求即携带自定义 UA
          try {
            this.controller.setCustomUserAgent(DESKTOP_CHROME_UA);
          } catch (error) {
            // 设置失败不阻断登录流程，仅记录日志（后续可用 console 输出进一步定位）
            const reason = error instanceof Error ? error.message : JSON.stringify(error);
            console.error(`LoginPage 设置自定义 UA 失败：${reason}`);
          }
          this.startLoginPolling();
        })
        .onConsole((event) => {
          // 捕获网页 JS console 输出转发到应用 hilog，便于 devecocli log 按 "LoginPage console" 过滤诊断官网异常
          const message = event.message;
          const text = message.getMessage();
          const level = message.getMessageLevel();
          if (level === MessageLevel.Error || level === MessageLevel.Warn) {
            console.error(`LoginPage console: ${text}`);
          } else {
            console.info(`LoginPage console: ${text}`);
          }
          // 返回 false：消息仍照常打印到 hilog，上面的转发仅为统一过滤关键字
          return false;
        })
        .onLoadIntercept((event) => {
          // 快路径：主框架重定向到 /dashboard 立即处理（部分环境官网仍会跳转）；
          // 未跳转的环境由 session Cookie 轮询兜底
          const url = event.data.getRequestUrl();
          if (!this.handled && event.data.isMainFrame() && url.startsWith(DASHBOARD_PREFIX)) {
            this.handled = true;
            this.stopPolling();
            try {
              const cookieStr = webview.WebCookieManager.fetchCookieSync(url);
              this.onLoginSuccess(cookieStr);
            } catch (error) {
              this.handled = false;
              this.startLoginPolling();
            }
          }
          return false; // 放行导航
        })
    }
    .width('100%').height('100%')
    .backgroundColor(Theme.surface)
  }
}
