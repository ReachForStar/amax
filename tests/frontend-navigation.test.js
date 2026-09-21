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
    this.onFocus = null;
  }
  addEventListener(type, handler) { this.listeners.set(type, handler); }
  dispatch(type, event = {}) {
    return this.listeners.get(type)?.({ preventDefault() {}, key: '', ...event });
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  removeAttribute(name) { this.attributes.delete(name); }
  getAttribute(name) { return this.attributes.get(name); }
  focus() { this.onFocus?.(this); }
}

function createHarness({
  config,
  dashboard,
  confirm = true,
  configError,
  configErrorOnSettings = false,
  pendingSaveConfig = false,
  saveError,
  dashboardError,
  loginError,
} = {}) {
  const ids = [
    'config-screen', 'dashboard-screen', 'config-status', 'skeleton', 'error-msg',
    'dashboard-data', 'cookie-input', 'apikey-input', 'save-btn', 'back-btn',
    'config-form', 'refresh-btn', 'settings-btn', 'last-updated', 'user-name',
    'user-requests', 'today-yuan', 'log-count', 'today-tokens', 'token-detail',
    'quota-percent', 'progress-fill', 'quota-remaining', 'quota-total', 'quota-used',
    'login-btn', 'cookie-expiry',
    // 统计页与导出元素：harness 需覆盖 app.js 顶层引用的全部 #id，否则 vm 加载即抛错
    'stats-screen', 'stats-btn', 'stats-back-btn', 'range-start', 'range-end',
    'range-error', 'summary-title', 'trend-note', 'trend-error', 'trend-chart',
    'requests-chart', 'model-chart', 'model-error', 'model-basis-note', 'model-list',
    'remaining-block', 'remaining-chart', 'export-csv-btn', 'export-json-btn',
    'export-xlsx-btn',
  ];
  let focusedElement = null;
  const elements = Object.fromEntries(ids.map((id) => [id, new Element()]));
  Object.values(elements).forEach((element) => {
    element.onFocus = (focused) => { focusedElement = focused; };
  });
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
    querySelectorAll() { return []; },
  };
  const invokeCalls = [];
  const saveConfigCalls = [];
  let resolveSaveConfig;
  const saveConfigPromise = pendingSaveConfig
    ? new Promise((resolve) => { resolveSaveConfig = resolve; })
    : null;
  const invoke = async (command, args) => {
    invokeCalls.push(command);
    if (command === 'get_config') {
      if (configError || (configErrorOnSettings && invokeCalls.filter((call) => call === 'get_config').length > 1)) {
        throw configError ?? { code: 'storage', message: '数据库不可用' };
      }
      return config ?? { has_cookie: true, has_api_key: false, cookie_expires_at: null };
    }
    if (command === 'save_config') {
      saveConfigCalls.push(args);
      if (saveError) throw saveError;
      if (saveConfigPromise) return saveConfigPromise;
      return undefined;
    }
    if (command === 'fetch_dashboard') {
      if (dashboardError) throw dashboardError;
      return dashboard ?? {
        display_name: 'Tester', request_count: 3, logs_available: true,
        today_yuan: 1.25, log_count: 2, today_tokens: 1200,
        today_input: 800, today_output: 400, percent: 75,
        remaining: 75, total: 100, used: 25,
      };
    }
    if (command === 'open_login_window') {
      if (loginError) throw loginError;
      return undefined;
    }
    return undefined;
  };
  const eventHandlers = new Map();
  const listen = async (name, handler) => {
    eventHandlers.set(name, handler);
    return () => eventHandlers.delete(name);
  };
  const confirmCalls = [];
  const window = {
    __TAURI__: { core: { invoke, event: { listen } } },
    confirm(message) { confirmCalls.push(message); return confirm; },
    setInterval() { return 1; },
  };
  const context = vm.createContext({ console, Date, document, window });
  const source = fs.readFileSync(path.join(__dirname, '..', 'dist', 'app.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'dist/app.js' });
  return {
    elements,
    invokeCalls,
    saveConfigCalls,
    confirmCalls,
    documentListeners,
    focused: () => focusedElement,
    resolveSaveConfig: () => resolveSaveConfig?.(),
    emitEvent: (name, payload) => eventHandlers.get(name)?.({ payload }),
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

test('配置表单应使用提交语义并提供可访问的返回按钮', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'dist', 'index.html'), 'utf8');

  assert.match(html, /<form id="config-form"[^>]*>/);
  assert.match(html, /<button id="save-btn" type="submit"/);
  assert.match(html, /<button id="back-btn"[^>]*aria-label="返回看板"/);
  assert.match(html, /id="dashboard-screen"[^>]*aria-hidden="true"/);
});

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

test('设置读取失败时应停留看板并显示错误', async () => {
  const app = createHarness({ configErrorOnSettings: true });
  await app.flush();
  app.elements['error-msg'].classList.add('hidden');
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
  assert.match(app.elements['error-msg'].textContent, /无法打开设置：数据库不可用/);
});

test('首次配置页不应显示返回按钮', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['back-btn'].classList.contains('hidden'), true);
});

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

test('保存中应标记忙碌、禁用双按钮并阻止返回', async () => {
  const app = createHarness({ pendingSaveConfig: true });
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  app.elements['cookie-input'].value = `session=${'x'.repeat(50)}`;

  app.elements['config-form'].dispatch('submit');
  await app.flush();

  assert.equal(app.elements['config-form'].getAttribute('aria-busy'), 'true');
  assert.equal(app.elements['save-btn'].disabled, true);
  assert.equal(app.elements['back-btn'].disabled, true);
  await app.elements['back-btn'].dispatch('click');
  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);

  app.resolveSaveConfig();
  await app.flush();
});

test('保存失败应恢复忙碌状态并保留输入', async () => {
  const app = createHarness({ saveError: { code: 'storage', message: '本地数据库操作失败' } });
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  const cookie = `session=${'y'.repeat(50)}`;
  app.elements['cookie-input'].value = cookie;

  await app.elements['config-form'].dispatch('submit');
  await app.flush();

  assert.equal(app.elements['config-form'].getAttribute('aria-busy'), 'false');
  assert.equal(app.elements['save-btn'].disabled, false);
  assert.equal(app.elements['back-btn'].disabled, false);
  assert.equal(app.elements['cookie-input'].value, cookie);
  assert.match(app.elements['config-status'].textContent, /连接失败：本地数据库操作失败/);
});

test('认证失败应一键重登：进配置页、提示点登录按钮并聚焦', async () => {
  const app = createHarness({ dashboardError: { code: 'auth', message: 'Cookie 无效或已过期, 请重新登录获取' } });
  await app.flush();

  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), true);
  assert.match(app.elements['config-status'].textContent,
    /Cookie 无效或已过期, 请重新登录获取。请点击「使用官网登录获取」/);
  assert.equal(app.focused(), app.elements['login-btn']);
  assert.equal(app.elements['login-btn'].disabled, false);
  // 返回入口指向失效凭据，不能再回到陈旧看板
  assert.equal(app.elements['back-btn'].classList.contains('hidden'), true);
});

test('网络错误应留在看板显示重试文案，不引导重登', async () => {
  const app = createHarness({
    dashboardError: { code: 'network', message: '网络连接失败, 请检查网络或代理设置' },
  });
  await app.flush();

  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['config-screen'].classList.contains('hidden'), true);
  assert.match(app.elements['error-msg'].textContent,
    /^网络连接失败：网络连接失败, 请检查网络或代理设置$/);
  assert.doesNotMatch(app.elements['error-msg'].textContent, /使用官网登录获取/);
});

test('契约错误按通用获取失败提示', async () => {
  const app = createHarness({
    dashboardError: { code: 'data', message: '数据获取失败, 官网返回 HTTP 404' },
  });
  await app.flush();

  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
  assert.match(app.elements['error-msg'].textContent, /^获取数据失败：数据获取失败, 官网返回 HTTP 404$/);
});

test('保存成功但认证失败应恢复状态、保留输入和错误', async () => {
  const app = createHarness({
    dashboardError: { code: 'auth', message: 'Cookie 无效或已过期, 请重新登录获取' },
  });
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  const cookie = `session=${'z'.repeat(50)}`;
  app.elements['cookie-input'].value = cookie;

  await app.elements['config-form'].dispatch('submit');
  await app.flush();

  assert.equal(app.elements['config-form'].getAttribute('aria-busy'), 'false');
  assert.equal(app.elements['save-btn'].disabled, false);
  assert.equal(app.elements['back-btn'].disabled, false);
  assert.equal(app.elements['cookie-input'].value, cookie);
  assert.match(app.elements['config-status'].textContent,
    /Cookie 无效或已过期, 请重新登录获取。请点击「使用官网登录获取」/);
  assert.equal(app.elements['back-btn'].classList.contains('hidden'), true);
});

test('保存并加载看板成功后应清空配置输入', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  app.elements['cookie-input'].value = `session=${'s'.repeat(50)}`;
  app.elements['apikey-input'].value = 'sk-saved';

  await app.elements['config-form'].dispatch('submit');
  await app.flush();

  assert.equal(app.elements['cookie-input'].value, '');
  assert.equal(app.elements['apikey-input'].value, '');
  assert.equal(app.elements['config-form'].getAttribute('aria-busy'), 'false');
  assert.equal(app.elements['save-btn'].disabled, false);
  assert.equal(app.elements['back-btn'].disabled, false);
});

// ═══ 官网 WebView 登录 ═══

test('配置页应提供官网登录按钮且手动粘贴降级为兜底', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'dist', 'index.html'), 'utf8');

  assert.match(html, /<button id="login-btn" type="button"[^>]*>使用官网登录获取<\/button>/);
  const loginIdx = html.indexOf('id="login-btn"');
  const cookieIdx = html.indexOf('id="cookie-input"');
  assert.ok(loginIdx > -1 && cookieIdx > -1 && loginIdx < cookieIdx, '登录按钮应在 Cookie 输入之前');
  assert.match(html, /<span>或手动粘贴<\/span>/);
});

test('点击登录按钮应调用 open_login_window 并进入等待态', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  await app.elements['login-btn'].dispatch('click');
  await app.flush();

  assert.ok(app.invokeCalls.includes('open_login_window'));
  assert.equal(app.elements['login-btn'].disabled, true);
  assert.equal(app.elements['login-btn'].textContent, '等待登录…');
  assert.match(app.elements['config-status'].textContent, /请在窗口中完成登录/);
});

test('打开登录窗口失败应恢复按钮并提示', async () => {
  const app = createHarness({
    config: { has_cookie: false, has_api_key: false, cookie_expires_at: null },
    loginError: { code: 'storage', message: '创建登录窗口失败' },
  });
  await app.flush();

  await app.elements['login-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.elements['login-btn'].disabled, false);
  assert.equal(app.elements['login-btn'].textContent, '使用官网登录获取');
  assert.match(app.elements['config-status'].textContent, /无法打开登录窗口：创建登录窗口失败/);
});

test('登录成功应保存 Cookie 并进入看板', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  await app.elements['login-btn'].dispatch('click');
  await app.flush();
  app.emitEvent('login://success', { cookie: 'session=webview-cookie' });
  await app.flush();

  assert.ok(app.invokeCalls.includes('save_config'));
  assert.equal(app.saveConfigCalls.at(-1).expiresAt, null);
  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
  assert.equal(app.elements['config-form'].getAttribute('aria-busy'), 'false');
  assert.equal(app.elements['login-btn'].disabled, false);
});

test('登录窗口下发到期时间应透传保存并展示', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  await app.elements['login-btn'].dispatch('click');
  await app.flush();
  app.emitEvent('login://success', { cookie: 'session=webview-cookie', expires_at: '2026-12-31T08:00:00+00:00' });
  await app.flush();

  assert.equal(app.saveConfigCalls.at(-1).expiresAt, '2026-12-31T08:00:00+00:00');
  const expiry = app.elements['cookie-expiry'];
  assert.equal(expiry.classList.contains('hidden'), false);
  assert.match(expiry.textContent, /^凭据有效期至 .+（官网下发，仅供展示）$/);
});

test('登录成功后认证失败应回到配置页并提示', async () => {
  const app = createHarness({
    config: { has_cookie: false, has_api_key: false, cookie_expires_at: null },
    dashboardError: { code: 'auth', message: 'Cookie 无效或已过期, 请重新登录获取' },
  });
  await app.flush();

  await app.elements['login-btn'].dispatch('click');
  await app.flush();
  app.emitEvent('login://success', { cookie: 'session=stale' });
  await app.flush();

  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
  assert.match(app.elements['config-status'].textContent, /Cookie 无效或已过期/);
  assert.equal(app.elements['login-btn'].disabled, false);
});

test('手动粘贴保存应清掉上次登录留下的有效期', async () => {
  const app = createHarness({ config: { has_cookie: true, has_api_key: false, cookie_expires_at: '2026-12-31T08:00:00+00:00' } });
  await app.flush();
  assert.match(app.elements['cookie-expiry'].textContent, /^凭据有效期至 /);

  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  app.elements['cookie-input'].value = `session=${'m'.repeat(50)}`;
  await app.elements['config-form'].dispatch('submit');
  await app.flush();

  assert.equal(app.saveConfigCalls.at(-1).expiresAt, null);
  assert.equal(app.elements['cookie-expiry'].textContent, '官网未下发过期时间，失效由服务端判定');
});

test('官网未下发有效期时应明示由服务端判定', async () => {
  const app = createHarness({ config: { has_cookie: true, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  assert.equal(app.elements['cookie-expiry'].classList.contains('hidden'), false);
  assert.equal(app.elements['cookie-expiry'].textContent, '官网未下发过期时间，失效由服务端判定');
});

test('无凭据时不显示有效期说明', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  assert.equal(app.elements['cookie-expiry'].classList.contains('hidden'), true);
  assert.equal(app.elements['cookie-expiry'].textContent, '');
});

test('后台刷新发现凭据失效应引导重登而非停留在陈旧看板', async () => {
  const app = createHarness();
  await app.flush();
  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);

  app.emitEvent('auth://expired');
  await app.flush();

  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
  assert.match(app.elements['config-status'].textContent,
    /登录状态已失效。请点击「使用官网登录获取」/);
  assert.equal(app.focused(), app.elements['login-btn']);
});

test('已在配置页或保存中收到失效事件不应打断当前操作', async () => {
  const app = createHarness({ pendingSaveConfig: true, configErrorOnSettings: false });
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  app.elements['cookie-input'].value = `session=${'n'.repeat(50)}`;
  app.elements['config-form'].dispatch('submit');
  await app.flush();

  app.emitEvent('auth://expired');
  await app.flush();
  assert.doesNotMatch(app.elements['config-status'].textContent, /登录状态已失效/);
  assert.equal(app.elements['save-btn'].disabled, true);

  app.resolveSaveConfig();
  await app.flush();
});

test('登录取消应恢复按钮并给出提示', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  await app.elements['login-btn'].dispatch('click');
  await app.flush();
  app.emitEvent('login://cancelled');
  await app.flush();

  assert.equal(app.elements['login-btn'].disabled, false);
  assert.equal(app.elements['login-btn'].textContent, '使用官网登录获取');
  assert.match(app.elements['config-status'].textContent, /已取消登录/);
});

test('登录超时应恢复按钮并提示', async () => {
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, cookie_expires_at: null } });
  await app.flush();

  await app.elements['login-btn'].dispatch('click');
  await app.flush();
  app.emitEvent('login://timeout');
  await app.flush();

  assert.equal(app.elements['login-btn'].disabled, false);
  assert.match(app.elements['config-status'].textContent, /登录超时/);
});

test('非等待态收到登录事件不应改变按钮', async () => {
  const app = createHarness();
  await app.flush();

  app.emitEvent('login://cancelled');
  await app.flush();

  assert.equal(app.elements['login-btn'].disabled, false);
  assert.ok(!app.invokeCalls.includes('save_config'));
});
