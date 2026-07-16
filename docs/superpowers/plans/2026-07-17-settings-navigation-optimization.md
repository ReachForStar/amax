# 设置页返回与关键易用性优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 AMAX Dashboard 增加可安全返回看板的设置页，并修复设置读取、表单提交、焦点、加载状态和页面可访问性中的关键问题。

**Architecture:** 保持现有两页静态前端结构，在 `dist/app.js` 内用少量显式状态和集中入口管理页面切换、配置脏状态与保存忙碌状态。通过 DOM 夹具测试纯前端行为，不修改 Rust IPC、数据库、托盘或刷新协议。

**Tech Stack:** 原生 HTML/CSS/JavaScript、Node.js 内置 `node:test`、Tauri 2、Rust/Cargo。

## Global Constraints

- 不引入前端框架、npm 依赖或构建步骤。
- 只修改 `dist/index.html`、`dist/style.css`、`dist/app.js`，并新增一个前端行为测试文件。
- 保持 `window.__TAURI__.core.invoke`、`window.__TAURI_INTERNALS__.invoke`、`window.__TAURI__.invoke` 三个 IPC 兼容分支。
- 首次启动、Cookie 过期或认证失败时不能返回尚未成功渲染的看板。
- 设置页存在未保存输入时，返回必须使用原生确认框；取消保留输入，确认放弃输入。
- 不改变现有视觉语言、IPC command、事件名称、持久化格式、托盘或定时刷新行为。
- 涉及界面和交互，最终必须运行 `cargo tauri dev` 验证真实 WebView2 应用；若已有 `amax.exe`，请用户关闭，不主动终止进程。
- 每次提交只暂存本任务列出的具体文件，不使用 `git add .`。

---

## File Structure

- `dist/index.html`：配置页返回控件、标准表单提交语义和静态 ARIA 初始值。
- `dist/style.css`：配置页标题布局和次级返回按钮样式，复用现有色彩与按钮反馈。
- `dist/app.js`：页面状态、配置页进入/离开、脏状态确认、保存忙碌状态、统一错误提取、焦点与 ARIA 同步。
- `tests/frontend-navigation.test.js`：用 Node 内置 `node:test` 和最小 DOM/Tauri 夹具验证设置导航及错误分支，无第三方依赖。

### Task 1: 建立前端导航测试夹具

**Files:**
- Create: `tests/frontend-navigation.test.js`
- Test: `tests/frontend-navigation.test.js`

**Interfaces:**
- Consumes: `dist/app.js` 当前通过 `document.querySelector`、DOM 事件、`window.__TAURI__.core.invoke` 和 `window.confirm` 工作。
- Produces: `createHarness(overrides)`，返回 `{ elements, invokeCalls, confirmCalls, documentListeners, focused, flush }`，后续测试用它驱动应用。

- [ ] **Step 1: 创建最小 DOM 与 Tauri 测试夹具**

创建 `tests/frontend-navigation.test.js`：

```js
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

class ClassList {
  constructor(initial = []) { this.values = new Set(initial); }
  add(...names) { names.forEach((name) => this.values.add(name)); }
  remove(...names) { names.forEach((name) => this.values.delete(name)); }
  contains(name) { return this.values.has(name); }
  toggle(name, force) {
    const next = force === undefined ? !this.contains(name) : force;
    if (next) this.add(name); else this.remove(name);
    return next;
  }
}

class Element {
  constructor({ hidden = false, value = '' } = {}) {
    this.classList = new ClassList(hidden ? ['hidden'] : []);
    this.value = value;
    this.placeholder = '';
    this.textContent = '';
    this.disabled = false;
    this.attributes = new Map();
    this.listeners = new Map();
    this.style = {};
  }
  addEventListener(type, handler) { this.listeners.set(type, handler); }
  dispatch(type, event = {}) {
    return this.listeners.get(type)?.({ preventDefault() {}, key: '', ...event });
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  removeAttribute(name) { this.attributes.delete(name); }
  getAttribute(name) { return this.attributes.get(name); }
  focus() { globalThis.__focusedElement = this; }
}

function createHarness({ config, dashboard, confirm = true, configError } = {}) {
  const ids = [
    'config-screen', 'dashboard-screen', 'config-status', 'skeleton', 'error-msg',
    'dashboard-data', 'cookie-input', 'apikey-input', 'save-btn', 'back-btn',
    'config-form', 'refresh-btn', 'settings-btn', 'last-updated', 'user-name',
    'user-requests', 'today-yuan', 'log-count', 'today-tokens', 'token-detail',
    'quota-percent', 'progress-fill', 'quota-remaining', 'quota-total', 'quota-used',
  ];
  const elements = Object.fromEntries(ids.map((id) => [id, new Element()]));
  elements['dashboard-screen'].classList.add('hidden');
  elements['dashboard-data'].classList.add('hidden');
  elements.skeleton.classList.add('hidden');
  elements['error-msg'].classList.add('hidden');
  elements['config-status'].classList.add('hidden');
  elements['back-btn'].classList.add('hidden');
  const progressBar = new Element();
  const documentListeners = new Map();
  const document = {
    body: { innerHTML: '' },
    querySelector(selector) {
      if (selector === '.progress-bar') return progressBar;
      if (selector.startsWith('#')) return elements[selector.slice(1)];
      return null;
    },
    addEventListener(type, handler) { documentListeners.set(type, handler); },
  };
  const invokeCalls = [];
  const invoke = async (command) => {
    invokeCalls.push(command);
    if (command === 'get_config') {
      if (configError) throw configError;
      return config ?? { has_cookie: true, has_api_key: false, expired: false };
    }
    if (command === 'fetch_dashboard') {
      return dashboard ?? {
        display_name: 'Tester', request_count: 3, logs_available: true,
        today_yuan: 1.25, log_count: 2, today_tokens: 1200,
        today_input: 800, today_output: 400, percent: 75,
        remaining: 75, total: 100, used: 25,
      };
    }
    return undefined;
  };
  const confirmCalls = [];
  const window = {
    __TAURI__: { core: { invoke } },
    confirm(message) { confirmCalls.push(message); return confirm; },
    setInterval() { return 1; },
  };
  const context = vm.createContext({ console, Date, document, window });
  context.globalThis.__focusedElement = null;
  const source = fs.readFileSync(path.join(__dirname, '..', 'dist', 'app.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'dist/app.js' });
  return {
    elements,
    invokeCalls,
    confirmCalls,
    documentListeners,
    focused: () => context.globalThis.__focusedElement,
    flush: () => new Promise((resolve) => setImmediate(resolve)),
  };
}

test('已加载看板进入设置后应提供返回入口', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['back-btn'].classList.contains('hidden'), false);
});
```

- [ ] **Step 2: 运行测试并确认失败**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: FAIL，因为返回按钮仍保持隐藏。

- [ ] **Step 3: 仅提交测试夹具**

```bash
git add tests/frontend-navigation.test.js
git commit -m "test: 建立设置导航前端夹具"
git push
```

### Task 2: 增加返回控件与基础页面语义

**Files:**
- Modify: `dist/index.html:9-43`
- Modify: `dist/style.css:27-50`
- Test: `tests/frontend-navigation.test.js`

**Interfaces:**
- Consumes: 测试夹具查询 `#config-form`、`#back-btn`。
- Produces: `#back-btn`、`#config-form`，页面初始 `aria-hidden`，提交型 `#save-btn`。

- [ ] **Step 1: 扩展测试断言静态语义**

在现有测试文件末尾添加：

```js
test('配置表单应使用提交语义并提供可访问的返回按钮', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'dist', 'index.html'), 'utf8');

  assert.match(html, /<form id="config-form"[^>]*>/);
  assert.match(html, /<button id="save-btn" type="submit"/);
  assert.match(html, /<button id="back-btn"[^>]*aria-label="返回看板"/);
  assert.match(html, /id="dashboard-screen"[^>]*aria-hidden="true"/);
});
```

- [ ] **Step 2: 运行测试并确认静态语义失败**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: FAIL，缺少 `config-form` ID、返回按钮、提交类型或初始 `aria-hidden`。

- [ ] **Step 3: 修改配置页结构**

将 `dist/index.html` 的配置页头部和表单起始部分改为：

```html
<section id="config-screen" class="screen config-screen" aria-labelledby="config-title" aria-hidden="false">
  <main class="config-shell">
    <header class="access-header">
      <div class="access-nav">
        <p class="product-mark">AMAX / ACCESS</p>
        <button id="back-btn" type="button" class="btn secondary hidden" aria-label="返回看板">返回看板</button>
      </div>
      <h1 id="config-title">连接你的 AMAX 账户</h1>
      <p class="access-intro">凭据只保存在当前 Windows 用户的本地应用数据中。</p>
    </header>

    <form id="config-form" class="config-form" novalidate aria-busy="false">
```

将保存按钮改为：

```html
<button id="save-btn" type="submit" class="btn primary">保存并验证</button>
```

将看板 section 起始标签改为：

```html
<section id="dashboard-screen" class="screen hidden" aria-labelledby="dashboard-title" aria-hidden="true">
```

- [ ] **Step 4: 增加次级按钮样式**

在 `dist/style.css` 的配置页样式中加入：

```css
.access-nav { display: flex; justify-content: space-between; align-items: center; gap: 16px; }
.btn.secondary { min-height: 34px; padding: 0 11px; border: 1px solid var(--line); background: transparent; color: var(--muted); font-size: 12px; font-weight: 650; }
.btn.secondary:hover:not(:disabled) { border-color: #91a298; background: var(--surface-pressed); color: var(--ink); }
```

- [ ] **Step 5: 运行前端测试**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: 静态语义测试 PASS；行为测试仍 FAIL，因为 JS 尚未显示返回按钮。

- [ ] **Step 6: 提交结构与样式**

```bash
git add dist/index.html dist/style.css tests/frontend-navigation.test.js
git commit -m "feat: 增加设置页返回控件"
git push
```

### Task 3: 集中页面、配置和忙碌状态

**Files:**
- Modify: `dist/app.js:18-123`
- Modify: `tests/frontend-navigation.test.js`

**Interfaces:**
- Consumes: `#config-form`、`#back-btn`、`showScreen(screen)`。
- Produces: `getErrorMessage(error)`、`setConfigBusy(isBusy)`、`resetConfigForm()`、`isConfigDirty()`、`leaveSettings()`。

- [ ] **Step 1: 添加返回与脏状态测试**

在测试文件末尾添加：

```js
test('未修改设置时返回看板不应确认', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  await app.elements['back-btn'].dispatch('click');

  assert.equal(app.confirmCalls.length, 0);
  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['config-screen'].getAttribute('aria-hidden'), 'true');
});

test('取消放弃修改时应保留设置页和输入', async () => {
  const app = createHarness({ confirm: false });
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  app.elements['cookie-input'].value = 'session=changed';
  await app.elements['back-btn'].dispatch('click');

  assert.deepEqual(app.confirmCalls, ['放弃未保存的修改并返回吗？']);
  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['cookie-input'].value, 'session=changed');
});

test('确认放弃修改时应清空输入并返回看板', async () => {
  const app = createHarness({ confirm: true });
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  app.elements['apikey-input'].value = 'sk-changed';
  await app.elements['back-btn'].dispatch('click');

  assert.equal(app.elements['apikey-input'].value, '');
  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
});
```

- [ ] **Step 2: 运行测试并确认失败**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: FAIL，因为返回事件、脏状态与 ARIA 同步尚未实现。

- [ ] **Step 3: 增加 DOM 引用和状态**

在 `dist/app.js` DOM 区加入：

```js
const configForm = $('#config-form');
const backBtn = $('#back-btn');
const settingsBtn = $('#settings-btn');
```

在现有状态变量后加入：

```js
let canReturnToDashboard = false;
let configBaseline = { cookie: '', apiKey: '' };
let isSavingConfig = false;
```

- [ ] **Step 4: 增加统一状态辅助函数**

在工具区加入并替换 `showScreen`：

```js
function getErrorMessage(error) {
  if (typeof error === 'string') return error;
  return error?.message || error?.toString?.() || '未知错误';
}

function showScreen(screen) {
  const showingConfig = screen === 'config';
  configScreen.classList.toggle('hidden', !showingConfig);
  configScreen.setAttribute('aria-hidden', String(!showingConfig));
  dashboardScreen.classList.toggle('hidden', showingConfig);
  dashboardScreen.setAttribute('aria-hidden', String(showingConfig));
}

function setConfigBusy(isBusy) {
  isSavingConfig = isBusy;
  saveBtn.disabled = isBusy;
  backBtn.disabled = isBusy;
  configForm.setAttribute('aria-busy', String(isBusy));
}

function resetConfigForm() {
  cookieInput.value = '';
  apikeyInput.value = '';
  configBaseline = { cookie: '', apiKey: '' };
  configStatus.classList.add('hidden');
}

function isConfigDirty() {
  return cookieInput.value !== configBaseline.cookie
    || apikeyInput.value !== configBaseline.apiKey;
}

function leaveSettings() {
  if (!canReturnToDashboard || isSavingConfig) return;
  if (isConfigDirty() && !window.confirm('放弃未保存的修改并返回吗？')) return;
  resetConfigForm();
  showScreen('dashboard');
  settingsBtn.focus();
}
```

- [ ] **Step 5: 将保存点击事件改为表单提交事件**

把：

```js
saveBtn.addEventListener('click', async () => {
```

改为：

```js
configForm.addEventListener('submit', async (event) => {
  event.preventDefault();
```

把提交前的：

```js
saveBtn.disabled = true;
```

改为：

```js
setConfigBusy(true);
```

把两个失败/认证分支中的 `saveBtn.disabled = false` 改为：

```js
setConfigBusy(false);
```

并把异常提取统一改为：

```js
const msg = getErrorMessage(e);
```

- [ ] **Step 6: 绑定返回按钮**

在按钮区加入：

```js
backBtn.addEventListener('click', leaveSettings);
```

- [ ] **Step 7: 运行测试并确认通过**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: 当前全部测试 PASS。

- [ ] **Step 8: 提交状态管理**

```bash
git add dist/app.js tests/frontend-navigation.test.js
git commit -m "feat: 管理设置返回与未保存修改"
git push
```

### Task 4: 修复设置入口、认证失效与后台更新边界

**Files:**
- Modify: `dist/app.js:125-247`
- Modify: `tests/frontend-navigation.test.js`

**Interfaces:**
- Consumes: `canReturnToDashboard`、`resetConfigForm()`、`setConfigBusy()`、`getErrorMessage()`。
- Produces: `openSettings()`；认证失败清除返回资格；后台事件不授予首次配置页返回资格。

- [ ] **Step 1: 添加设置读取失败和首次配置测试**

在测试文件末尾添加：

```js
test('设置读取失败时应停留看板并显示错误', async () => {
  const app = createHarness({ configError: '数据库不可用' });
  await app.flush();
  app.elements['error-msg'].classList.add('hidden');
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
  assert.match(app.elements['error-msg'].textContent, /无法打开设置：数据库不可用/);
});

test('首次配置页不应显示返回按钮', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, expired: false } });
  await app.flush();

  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['back-btn'].classList.contains('hidden'), true);
});
```

- [ ] **Step 2: 运行测试并确认失败**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: FAIL，现有设置入口吞掉错误并进入配置页。

- [ ] **Step 3: 实现集中设置入口**

用以下函数替换现有 `#settings-btn` 点击处理：

```js
async function openSettings() {
  try {
    const cfg = await invoke('get_config');
    resetConfigForm();
    cookieInput.placeholder = cfg.has_cookie
      ? '已安全保存；不修改请留空'
      : 'session=MTc4MzQyOTkyN3xE...';
    apikeyInput.placeholder = cfg.has_api_key
      ? '已安全保存；不修改请留空'
      : 'sk-...';
    setConfigBusy(false);
    backBtn.classList.toggle('hidden', !canReturnToDashboard);
    showScreen('config');
    cookieInput.focus();
  } catch (error) {
    showError('无法打开设置：' + getErrorMessage(error));
  }
}

settingsBtn.addEventListener('click', openSettings);
```

- [ ] **Step 4: 明确成功渲染和认证失败边界**

在 `renderDashboard(d)` 开头加入：

```js
hasDashboardData = true;
canReturnToDashboard = true;
```

保留原有 `hasDashboardData = true` 只出现一次。

在认证失败分支进入配置页前加入：

```js
canReturnToDashboard = false;
backBtn.classList.add('hidden');
```

并把该分支恢复按钮改为 `setConfigBusy(false)`。

- [ ] **Step 5: 限制后台事件**

将事件监听回调改为：

```js
window._unlistenDashboard = await listen('dashboard-updated', (event) => {
  if (event.payload && hasDashboardData) renderDashboard(event.payload);
});
```

- [ ] **Step 6: 初始化时明确返回按钮状态**

在 `init()` 获取配置后加入：

```js
canReturnToDashboard = false;
backBtn.classList.add('hidden');
```

首次配置、过期和初始化失败都保持该状态；只有 `renderDashboard()` 成功后才授予返回资格。

- [ ] **Step 7: 运行前端测试**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: 全部 PASS。

- [ ] **Step 8: 提交设置入口修复**

```bash
git add dist/app.js tests/frontend-navigation.test.js
git commit -m "fix: 修复设置入口与返回边界"
git push
```

### Task 5: 增加 Escape、焦点和忙碌状态验证

**Files:**
- Modify: `dist/app.js:190-249`
- Modify: `tests/frontend-navigation.test.js`

**Interfaces:**
- Consumes: `leaveSettings()`、`isSavingConfig`、`showScreen()`。
- Produces: `Escape` 导航；进入设置聚焦 Cookie；返回聚焦设置按钮；保存中阻止返回。

- [ ] **Step 1: 添加 Escape 与焦点测试**

在测试文件末尾添加：

```js
test('Escape 应复用设置返回规则', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  app.documentListeners.get('keydown')({ key: 'Escape' });

  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
  assert.equal(app.focused(), app.elements['settings-btn']);
});

test('进入设置页应聚焦 Cookie 输入框', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.focused(), app.elements['cookie-input']);
});
```

- [ ] **Step 2: 运行测试并确认 Escape 测试失败**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: FAIL，因为尚未注册 `keydown`。

- [ ] **Step 3: 注册 Escape 处理**

在按钮事件区加入：

```js
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !configScreen.classList.contains('hidden')) {
    leaveSettings();
  }
});
```

- [ ] **Step 4: 确保保存成功和失败正确恢复忙碌状态**

成功路径调整为：

```js
await invoke('save_config', { cookie, apiKey: apikeyInput.value.trim() });
if (cookie) hasSavedCookie = true;
await loadDashboard();
resetConfigForm();
```

保存失败 catch 内和认证失败分支执行 `setConfigBusy(false)`；提交开始执行 `setConfigBusy(true)`。

- [ ] **Step 5: 统一剩余异常提取与中文标点**

将 `loadDashboard()` 和 `init()` 中重复异常转换替换为 `getErrorMessage(e)`，并将面向用户的半角标点改为全角标点：

```js
showConfigError('Cookie 无效或已过期，请重新获取');
showConfigError('Cookie 太短（少于 50 个字符），请确认已完整复制。\n\n获取方式：浏览器 F12 → Application → Cookies → 双击 session 的 Value 列 → Ctrl+C');
```

- [ ] **Step 6: 运行前端测试和语法检查**

Run: `node --test tests/frontend-navigation.test.js`  
Expected: 全部 PASS。

Run: `node --check dist/app.js`  
Expected: 退出码 0，无输出。

- [ ] **Step 7: 提交键盘与反馈优化**

```bash
git add dist/app.js tests/frontend-navigation.test.js
git commit -m "feat: 完善设置键盘与焦点交互"
git push
```

### Task 6: 完整审查、静态验证与真实应用验收

**Files:**
- Modify if required by verified findings only: `dist/index.html`, `dist/style.css`, `dist/app.js`, `tests/frontend-navigation.test.js`
- Verify: workspace and real Tauri app

**Interfaces:**
- Consumes: Tasks 1-5 的完整设置导航实现。
- Produces: 已通过前端测试、Rust 检查、代码审查与真实 WebView2 验收的功能。

- [ ] **Step 1: 运行前端行为和语法检查**

```bash
node --test tests/frontend-navigation.test.js
node --check dist/app.js
```

Expected: 所有测试 PASS；语法检查退出码 0。

- [ ] **Step 2: 运行 Rust 格式、编译和测试**

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全部退出码 0，无 warning/error。

- [ ] **Step 3: 调用 Rust 与通用代码审查**

调用 `ecc:rust-reviewer` 检查本次变更对 Rust/Tauri 行为是否有回归风险；即使没有 Rust 文件变化，也确认 IPC 兼容边界。随后调用 `ecc:code-reviewer` 审查所有修改文件的正确性、可维护性和无障碍行为。对确认的问题先补测试再修复，并重复 Steps 1-2。

- [ ] **Step 4: 检查真实应用运行前提**

```bash
tasklist.exe /FI "IMAGENAME eq amax.exe"
```

Expected: 若没有 `amax.exe`，继续；若存在，停止并请用户关闭，不调用 `taskkill`。

- [ ] **Step 5: 启动真实 Tauri 应用**

Run: `cargo tauri dev`  
Expected: Rust 编译成功，WebView2 窗口显示，无启动错误。

- [ ] **Step 6: 在真实应用中逐项验收**

1. 已加载看板 → 设置 → 未修改 → 返回，无确认框。
2. 已加载看板 → 设置 → 修改 Cookie/API Key → 返回 → 取消，输入保留。
3. 重复上一步 → 确认，输入清空并返回原看板，不额外刷新。
4. 设置页按 `Escape`，行为与返回按钮一致。
5. 设置页按 `Enter`，触发表单保存与验证。
6. 保存过程中保存和返回按钮禁用，结束后状态恢复。
7. 首次启动、Cookie 过期、认证失败时返回按钮隐藏。
8. 手动刷新失败保留已有数据和原更新时间。
9. 最小化、关闭到托盘、托盘恢复和后台刷新无回归。

- [ ] **Step 7: 运行最终差异检查**

```bash
git diff --check
git status --short
```

Expected: `git diff --check` 无输出；状态只包含本计划允许的修改文件，`.claude/` 和 `.superpowers/` 等未跟踪目录不暂存。

- [ ] **Step 8: 提交并推送最终修正**

若审查或真实验收产生修正：

```bash
git add dist/index.html dist/style.css dist/app.js tests/frontend-navigation.test.js
git commit -m "fix: 完成设置导航验收优化"
git push
```

若没有额外修正，不创建空提交；确认前面每个任务提交均已推送。
