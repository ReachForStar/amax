# AMAX Dashboard A2 平衡科技页面优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将现有 AMAX Dashboard 配置页与看板页改造成浅色冷灰绿、编辑式信息层级与精密开发工具气质并存的 A2 平衡科技界面。

**Architecture:** 保留无框架静态前端与全部 Tauri IPC 契约，只重构 `dist/index.html` 的语义结构、`dist/style.css` 的视觉系统，以及 `dist/app.js` 的展示文案和 DOM 输出。HTML、CSS、JS 作为一个聚合任务同步修改，避免选择器和状态短暂失配；第二个任务只负责真实 Tauri 应用验收和必要修正。

**Tech Stack:** Tauri 2、原生 HTML、CSS、JavaScript、Rust/Cargo、Windows WebView2

## Global Constraints

- 页面基底使用浅色冷灰绿，正文使用近黑冷绿色，森林绿是唯一常规强调色。
- 不使用渐变、玻璃拟态、霓虹发光、多彩阴影、功能性 emoji 或通用等权卡片阵列。
- 金额、Token、百分比和遥测标签使用等宽字体栈或 `font-variant-numeric: tabular-nums`。
- 不加载外部字体、图片、前端库或新增构建步骤。
- 不修改 Rust 后端、数据库、Tauri `520×680` 窗口尺寸、IPC command、`dashboard-updated` 事件或数据字段。
- 今日费用保留 6 位小数；Token 格式化、额度比例和旧数据保留行为保持不变。
- API Key 文案必须为“兼容配置，当前账户汇总无需填写”。
- Cookie 本地过期提示必须为“Cookie 已过期（超过 15 天），请重新获取”。
- 所有按钮和输入框必须有 `focus-visible`；图标按钮点击目标不小于 36×36px。
- 动画只改变 `transform` 和 `opacity`，进度条宽度更新除外；必须支持 `prefers-reduced-motion`。
- 内容区允许垂直滚动，不允许横向滚动；窄宽度下 Token 双列转为单列。

---

## 文件结构

- Modify: `dist/index.html`——配置页、看板页、骨架屏和内联 SVG 图标的语义结构。
- Modify: `dist/style.css`——A2 设计变量、精密网格、开放式数据布局、状态、焦点、响应式和 reduced motion。
- Modify: `dist/app.js`——更新文案、用户元信息、同步状态和额度文本，不改变 IPC 与刷新流程。
- 不创建新运行时文件；三个现有文件共同构成完整前端，拆分会引入无必要加载与构建复杂度。

### Task 1: 同步重构配置页、看板页与展示逻辑

**Files:**
- Modify: `dist/index.html:1-110`
- Modify: `dist/style.css:1-223`
- Modify: `dist/app.js:18-243`

**Interfaces:**
- Consumes: Tauri commands `get_config`、`save_config`、`fetch_dashboard`；事件 `dashboard-updated`；现有 `DashboardData` 字段。
- Produces: 保持现有 DOM IDs：`config-screen`、`dashboard-screen`、`config-status`、`cookie-input`、`apikey-input`、`save-btn`、`last-updated`、`refresh-btn`、`settings-btn`、`skeleton`、`error-msg`、`dashboard-data`、`user-name`、`user-requests`、`today-yuan`、`log-count`、`today-tokens`、`token-detail`、`quota-percent`、`progress-fill`、`quota-remaining`、`quota-total`、`quota-used`。

- [ ] **Step 1: 记录改造前静态检查基线**

Run:

```bash
node --check dist/app.js
cargo test --workspace
```

Expected: 两条命令退出码为 0；若基线失败，先停止并报告，不在视觉改造中掩盖既有故障。

- [ ] **Step 2: 用语义结构替换配置页与看板页**

将 `dist/index.html` 的 `<body>` 内容替换为以下结构，保留 head 中现有 meta、title、stylesheet：

```html
<body>
  <section id="config-screen" class="screen config-screen" aria-labelledby="config-title">
    <main class="config-shell">
      <header class="access-header">
        <p class="product-mark">AMAX / ACCESS</p>
        <h1 id="config-title">连接你的 AMAX 账户</h1>
        <p class="access-intro">凭据只保存在当前 Windows 用户的本地应用数据中。</p>
      </header>

      <form class="config-form" novalidate>
        <div class="form-group">
          <label for="cookie-input">Session Cookie</label>
          <input type="password" id="cookie-input" placeholder="session=MTc4MzQyOTkyN3xE..." autocomplete="off" />
          <ol class="access-steps">
            <li>登录 <a href="https://ai.amaxsmp.com/dashboard" target="_blank" rel="noreferrer">AMAX Dashboard</a></li>
            <li>按 F12，打开 Application → Cookies</li>
            <li>找到 session，复制完整 Value</li>
          </ol>
        </div>

        <div class="form-group optional-field">
          <div class="label-row">
            <label for="apikey-input">API Key</label>
            <span>可选</span>
          </div>
          <input type="password" id="apikey-input" placeholder="sk-..." autocomplete="off" />
          <p class="hint">兼容配置，当前账户汇总无需填写</p>
        </div>

        <div id="config-status" class="status hidden" role="status" aria-live="polite"></div>
        <button id="save-btn" type="button" class="btn primary">保存并验证</button>
      </form>
    </main>
  </section>

  <section id="dashboard-screen" class="screen hidden" aria-labelledby="dashboard-title">
    <header class="topbar">
      <p id="dashboard-title" class="product-mark">AMAX / DAILY</p>
      <div class="topbar-actions">
        <span class="sync-state"><span class="sync-dot" aria-hidden="true"></span><span id="last-updated">等待同步</span></span>
        <button id="refresh-btn" class="btn icon" title="刷新数据" aria-label="刷新数据">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11a8 8 0 1 0-2.34 5.66M20 4v7h-7" /></svg>
        </button>
        <button id="settings-btn" class="btn icon" title="设置" aria-label="设置">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" /><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6l-.05.08H9.95L9.9 20a1.7 1.7 0 0 0-1-.6 1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-.6-1l-.08-.05V9.95L4 9.9a1.7 1.7 0 0 0 .6-1 1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-.6l.05-.08h3.99l.05.08a1.7 1.7 0 0 0 1 .6 1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.4 9c.08.38.3.74.6 1l.08.05v3.99L20 14c-.3.26-.52.62-.6 1Z" /></svg>
        </button>
      </div>
    </header>

    <main class="content">
      <div id="error-msg" class="error-strip hidden" role="alert"></div>

      <div id="skeleton" class="skeleton hidden" aria-label="正在加载数据">
        <div class="sk-line sk-user"></div><div class="sk-line sk-meta"></div>
        <div class="sk-line sk-money"></div>
        <div class="sk-metrics"><div class="sk-line"></div><div class="sk-line"></div></div>
        <div class="sk-quota"><div class="sk-line"></div><div class="sk-bar"></div></div>
      </div>

      <article id="dashboard-data" class="dashboard-data hidden">
        <header class="account-header">
          <h1 id="user-name"></h1>
          <p id="user-requests"></p>
        </header>

        <section class="cost-block" aria-labelledby="cost-label">
          <p id="cost-label" class="telemetry-label">COST TODAY</p>
          <p id="today-yuan" class="primary-value"></p>
          <p id="log-count" class="metric-note"></p>
        </section>

        <section class="token-grid" aria-label="Token 用量">
          <div class="token-metric">
            <p class="telemetry-label">TOKENS</p>
            <p id="today-tokens" class="secondary-value"></p>
          </div>
          <div class="token-metric token-io">
            <p class="telemetry-label">INPUT / OUTPUT</p>
            <p id="token-detail" class="secondary-value"></p>
          </div>
        </section>

        <section class="quota-block" aria-labelledby="quota-label">
          <div class="quota-heading">
            <div><p id="quota-label" class="telemetry-label">AVAILABLE QUOTA</p><p id="quota-percent" class="quota-percent"></p></div>
            <p id="quota-remaining" class="quota-remaining"></p>
          </div>
          <div class="progress-bar" role="progressbar" aria-label="剩余额度比例" aria-valuemin="0" aria-valuemax="100"><div id="progress-fill" class="progress-fill"></div></div>
          <div class="quota-detail"><span id="quota-used"></span><span id="quota-total"></span></div>
        </section>
      </article>
    </main>
  </section>
  <script src="app.js"></script>
</body>
```

- [ ] **Step 3: 实现 A2 视觉系统和完整状态样式**

将 `dist/style.css` 替换为以下视觉规则；可以按格式化需要换行，但不得改变颜色、布局和状态约束：

```css
:root {
  --surface: #e8ece8;
  --surface-raised: #f2f4f1;
  --surface-pressed: #dce3dd;
  --ink: #19221e;
  --muted: #627069;
  --faint: #8a9690;
  --line: #bec9c1;
  --accent: #2c6b4b;
  --accent-soft: #d2e2d7;
  --error: #8b3e3e;
  --error-bg: #eadada;
  --mono: "Cascadia Mono", "SFMono-Regular", Consolas, monospace;
  --sans: "Segoe UI Variable", "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
}

*, *::before, *::after { box-sizing: border-box; }
html, body { width: 100%; min-height: 100%; margin: 0; }
body { overflow: hidden; background: var(--surface); color: var(--ink); font-family: var(--sans); font-size: 14px; }
button, input { font: inherit; }
a { color: var(--accent); text-underline-offset: 3px; }
.hidden { display: none !important; }
.screen { position: absolute; inset: 0; overflow-x: hidden; overflow-y: auto; background-color: var(--surface); background-image: linear-gradient(rgba(44, 107, 75, .045) 1px, transparent 1px), linear-gradient(90deg, rgba(44, 107, 75, .035) 1px, transparent 1px); background-size: 24px 24px; }
.product-mark, .telemetry-label { margin: 0; font-family: var(--mono); font-size: 10px; font-weight: 700; letter-spacing: .14em; }
.product-mark { color: var(--accent); }
.telemetry-label { color: var(--muted); }

.config-screen { padding: 52px 44px 60px; }
.config-shell { width: 100%; max-width: 432px; margin: 0 auto; }
.access-header { padding-bottom: 28px; border-bottom: 1px solid var(--line); }
.access-header h1 { margin: 18px 0 9px; font-size: 30px; line-height: 1.15; letter-spacing: -.04em; }
.access-intro { max-width: 34em; margin: 0; color: var(--muted); line-height: 1.65; }
.config-form { display: grid; gap: 25px; padding-top: 27px; }
.form-group { display: grid; gap: 9px; }
.form-group label { font-weight: 650; }
.label-row { display: flex; justify-content: space-between; align-items: baseline; }
.label-row span { color: var(--faint); font: 10px var(--mono); letter-spacing: .08em; }
.form-group input { width: 100%; min-height: 44px; border: 1px solid var(--line); border-radius: 4px; outline: 0; background: rgba(242, 244, 241, .88); color: var(--ink); padding: 10px 12px; font-family: var(--mono); font-size: 12px; transition: border-color .18s, box-shadow .18s, background-color .18s; }
.form-group input:hover { border-color: #91a298; }
.form-group input:focus-visible { border-color: var(--accent); box-shadow: 0 0 0 3px rgba(44, 107, 75, .16); background: var(--surface-raised); }
.access-steps { margin: 2px 0 0; padding-left: 22px; color: var(--muted); font-size: 12px; line-height: 1.75; }
.hint { margin: 0; color: var(--muted); font-size: 12px; }
.status { border-left: 2px solid var(--accent); padding: 9px 11px; background: var(--accent-soft); color: var(--accent); font-size: 12px; line-height: 1.55; white-space: pre-line; }
.status.error { border-color: var(--error); background: var(--error-bg); color: var(--error); }
.status.loading { color: var(--accent); }
.btn { border: 0; border-radius: 4px; cursor: pointer; transition: transform .16s, background-color .16s, color .16s, opacity .16s; }
.btn:active:not(:disabled) { transform: translateY(1px) scale(.99); }
.btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
.btn:disabled { cursor: wait; opacity: .5; }
.btn.primary { min-height: 44px; background: var(--accent); color: #f7faf7; font-weight: 650; }
.btn.primary:hover:not(:disabled) { background: #245a3f; }

.topbar { position: sticky; top: 0; z-index: 2; display: flex; justify-content: space-between; align-items: center; min-height: 58px; padding: 0 22px; border-bottom: 1px solid var(--line); background: rgba(232, 236, 232, .94); }
.topbar-actions, .sync-state { display: flex; align-items: center; }
.topbar-actions { gap: 5px; }
.sync-state { gap: 7px; margin-right: 5px; color: var(--muted); font: 10px var(--mono); white-space: nowrap; }
.sync-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--accent); box-shadow: 0 0 0 3px rgba(44, 107, 75, .12); }
.btn.icon { display: grid; place-items: center; width: 36px; height: 36px; padding: 0; background: transparent; color: var(--muted); }
.btn.icon:hover:not(:disabled) { background: var(--surface-pressed); color: var(--ink); }
.btn.icon svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
.btn.icon.is-loading svg { animation: spin .75s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
.content { min-height: calc(100% - 58px); padding: 30px 34px 46px; }
.error-strip { margin-bottom: 22px; border-left: 2px solid var(--error); background: var(--error-bg); color: var(--error); padding: 9px 11px; font-size: 12px; line-height: 1.5; }
.dashboard-data { max-width: 452px; margin: 0 auto; }
.account-header h1 { margin: 0; font-size: 20px; letter-spacing: -.025em; }
.account-header p { margin: 7px 0 0; color: var(--muted); font: 10px var(--mono); }
.cost-block { padding: 43px 0 32px; }
.primary-value { margin: 9px 0 0; font-family: var(--mono); font-size: clamp(35px, 9vw, 47px); font-weight: 600; line-height: 1.05; letter-spacing: -.065em; font-variant-numeric: tabular-nums; }
.metric-note { margin: 9px 0 0; color: var(--muted); font-size: 11px; }
.token-grid { display: grid; grid-template-columns: .9fr 1.35fr; border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); }
.token-metric { min-width: 0; padding: 18px 0; }
.token-io { border-left: 1px solid var(--line); padding-left: 20px; }
.secondary-value { margin: 8px 0 0; overflow-wrap: anywhere; font-family: var(--mono); font-size: 18px; font-weight: 600; font-variant-numeric: tabular-nums; }
.quota-block { margin-top: 34px; border-left: 2px solid var(--accent); padding: 3px 0 4px 17px; }
.quota-heading { display: flex; justify-content: space-between; align-items: end; gap: 16px; }
.quota-percent { margin: 6px 0 0; font-family: var(--mono); font-size: 30px; font-weight: 600; letter-spacing: -.05em; font-variant-numeric: tabular-nums; }
.quota-remaining { margin: 0; color: var(--accent); font: 11px var(--mono); text-align: right; }
.progress-bar { height: 4px; margin-top: 14px; overflow: hidden; background: #cbd4cd; }
.progress-fill { width: 0; height: 100%; background: var(--accent); transition: width .4s ease; }
.quota-detail { display: flex; justify-content: space-between; gap: 15px; margin-top: 9px; color: var(--muted); font: 9px var(--mono); }
.skeleton { max-width: 452px; margin: 0 auto; }
.sk-line, .sk-bar { background: var(--surface-pressed); animation: pulse 1.4s ease-in-out infinite; }
.sk-line { height: 12px; }
.sk-user { width: 42%; height: 20px; }
.sk-meta { width: 34%; margin-top: 9px; }
.sk-money { width: 69%; height: 45px; margin-top: 48px; }
.sk-metrics { display: grid; grid-template-columns: .9fr 1.35fr; gap: 20px; margin-top: 42px; padding: 20px 0; border-block: 1px solid var(--line); }
.sk-quota { margin-top: 34px; border-left: 2px solid var(--line); padding: 4px 0 4px 17px; }
.sk-quota .sk-line { width: 32%; height: 29px; }
.sk-bar { height: 4px; margin-top: 15px; }
@keyframes pulse { 50% { opacity: .45; } }
::-webkit-scrollbar { width: 7px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: #aebbb2; border-radius: 4px; }

@media (max-width: 420px) {
  .config-screen { padding: 38px 24px 48px; }
  .content { padding-inline: 24px; }
  .sync-state { display: none; }
  .token-grid { grid-template-columns: 1fr; }
  .token-io { border-left: 0; border-top: 1px solid var(--line); padding-left: 0; }
}
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after { scroll-behavior: auto !important; animation-duration: .01ms !important; animation-iteration-count: 1 !important; transition-duration: .01ms !important; }
}
```

- [ ] **Step 4: 适配展示文案与无障碍状态，不改变数据流**

在 `dist/app.js` 中进行以下精确修改：

1. `updateLastUpdated()` 保持相对时间，同时让顶栏状态初始值被更新。
2. `renderDashboard(d)` 替换为：

```javascript
function renderDashboard(d) {
  hasDashboardData = true;
  skeleton.classList.add('hidden');
  dashboardData.classList.remove('hidden');
  errorEl.classList.add('hidden');

  $('#user-name').textContent = d.display_name + ' 的今日用量';
  $('#user-requests').textContent = d.request_count.toLocaleString() + ' REQUESTS · 数据已同步';
  $('#today-yuan').textContent = d.logs_available ? '¥' + d.today_yuan.toFixed(6) : '暂不可用';
  $('#log-count').textContent = d.logs_available
    ? (d.log_count == null ? '成功请求汇总' : d.log_count + ' 次请求')
    : '日志接口暂不可用';
  $('#today-tokens').textContent = d.logs_available ? formatTokens(d.today_tokens) : '暂不可用';
  $('#token-detail').textContent = d.logs_available
    ? formatTokens(d.today_input) + ' / ' + formatTokens(d.today_output)
    : '额度信息仍可正常查看';
  $('#quota-percent').textContent = d.percent.toFixed(1) + '%';
  const percent = Math.min(Math.max(d.percent, 0), 100);
  $('#progress-fill').style.width = percent.toFixed(1) + '%';
  $('.progress-bar').setAttribute('aria-valuenow', percent.toFixed(1));
  $('#quota-remaining').textContent = '剩余 ¥' + d.remaining.toFixed(2);
  $('#quota-total').textContent = 'TOTAL ¥' + d.total.toFixed(2);
  $('#quota-used').textContent = 'USED ¥' + d.used.toFixed(2);
  markUpdated();
}
```

3. 将 Cookie 过期初始化文案改为：

```javascript
showConfigError('Cookie 已过期（超过 15 天），请重新获取');
```

4. 保留 `loadDashboard()`、`dashboardRequest`、`setupEventListener()`、IPC command 名称和异常分支，不重写业务流程。

- [ ] **Step 5: 运行静态检查并检查语义约束**

Run:

```bash
node --check dist/app.js
node -e "const fs=require('fs');const h=fs.readFileSync('dist/index.html','utf8');for(const id of ['config-screen','dashboard-screen','config-status','cookie-input','apikey-input','save-btn','last-updated','refresh-btn','settings-btn','skeleton','error-msg','dashboard-data','user-name','user-requests','today-yuan','log-count','today-tokens','token-detail','quota-percent','progress-fill','quota-remaining','quota-total','quota-used'])if(!h.includes('id=\"'+id+'\"'))throw Error('missing '+id);if(/[💾👋💰📊💳🟢⚙]/u.test(h))throw Error('functional emoji remains');"
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Expected: 所有命令退出码为 0；24 个既有 DOM ID 全部保留；HTML 不含旧功能性 emoji。

- [ ] **Step 6: 审查范围和精确差异**

Run:

```bash
git diff --check
git diff --stat
git status --short
```

Expected: 产品差异仅包含 `dist/index.html`、`dist/style.css`、`dist/app.js`；`.claude/` 与 `.superpowers/` 不得暂存。

- [ ] **Step 7: 进行前端、可访问性和通用代码审查**

依次调用：

- `ecc:a11y-architect`：检查语义、键盘焦点、ARIA、对比度和 reduced motion。
- `ecc:code-reviewer`：检查 DOM 选择器、状态切换、错误降级和无关行为变化。

Critical/Important 问题必须修复后重新执行 Step 5；Minor 问题记录给最终审查。

- [ ] **Step 8: 提交聚合前端改造**

Run:

```bash
git add dist/index.html dist/style.css dist/app.js
git commit -m "自动提交：重塑 A2 平衡科技页面"
```

Expected: 提交只包含三个 `dist/` 文件；暂不 push，等待真实应用验收。

### Task 2: 真实 Tauri 应用验收与交付

**Files:**
- Verify: `dist/index.html`
- Verify: `dist/style.css`
- Verify: `dist/app.js`
- Modify only if verification exposes a defect: the affected file among the three above

**Interfaces:**
- Consumes: Task 1 的静态前端提交、Tauri `520×680` 主窗口及 `.claude/skills/verify/SKILL.md` 流程（若该项目技能文件缺失，则使用 `cargo tauri dev` 和浏览器开发工具手工驱动）。
- Produces: 在真实 WebView2 中通过配置页、看板页、状态和窗口尺寸验收的可交付提交。

- [ ] **Step 1: 启动真实应用**

Run:

```bash
cargo tauri dev
```

Expected: 应用成功编译并打开 520×680 主窗口。若 `target/debug/amax.exe` 被已有进程占用，停止并请用户关闭应用，不得用 `cargo check` 替代。

- [ ] **Step 2: 验证配置页**

在真实窗口中检查：

- `AMAX / ACCESS`、标题、认证说明和编号 Cookie 步骤均可见。
- 页面为浅色冷灰绿，极淡网格不影响阅读。
- API Key 文案准确显示“兼容配置，当前账户汇总无需填写”。
- 520×680 下无横向滚动；缩短窗口高度时可垂直滚动到底部按钮。
- Tab 顺序依次进入 Cookie、API Key、保存按钮；每项有清晰焦点环。
- 输入短 Cookie 后显示内联错误，不弹窗。

Expected: 所有检查通过并记录截图或可复核观察结果。

- [ ] **Step 3: 验证看板与状态**

若本机已有有效 Cookie：

- 保存或启动后进入看板。
- 主金额保留 6 位小数，Token 和输入/输出格式正确。
- 额度百分比、进度、剩余、已使用和总额与数据一致。
- 刷新按钮加载时旋转且禁用；设置按钮返回配置页。
- 后台事件仍更新数据；网络失败时旧数据保留并显示紧凑错误条。
- 日志接口不可用只降级费用和 Token，额度区仍显示。

若本机没有有效 Cookie：至少通过开发工具或现有可离线入口验证骨架屏、配置错误、焦点、滚动和窗口生命周期，并明确记录无法验证的凭据依赖项。

- [ ] **Step 4: 验证 reduced motion 与窄宽度**

使用 WebView2 开发工具模拟 `prefers-reduced-motion: reduce` 和小于 420px 的可用宽度。

Expected: 非必要动画近似禁用；Token 区转为单列；无横向滚动、遮挡或文本溢出。

- [ ] **Step 5: 修复真实应用中发现的问题并回归**

仅当 Step 2–4 暴露缺陷时修改对应 `dist/` 文件。每次修复后运行：

```bash
node --check dist/app.js
cargo test --workspace
```

Expected: 静态检查通过，并在真实应用中重新驱动失败场景确认已修复。

- [ ] **Step 6: 运行最终完整验证**

Run:

```bash
node --check dist/app.js
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check HEAD~1..HEAD
git status --short
```

Expected: 所有检查退出码为 0；工作区除未跟踪的本地 `.claude/`、`.superpowers/` 外无产品改动。如果 Step 5 有修复，先按精确文件 `git add` 并创建提交 `自动提交：修正 A2 页面验收问题`，再重复本步骤。

- [ ] **Step 7: 最终代码审查**

调用 `ecc:code-reviewer` 审查 Task 1 起始提交至当前 HEAD 的完整前端差异，重点检查：

- IPC、事件、DOM ID 和错误降级兼容性。
- A2 视觉约束与去 AI 味目标。
- 键盘、ARIA、对比度、滚动和 reduced motion。
- 真实应用验收报告是否覆盖受影响流程。

修复所有 Critical/Important 后重复 Step 6 和相关真实应用场景。

- [ ] **Step 8: 推送交付提交**

Run:

```bash
git push
```

Expected: 当前分支全部 A2 页面提交成功推送；不提交 `.claude/` 或 `.superpowers/`。
