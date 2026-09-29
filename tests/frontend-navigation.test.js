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
    this.checked = false;
    this.placeholder = '';
    this.textContent = '';
    this.disabled = false;
    this.attributes = new Map();
    this.listeners = new Map();
    this.style = {};
    this.onFocus = null;
    this.dataset = {};
    this.title = '';
  }
  addEventListener(type, handler) { this.listeners.set(type, handler); }
  // app.js 沿用真实 DOM 的 className 整体赋值写法（见 config-status），harness 必须让
  // 它与 classList 同步，否则测试看到的 hidden/error 状态会和浏览器里的不一样
  get className() { return [...this.classList.values].join(' '); }
  set className(name) {
    this.classList = new ClassList(String(name).split(/\s+/).filter(Boolean));
  }
  dispatch(type, event = {}) {
    return this.listeners.get(type)?.({ preventDefault() {}, key: '', ...event });
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  removeAttribute(name) { this.attributes.delete(name); }
  getAttribute(name) { return this.attributes.get(name); }
  focus() { this.onFocus?.(this); }
  append(...children) { return this.appendChild(children[0]); }
  appendChild(child) { return child; }
  closest() { return this; }
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
  appVersion = '0.2.3',
  versionError,
  checkUpdateError,
  applyUpdateError,
  channelsPayload,
  channelsError,
  setChannelsError,
  testChannelError,
  testChannelResult = { ok: true, channel: 'notification', error: null },
  alertSettingsPayload,
  alertSettingsError,
  setAlertSettingsError,
} = {}) {
  const ids = [
    'config-screen', 'dashboard-screen', 'config-status', 'skeleton', 'error-msg',
    'dashboard-data', 'cookie-input', 'apikey-input', 'save-btn', 'back-btn',
    'config-form', 'refresh-btn', 'settings-btn', 'last-updated', 'user-name',
    'user-requests', 'today-yuan', 'log-count', 'today-tokens', 'token-detail',
    'quota-percent', 'progress-fill', 'quota-remaining', 'quota-total', 'quota-used',
    'login-btn', 'cookie-expiry',
    // 更新区元素
    'app-version', 'update-status', 'check-update-btn', 'apply-update-btn',
    // 通知渠道区元素
    'ch-notification', 'ch-meow', 'ch-mail', 'ch-notification-state', 'ch-meow-state',
    'ch-mail-state', 'meow-nickname-input', 'smtp-host',
    'smtp-port', 'smtp-tls', 'smtp-user', 'smtp-to', 'smtp-auth-code',
    'clear-smtp-auth-code', 'channel-notices', 'channels-status', 'save-channels-btn',
    'test-notification-btn', 'test-meow-btn', 'test-mail-btn', 'delivery-log',
    'delivery-summary',
    // 告警规则区元素
    'alert-enabled', 'rule-quota-low', 'rule-runout-soon', 'rule-spike',
    'alert-quota-percent', 'alert-runout-days', 'alert-spike-multiplier',
    'alert-max-fires', 'alert-progress', 'alert-settings-status',
    'save-alert-settings-btn',
    // 统计页与导出元素：harness 需覆盖 app.js 顶层引用的全部 #id，否则 vm 加载即抛错
    'stats-screen', 'stats-btn', 'stats-back-btn', 'range-start', 'range-end',
    'range-error', 'summary-title', 'trend-note', 'trend-error', 'trend-chart',
    'requests-chart', 'model-chart', 'model-error', 'model-basis-note', 'model-list',
    'remaining-block', 'remaining-chart', 'export-csv-btn', 'export-json-btn',
    'export-xlsx-btn', 'sum-total-yuan', 'sum-avg-yuan',
    'sum-peak', 'sum-days', 'sum-requests',
  ];
  let focusedElement = null;
  const elements = Object.fromEntries(ids.map((id) => [id, new Element()]));
  Object.values(elements).forEach((element) => {
    element.onFocus = (focused) => { focusedElement = focused; };
  });
  elements['dashboard-screen'].classList.add('hidden');
  elements['dashboard-data'].classList.add('hidden');
  elements['stats-screen'].classList.add('hidden');
  elements.skeleton.classList.add('hidden');
  elements['error-msg'].classList.add('hidden');
  elements['config-status'].classList.add('hidden');
  elements['back-btn'].classList.add('hidden');
  elements['update-status'].classList.add('hidden');
  elements['apply-update-btn'].classList.add('hidden');
  const presets = [7, 14, 30].map((days, index) => {
    const btn = new Element();
    btn.dataset.days = String(days);
    if (index === 0) btn.classList.add('is-active');
    return btn;
  });
  const progressBar = new Element();
  const documentListeners = new Map();
  const document = {
    body: { innerHTML: '' },
    head: {
      appendChild(child) {
        // 模拟同源脚本加载成功：onload 在下一个微任务触发
        if (child.src) queueMicrotask(() => child.onload?.());
        return child;
      },
    },
    createElement(tag) { return new Element(); },
    querySelector(selector) {
      if (selector === '.progress-bar') return progressBar;
      if (selector.startsWith('#')) return elements[selector.slice(1)];
      return null;
    },
    querySelectorAll(selector) {
      if (selector === '.range-preset') return presets;
      return [];
    },
    addEventListener(type, handler) {
      // 真实 DOM 允许同一类型挂多个监听（app.js 在此同时挂 keydown 返回看板与活动上报）
      if (!documentListeners.has(type)) documentListeners.set(type, []);
      documentListeners.get(type).push(handler);
    },
  };
  const invokeCalls = [];
  const saveConfigCalls = [];
  // 更新相关命令按调用顺序记录（check / apply），活动上报只数次数（5 秒节流）
  const updateCalls = [];
  // 渠道命令入参原样记下，测试据此断言「凭据留空不提交」「清除才提交空串」
  const setChannelsCalls = [];
  const testChannelCalls = [];
  // 告警参数按整包 args 记下（settings + rulesEnabled），测试据此断言提交形状
  const setAlertSettingsCalls = [];
  let activityCalls = 0;
  let resolveSaveConfig;
  const saveConfigPromise = pendingSaveConfig
    ? new Promise((resolve) => { resolveSaveConfig = resolve; })
    : null;
  // 统计用量请求队列：每个请求挂起，测试按需以任意顺序 resolve
  const usageCalls = [];
  // deliver::view 的最小可信形状：只开系统通知、没填过昵称也没配过凭据
  const defaultChannelsPayload = () => ({
    channels: {
      notification: { enabled: true, available: 'ready' },
      meow: { enabled: false, available: 'off', nickname: null, notice: null },
      mail: {
        enabled: false, available: 'off', host: '', port: 465, tls: 'implicit',
        user: '', to: '', authCodeConfigured: false, notice: null,
      },
      problems: [],
    },
    deliveries: [],
  });
  // 后端保存后回读的是真实视图，桩里按提交值映射，保证「回读覆盖输入框」这条路被测到
  const viewFromSubmission = (submitted) => ({
    notification: {
      enabled: submitted.notification,
      available: submitted.notification ? 'ready' : 'off',
    },
    meow: {
      enabled: submitted.meow,
      available: submitted.meow && submitted.meowNickname ? 'ready' : 'off',
      nickname: submitted.meowNickname || null,
      notice: null,
    },
    mail: {
      enabled: submitted.mail,
      available: 'off',
      host: submitted.smtp?.host ?? '',
      port: submitted.smtp?.port ?? 465,
      tls: submitted.smtp?.tls ?? 'implicit',
      user: submitted.smtp?.user ?? '',
      to: submitted.smtp?.to ?? '',
      authCodeConfigured: Boolean(submitted.smtp?.authCode),
      notice: null,
    },
    problems: [],
  });
  // get_alert_settings 的最小可信形状：默认参数、三条规则全开、今天还没投过（区间与后端 param_ranges 一致）
  const defaultAlertSettingsPayload = () => ({
    settings: {
      enabled: true, quotaPercent: 10, runoutDays: 5,
      spikeMultiplier: 3, maxFiresPerDay: 2,
    },
    rulesEnabled: ['quota_low', 'runout_soon', 'spike'],
    paramRanges: [
      { key: 'quota_percent', min: 1, max: 99 },
      { key: 'runout_days', min: 1, max: 90 },
      { key: 'spike_multiplier', min: 1.5, max: 100 },
      { key: 'max_fires_per_day', min: 0, max: 10 },
    ],
    status: { state: 'armed', day: '2026-09-22', firedToday: 0, cap: 2 },
  });
  // 后端 set_alert_settings 回读的是 sanitized() 之后的值，桩照做同样的夹取，
  // 才能测出「界面显示的是真正生效的那个数」
  const clamp = (value, min, max) => Math.min(Math.max(value, min), max);
  const alertSettingsFromSubmission = ({ settings, rulesEnabled }) => ({
    settings: {
      enabled: settings.enabled,
      quotaPercent: clamp(settings.quotaPercent, 1, 99),
      runoutDays: clamp(settings.runoutDays, 1, 90),
      spikeMultiplier: clamp(settings.spikeMultiplier, 1.5, 100),
      maxFiresPerDay: clamp(settings.maxFiresPerDay, 0, 10),
    },
    rulesEnabled,
    status: {
      state: settings.enabled ? 'armed' : 'disabled',
      day: '2026-09-22',
      firedToday: 0,
      cap: clamp(settings.maxFiresPerDay, 0, 10),
    },
  });
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
    if (command === 'fetch_usage_stats') {
      return new Promise((resolve) => usageCalls.push({ args, resolve }));
    }
    if (command === 'get_app_version') {
      if (versionError) throw versionError;
      return appVersion;
    }
    if (command === 'check_for_updates_now') {
      updateCalls.push('check');
      if (checkUpdateError) throw checkUpdateError;
      return undefined;
    }
    if (command === 'apply_update_now') {
      updateCalls.push('apply');
      if (applyUpdateError) throw applyUpdateError;
      return undefined;
    }
    if (command === 'report_user_activity') {
      activityCalls += 1;
      return undefined;
    }
    if (command === 'get_alert_channels') {
      if (channelsError) throw channelsError;
      return channelsPayload ?? defaultChannelsPayload();
    }
    if (command === 'set_alert_channels') {
      setChannelsCalls.push(args.channels);
      if (setChannelsError) throw setChannelsError;
      return viewFromSubmission(args.channels);
    }
    if (command === 'test_alert_channel') {
      testChannelCalls.push(args.channel);
      if (testChannelError) throw testChannelError;
      return testChannelResult;
    }
    if (command === 'get_alert_settings') {
      if (alertSettingsError) throw alertSettingsError;
      return alertSettingsPayload ?? defaultAlertSettingsPayload();
    }
    if (command === 'set_alert_settings') {
      setAlertSettingsCalls.push(args);
      if (setAlertSettingsError) throw setAlertSettingsError;
      return alertSettingsFromSubmission(args);
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
  // Chart 桩：统计页渲染路径的 new Chart / destroy 调用
  const chartStubInstances = [];
  class Chart {
    constructor(canvas, config) { this.config = config; chartStubInstances.push(this); }
    destroy() { }
  }
  const context = vm.createContext({ console, Date, document, window, Chart });
  const source = fs.readFileSync(path.join(__dirname, '..', 'dist', 'app.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'dist/app.js' });
  return {
    elements,
    presets,
    invokeCalls,
    saveConfigCalls,
    usageCalls,
    chartStubInstances,
    confirmCalls,
    updateCalls,
    setChannelsCalls,
    testChannelCalls,
    setAlertSettingsCalls,
    documentListeners,
    activityCalls: () => activityCalls,
    // 同类型可能挂了多个监听（如 keydown：返回看板 + 活动上报），按注册顺序全部触发
    dispatchDocument(type, event = {}) {
      documentListeners.get(type)?.forEach((handler) => handler({ preventDefault() {}, key: '', ...event }));
    },
    focused: () => focusedElement,
    resolveSaveConfig: () => resolveSaveConfig?.(),
    emitEvent: (name, payload) => eventHandlers.get(name)?.({ payload }),
    // 以指定顺序结算第 index 个统计用量请求（0 为最早发起）
    resolveUsage: (index, value) => usageCalls[index]?.resolve(value),
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

  app.dispatchDocument('keydown', { key: 'Escape' });

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

// ═══ 自动更新 ═══

test('设置页应显示当前版本并提供检查更新入口', async () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'dist', 'index.html'), 'utf8');
  assert.match(html, /<button id="check-update-btn" type="button"/);
  assert.match(html, /<span id="app-version">/);

  const app = createHarness({ appVersion: '0.2.3' });
  await app.flush();

  assert.ok(app.invokeCalls.includes('get_app_version'));
  assert.equal(app.elements['app-version'].textContent, 'v0.2.3');
});

test('版本读取失败不应中断启动流程', async () => {
  const app = createHarness({ versionError: { code: 'storage', message: '不可用' } });
  await app.flush();

  assert.equal(app.elements['app-version'].textContent, '版本未知');
  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
});

test('点击检查更新应触发后台检查并恢复按钮', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  await app.elements['check-update-btn'].dispatch('click');
  await app.flush();

  assert.deepEqual(app.updateCalls, ['check']);
  assert.equal(app.elements['check-update-btn'].disabled, false);
});

test('更新就绪状态应展示后端下发文案并给出立即安装入口', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  app.emitEvent('update://status', {
    state: 'staged', version: '0.3.0', message: 'v0.3.0 已下载并验签，将在应用空闲时自动安装并重启（约 90 秒无操作）',
  });
  await app.flush();

  assert.equal(app.elements['update-status'].classList.contains('hidden'), false);
  assert.match(app.elements['update-status'].textContent, /已下载并验签/);
  assert.equal(app.elements['apply-update-btn'].classList.contains('hidden'), false);
});

test('非就绪状态不应出现立即安装入口', async () => {
  const app = createHarness();
  await app.flush();

  app.emitEvent('update://status', { state: 'up_to_date', version: null, message: null });
  await app.flush();

  assert.equal(app.elements['update-status'].textContent, '已是最新版本');
  assert.equal(app.elements['apply-update-btn'].classList.contains('hidden'), true);
});

test('点击立即安装应调用 apply_update_now，失败时撤下入口', async () => {
  const app = createHarness({ applyUpdateError: { code: 'storage', message: '安装 v0.3.0 失败: msiexec 拒绝' } });
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();
  app.emitEvent('update://status', { state: 'staged', version: '0.3.0', message: '就绪' });
  await app.flush();

  await app.elements['apply-update-btn'].dispatch('click');
  await app.flush();

  assert.deepEqual(app.updateCalls, ['apply']);
  assert.match(app.elements['update-status'].textContent, /安装 v0.3.0 失败/);
  assert.equal(app.elements['apply-update-btn'].classList.contains('hidden'), true);
});

test('用户活动应节流上报，且不与返回看板的 Escape 冲突', async () => {
  const app = createHarness();
  await app.flush();
  await app.elements['settings-btn'].dispatch('click');
  await app.flush();

  app.dispatchDocument('pointerdown');
  app.dispatchDocument('pointerdown');
  assert.equal(app.activityCalls(), 1);

  // keydown 上同时挂着「Escape 返回看板」与活动上报两个监听：Escape 照常生效，
  // 活动上报因落在同一节流窗口内被合并，不会每个事件都打一次 IPC
  assert.equal(app.documentListeners.get('keydown').length, 2);
  app.dispatchDocument('keydown', { key: 'Escape' });
  assert.equal(app.activityCalls(), 1);
  assert.equal(app.elements['dashboard-screen'].classList.contains('hidden'), false);
});

// ═══ 告警规则 ═══

test('进入设置页应回显告警参数、规则勾选与当日进度', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;

  assert.equal(e['alert-enabled'].checked, true);
  assert.equal(e['rule-quota-low'].checked, true);
  assert.equal(e['rule-runout-soon'].checked, true);
  assert.equal(e['rule-spike'].checked, true);
  assert.equal(e['alert-quota-percent'].value, '10');
  assert.equal(e['alert-runout-days'].value, '5');
  assert.equal(e['alert-spike-multiplier'].value, '3');
  assert.equal(e['alert-max-fires'].value, '2');
  assert.equal(e['alert-progress'].textContent, '待触发 · 今天 0/2 次');
  // 区间只有一份来源：后端 param_ranges 覆盖界面上的静态默认值
  assert.equal(e['alert-quota-percent'].max, '99');
  assert.equal(e['alert-runout-days'].max, '90');
  assert.equal(e['alert-spike-multiplier'].min, '1.5');
  assert.equal(e['alert-max-fires'].min, '0');
});

test('保存应提交 camelCase 参数与勾选规则的 key 子集', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['alert-quota-percent'].value = '20';
  e['alert-max-fires'].value = '4';
  e['rule-spike'].checked = false;
  await e['save-alert-settings-btn'].dispatch('click');
  await app.flush();

  assert.deepEqual(plain(app.setAlertSettingsCalls), [{
    settings: {
      enabled: true, quotaPercent: 20, runoutDays: 5,
      spikeMultiplier: 3, maxFiresPerDay: 4,
    },
    rulesEnabled: ['quota_low', 'runout_soon'],
  }]);
  assert.match(e['alert-settings-status'].textContent, /已保存/);
  assert.equal(e['alert-settings-status'].classList.contains('error'), false);
});

test('越界参数应按后端夹取结果回显，不留下界面与库里不一致的数字', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['alert-quota-percent'].value = '200';
  await e['save-alert-settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.setAlertSettingsCalls.at(-1).settings.quotaPercent, 200);
  assert.equal(e['alert-quota-percent'].value, '99');
  assert.equal(e['alert-progress'].textContent, '待触发 · 今天 0/2 次');
});

test('参数留空不该被静默夹成下限值', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['alert-runout-days'].value = '';
  e['alert-spike-multiplier'].value = '';
  await e['save-alert-settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.setAlertSettingsCalls.length, 0, '不合法的输入不该打到后端');
  assert.equal(e['alert-settings-status'].classList.contains('error'), true);
  assert.match(e['alert-settings-status'].textContent, /请填写：可用天数（天）、花费倍数（倍）/);
  // 拦下之后输入框保持用户写的内容，不替他改数
  assert.equal(e['alert-runout-days'].value, '');
});

test('整数字段带小数应给出可读提示而非 serde 类型错误', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['alert-max-fires'].value = '1.5';
  await e['save-alert-settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.setAlertSettingsCalls.length, 0);
  assert.match(e['alert-settings-status'].textContent, /需为整数：每日最多通知（次）/);
});

test('每日最多通知 0 次是合法输入，表示当日全部静默', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['alert-max-fires'].value = '0';
  await e['save-alert-settings-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.setAlertSettingsCalls.at(-1).settings.maxFiresPerDay, 0);
  assert.equal(e['alert-max-fires'].value, '0');
});

test('告警设置读取失败不该挡住渠道区与参数填写', async () => {
  const app = createHarness({ alertSettingsError: { code: 'storage', message: '数据库不可用' } });
  await openSettings(app);
  const e = app.elements;

  assert.equal(e['alert-settings-status'].classList.contains('error'), true);
  assert.match(e['alert-settings-status'].textContent, /告警设置读取失败：数据库不可用/);
  // 渠道区照常读出并渲染
  assert.equal(e['ch-notification'].checked, true);
  assert.equal(e['delivery-summary'].textContent, '今日无投递');
  // 参数区停在空值上，用户仍能直接填
  e['alert-quota-percent'].value = '15';
  e['alert-runout-days'].value = '3';
  e['alert-spike-multiplier'].value = '2';
  e['alert-max-fires'].value = '1';
  await e['save-alert-settings-btn'].dispatch('click');
  await app.flush();
  assert.equal(app.setAlertSettingsCalls.at(-1).settings.quotaPercent, 15);
});

test('alert://status 事件应刷新当日进度行', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;

  app.emitEvent('alert://status', {
    state: 'cap_hit', day: '2026-09-22', firedToday: 2, cap: 2,
  });
  assert.equal(e['alert-progress'].textContent, '今日已达上限 · 今天 2/2 次');

  app.emitEvent('alert://status', {
    state: 'disabled', day: '2026-09-23', firedToday: 0, cap: 2,
  });
  assert.equal(e['alert-progress'].textContent, '已停用 · 今天 0/2 次');
});

test('未保存的告警参数在返回看板前应确认，保存后不再打扰', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;

  e['alert-quota-percent'].value = '25';
  await e['back-btn'].dispatch('click');
  assert.deepEqual(app.confirmCalls, ['放弃未保存的修改并返回吗？']);

  await e['save-alert-settings-btn'].dispatch('click');
  await app.flush();
  await e['back-btn'].dispatch('click');
  assert.deepEqual(app.confirmCalls, ['放弃未保存的修改并返回吗？'], '保存成功后不该再确认第二次');
  assert.equal(e['config-screen'].classList.contains('hidden'), true);
});

test('告警规则区应可访问且按钮不带提交语义', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'dist', 'index.html'), 'utf8');
  const tagOf = (id) => html.match(new RegExp(`<(input|button)[^>]*id="${id}"[^>]*>`))?.[0] ?? '';

  assert.match(html, /<section class="alert-block"[^>]*aria-labelledby="rules-title"/);
  assert.match(tagOf('save-alert-settings-btn'), /type="button"/);
  assert.match(html, /id="alert-settings-status"[^>]*role="status"/);
  assert.match(tagOf('alert-quota-percent'), /type="number"/);
  assert.match(tagOf('alert-max-fires'), /min="0"/);
  // 参数区在 form 之外，误按回车不该触发凭据保存
  assert.equal(html.indexOf('<form id="config-form"') < html.indexOf('id="alert-quota-percent"'), true);
  assert.equal(html.indexOf('</form>') < html.indexOf('id="alert-quota-percent"'), true);
});

// ═══ 通知渠道 ═══

// 一个「三个渠道都开着、昵称已填、授权码解不开、库里还有脏字段」的视图：
// 覆盖明文回显与打码回显这两条不同的口径、状态标签、提示汇总与投递复盘
function richChannelsPayload() {
  return {
    channels: {
      notification: { enabled: true, available: 'ready' },
      meow: { enabled: true, available: 'ready', nickname: 'pikachu', notice: null },
      mail: {
        enabled: true, available: 'incomplete', host: 'smtp.qq.com', port: 465, tls: 'implicit',
        user: 'me@qq.com', to: 'you@qq.com', authCodeConfigured: true,
        notice: '凭据无法解密（换过机器或 Windows 用户），请重新填写',
      },
      problems: ['smtp_port 不是合法端口，已按 465 保存'],
    },
    deliveries: [
      { rule: 'quota_low', channel: 'mail', ok: true, error: null, at: '2026-09-22T08:15:00Z' },
      { rule: 'spike', channel: 'meow', ok: false, error: '连接超时', at: '2026-09-22T07:02:00Z' },
    ],
  };
}

async function openSettings(harness) {
  await harness.flush();
  await harness.elements['settings-btn'].dispatch('click');
  await harness.flush();
}

// app.js 跑在 vm 上下文里，它造出的对象带着另一个 realms 的 Object 原型，
// deepStrictEqual 会因为「同结构不同引用」判失败——断言前先归一成宿主纯对象
const plain = (value) => JSON.parse(JSON.stringify(value));

test('进入设置页应原样回显昵称并把授权码按打码口径展示', async () => {
  const app = createHarness({ channelsPayload: richChannelsPayload() });
  await openSettings(app);
  const e = app.elements;

  assert.equal(e['ch-notification'].checked, true);
  assert.equal(e['ch-meow'].checked, true);
  assert.equal(e['ch-mail'].checked, true);
  assert.equal(e['ch-notification-state'].textContent, '可用');
  assert.equal(e['ch-meow-state'].textContent, '可用');
  assert.equal(e['ch-mail-state'].textContent, '缺配置');

  // 昵称是明文配置：输入框里就是库里的值，可以直接改
  assert.equal(e['meow-nickname-input'].value, 'pikachu');
  // 授权码是真凭据：视图不给原值，输入框必须留空，靠 placeholder 说明「已保存」，靠「清除」才允许删
  assert.equal(e['smtp-auth-code'].value, '');
  assert.match(e['smtp-auth-code'].placeholder, /已保存/);
  assert.equal(e['clear-smtp-auth-code'].classList.contains('hidden'), false);
  assert.equal(e['smtp-host'].value, 'smtp.qq.com');
  assert.equal(e['smtp-port'].value, '465');
  assert.equal(e['smtp-to'].value, 'you@qq.com');

  assert.match(e['channel-notices'].textContent, /无法解密/);
  assert.match(e['channel-notices'].textContent, /smtp_port/);
  assert.equal(e['channel-notices'].classList.contains('hidden'), false);

  assert.equal(e['delivery-summary'].textContent, '今日 2 条投递');
  assert.match(e['delivery-log'].textContent, /08:15 额度低于阈值 · 邮件 · 已送达/);
  assert.match(e['delivery-log'].textContent, /07:02 用量突增 · MeoW 推送 · 失败：连接超时/);
});

test('未配任何凭据时渠道区应干净呈现且不留陈旧提示', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;

  assert.equal(e['ch-meow'].checked, false);
  assert.equal(e['meow-nickname-input'].value, '');
  assert.equal(e['clear-smtp-auth-code'].classList.contains('hidden'), true);
  assert.equal(e['channel-notices'].classList.contains('hidden'), true);
  assert.equal(e['delivery-summary'].textContent, '今日无投递');
  assert.equal(e['delivery-log'].classList.contains('hidden'), true);
  assert.equal(e['channels-status'].classList.contains('hidden'), true);
});

test('只开系统通知时保存应提交 smtp:null 且不带上任何凭据', async () => {
  const app = createHarness();
  await openSettings(app);
  await app.elements['save-channels-btn'].dispatch('click');
  await app.flush();

  assert.deepEqual(plain(app.setChannelsCalls), [{
    notification: true, meow: false, mail: false, meowNickname: '', smtp: null,
  }]);
  assert.equal(app.elements['channels-status'].classList.contains('error'), false);
  assert.match(app.elements['channels-status'].textContent, /已保存/);
});

test('填了 SMTP 明文但没开邮件也应整体提交，避免丢弃已输入内容', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['smtp-host'].value = 'smtp.exmail.qq.com';
  e['smtp-port'].value = '465';
  e['smtp-user'].value = 'me@qq.com';
  await e['save-channels-btn'].dispatch('click');
  await app.flush();

  assert.deepEqual(plain(app.setChannelsCalls[0].smtp), {
    host: 'smtp.exmail.qq.com', port: 465, tls: 'implicit', user: 'me@qq.com',
    to: '', authCode: null,
  });
  assert.equal(app.setChannelsCalls[0].mail, false);
});

test('点清除才提交空授权码，留空表示保持原值', async () => {
  const app = createHarness({ channelsPayload: richChannelsPayload() });
  await openSettings(app);
  await app.elements['clear-smtp-auth-code'].dispatch('click');
  await app.flush();

  const submitted = app.setChannelsCalls[app.setChannelsCalls.length - 1];
  assert.equal(submitted.smtp.authCode, '');
  // 昵称不是凭据：清除授权码不该把它一起抹掉，提交的仍是库里回显的原值
  assert.equal(submitted.meowNickname, 'pikachu');
  assert.match(app.elements['channels-status'].textContent, /授权码已清除/);
});

test('把昵称清空再保存就是删除，不需要额外的清除按钮', async () => {
  const app = createHarness({ channelsPayload: richChannelsPayload() });
  await openSettings(app);
  const e = app.elements;
  e['meow-nickname-input'].value = '';
  await e['save-channels-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.setChannelsCalls.at(-1).meowNickname, '');
  assert.equal(e['meow-nickname-input'].value, '');
  assert.equal(e['ch-meow-state'].textContent, '未开启');
});

test('保存成功后应清空授权码输入并按后端视图回读昵称', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['ch-meow'].checked = true;
  e['meow-nickname-input'].value = 'pikachu';
  await e['save-channels-btn'].dispatch('click');
  await app.flush();

  // 昵称回读后仍留在框里（明文配置），授权码则必须被清掉（真凭据）
  assert.equal(e['meow-nickname-input'].value, 'pikachu');
  assert.equal(e['smtp-auth-code'].value, '');
  assert.equal(e['ch-meow-state'].textContent, '可用');
  // 回读之后表单应与基线一致，返回看板不该再要求确认
  e['back-btn'].dispatch('click');
  assert.deepEqual(app.confirmCalls, []);
});

test('渠道保存失败应保留输入并在下次返回时提示未保存', async () => {
  const app = createHarness({ setChannelsError: { code: 'input', message: 'QQ 邮箱主机不合法' } });
  await openSettings(app);
  const e = app.elements;
  e['smtp-host'].value = 'smtp.qq.com';
  e['ch-mail'].checked = true;
  await e['save-channels-btn'].dispatch('click');
  await app.flush();

  assert.equal(e['channels-status'].classList.contains('error'), true);
  assert.match(e['channels-status'].textContent, /QQ 邮箱主机不合法/);
  assert.equal(e['smtp-host'].value, 'smtp.qq.com');
  assert.equal(e['save-channels-btn'].disabled, false);

  // 写库失败时基线不能推进：这些输入仍是未保存的修改
  await e['back-btn'].dispatch('click');
  assert.deepEqual(app.confirmCalls, ['放弃未保存的修改并返回吗？']);
});

test('测试投递按钮应按渠道调用后端并复盘当日投递记录', async () => {
  const app = createHarness({ channelsPayload: richChannelsPayload() });
  await openSettings(app);
  const before = app.invokeCalls.filter((call) => call === 'get_alert_channels').length;
  await app.elements['test-mail-btn'].dispatch('click');
  await app.flush();

  assert.deepEqual(app.testChannelCalls, ['mail']);
  assert.match(app.elements['channels-status'].textContent, /邮件：测试消息已送达/);
  assert.equal(app.elements['test-mail-btn'].disabled, false);
  const after = app.invokeCalls.filter((call) => call === 'get_alert_channels').length;
  assert.equal(after, before + 1, '测试完应重读投递记录');
});

test('测试投递被后端判为失败时应展示脱敏原因而不是成功文案', async () => {
  const app = createHarness({
    channelsPayload: richChannelsPayload(),
    testChannelResult: { ok: false, channel: 'meow', error: 'HTTP 429 请求过于频繁' },
  });
  await openSettings(app);
  await app.elements['test-meow-btn'].dispatch('click');
  await app.flush();

  assert.equal(app.elements['channels-status'].classList.contains('error'), true);
  assert.match(app.elements['channels-status'].textContent, /MeoW 推送测试失败：HTTP 429 请求过于频繁/);
});

test('测试投递抛错应回到可操作状态并给出错误', async () => {
  const app = createHarness({
    channelsPayload: richChannelsPayload(),
    testChannelError: { code: 'network', message: 'DNS 解析失败' },
  });
  await openSettings(app);
  await app.elements['test-notification-btn'].dispatch('click');
  await app.flush();

  assert.match(app.elements['channels-status'].textContent, /系统通知测试失败：DNS 解析失败/);
  assert.equal(app.elements['test-notification-btn'].disabled, false);
  assert.equal(app.elements['save-channels-btn'].disabled, false);
});

test('渠道设置读取失败不应挡住凭据配置', async () => {
  const app = createHarness({ channelsError: { code: 'storage', message: '数据库不可用' } });
  await openSettings(app);
  const e = app.elements;

  assert.equal(e['config-screen'].classList.contains('hidden'), false);
  assert.equal(e['back-btn'].classList.contains('hidden'), false);
  assert.equal(e['config-status'].classList.contains('hidden'), true);
  assert.equal(e['channels-status'].classList.contains('error'), true);
  assert.match(e['channels-status'].textContent, /渠道设置读取失败：数据库不可用/);

  // 离开再进来：上一次的降级提示不该残留成误导
  app.dispatchDocument('keydown', { key: 'Escape' });
  assert.equal(e['channels-status'].textContent, '');
  await openSettings(app);
  assert.match(e['channels-status'].textContent, /渠道设置读取失败/);
});

test('未保存的渠道修改在返回看板前应确认', async () => {
  const app = createHarness({ confirm: false });
  await openSettings(app);
  app.elements['ch-meow'].checked = true;
  await app.elements['back-btn'].dispatch('click');

  assert.deepEqual(app.confirmCalls, ['放弃未保存的修改并返回吗？']);
  assert.equal(app.elements['config-screen'].classList.contains('hidden'), false);
});

test('保存凭据成功不应清空用户改到一半的渠道输入', async () => {
  const app = createHarness();
  await openSettings(app);
  const e = app.elements;
  e['smtp-host'].value = 'smtp.qq.com';
  e['cookie-input'].value = `session=${'a'.repeat(50)}`;
  await e['config-form'].dispatch('submit');
  await app.flush();

  assert.equal(e['smtp-host'].value, 'smtp.qq.com');
  assert.equal(e['cookie-input'].value, '');
  // 渠道那半截修改仍在，返回时仍要确认
  await e['back-btn'].dispatch('click');
  assert.deepEqual(app.confirmCalls, ['放弃未保存的修改并返回吗？']);
});

test('设置页的通知渠道区应可访问且只把授权码当凭据藏起来', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'dist', 'index.html'), 'utf8');
  // 属性顺序不该让测试失败，故按标签取出来再查单个属性
  const tagOf = (id) => html.match(new RegExp(`<(input|button)[^>]*id="${id}"[^>]*>`))?.[0] ?? '';

  assert.match(html, /<section class="alert-block"[^>]*aria-labelledby="channels-title"/);
  assert.match(tagOf('meow-nickname-input'), /type="text"/);
  assert.match(tagOf('smtp-auth-code'), /type="password"/);
  assert.match(tagOf('save-channels-btn'), /type="button"/);
  assert.match(html, /id="channels-status"[^>]*role="status"/);
  // 昵称没有「清除」这层语义，输入框留空保存即是删除
  assert.equal(html.includes('clear-meow-nickname'), false);
});

// ═══ 统计页 ═══

function testTodayStr() {
  const d = new Date();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${m}-${day}`;
}

function testDaysAgoStr(days) {
  const d = new Date();
  d.setDate(d.getDate() - (days - 1));
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${m}-${day}`;
}

function usagePayload(totalYuan, peakDate) {
  return {
    daily: [{ date: peakDate, yuan: totalYuan, tokens: totalYuan * 1000, input_tokens: 400, output_tokens: 600 }],
    models: [],
    summary: {
      total_yuan: totalYuan, avg_yuan: totalYuan, peak_yuan: totalYuan,
      peak_date: peakDate, total_tokens: totalYuan * 1000,
      request_count: Math.round(totalYuan * 10), days_with_usage: 1,
    },
    percent_basis: 'quota',
  };
}

test('进入统计页应按需注入图表库再发起数据请求', async () => {
  const app = createHarness();
  await app.flush();
  app.elements['stats-btn'].dispatch('click'); // 勿 await：enterStats 等待图表库与数据
  await app.flush();

  assert.equal(app.elements['stats-screen'].classList.contains('hidden'), false);
  assert.equal(app.usageCalls.length, 1, '图表库就绪后应发起一次用量请求');
  assert.equal(app.invokeCalls.includes('fetch_usage_stats'), true);
});

test('快速切换区间时应丢弃过期响应，数据与最新区间一致', async () => {
  const app = createHarness();
  await app.flush();
  app.elements['stats-btn'].dispatch('click');
  await app.flush();
  assert.equal(app.usageCalls.length, 1);

  // 切到 14 天：第二个请求挂起，第一个（7 天）尚未返回
  app.presets[1].dispatch('click');
  assert.equal(app.usageCalls.length, 2);
  assert.notEqual(app.usageCalls[0].args.startDate, app.usageCalls[1].args.startDate);

  // 后发请求先返回 → 渲染 14 天数据
  app.resolveUsage(1, usagePayload(2.5, testDaysAgoStr(14)));
  await app.flush();
  assert.equal(app.elements['sum-total-yuan'].textContent, '¥2.500000');
  assert.equal(app.elements['sum-requests'].textContent, '25');
  assert.equal(app.presets[1].classList.contains('is-active'), true);
  assert.equal(app.chartStubInstances.length, 1, '趋势图应已创建');

  // 先发的 7 天响应后到 → 必须被丢弃，不得覆盖 14 天数据
  app.resolveUsage(0, usagePayload(1.0, testDaysAgoStr(7)));
  await app.flush();
  assert.equal(app.elements['sum-total-yuan'].textContent, '¥2.500000');
  assert.equal(app.elements['sum-requests'].textContent, '25');
  assert.equal(app.presets[1].classList.contains('is-active'), true);
});

test('自定义区间跨度超过 1096 天应在区间输入处报错', async () => {
  const app = createHarness();
  await app.flush();
  app.elements['stats-btn'].dispatch('click');
  await app.flush();

  app.elements['range-start'].value = '2020-01-01';
  app.elements['range-end'].value = testTodayStr();
  await app.elements['range-end'].dispatch('change');
  assert.equal(app.elements['range-error'].classList.contains('hidden'), false);
  assert.match(app.elements['range-error'].textContent, /1096/);

  // 合法区间（30 天）应清除报错
  app.elements['range-start'].value = testDaysAgoStr(30);
  app.elements['range-end'].value = testTodayStr();
  await app.elements['range-end'].dispatch('change');
  assert.equal(app.elements['range-error'].classList.contains('hidden'), true);
});
