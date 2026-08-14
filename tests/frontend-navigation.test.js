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
    this.dataset = {};
    this.title = '';
  }
  addEventListener(type, handler) { this.listeners.set(type, handler); }
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
} = {}) {
  const ids = [
    'config-screen', 'dashboard-screen', 'config-status', 'skeleton', 'error-msg',
    'dashboard-data', 'cookie-input', 'apikey-input', 'save-btn', 'back-btn',
    'config-form', 'refresh-btn', 'settings-btn', 'last-updated', 'user-name',
    'user-requests', 'today-yuan', 'log-count', 'today-tokens', 'token-detail',
    'quota-percent', 'progress-fill', 'quota-remaining', 'quota-total', 'quota-used',
    'stats-screen', 'stats-btn', 'stats-back-btn', 'range-start', 'range-end',
    'range-error', 'summary-title', 'trend-note', 'trend-error', 'model-error',
    'model-basis-note', 'model-list', 'remaining-block', 'export-csv-btn',
    'export-json-btn', 'export-xlsx-btn', 'trend-chart', 'requests-chart',
    'model-chart', 'remaining-chart', 'sum-total-yuan', 'sum-avg-yuan',
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
    addEventListener(type, handler) { documentListeners.set(type, handler); },
  };
  const invokeCalls = [];
  let resolveSaveConfig;
  const saveConfigPromise = pendingSaveConfig
    ? new Promise((resolve) => { resolveSaveConfig = resolve; })
    : null;
  // 统计用量请求队列：每个请求挂起，测试按需以任意顺序 resolve
  const usageCalls = [];
  const invoke = async (command, args) => {
    invokeCalls.push(command);
    if (command === 'get_config') {
      if (configError || (configErrorOnSettings && invokeCalls.filter((call) => call === 'get_config').length > 1)) {
        throw configError || '数据库不可用';
      }
      return config ?? { has_cookie: true, has_api_key: false, expired: false };
    }
    if (command === 'save_config') {
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
    if (command === 'fetch_usage_stats') {
      return new Promise((resolve) => usageCalls.push({ args, resolve }));
    }
    return undefined;
  };
  const confirmCalls = [];
  const window = {
    __TAURI__: { core: { invoke } },
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
    usageCalls,
    chartStubInstances,
    confirmCalls,
    documentListeners,
    focused: () => focusedElement,
    resolveSaveConfig: () => resolveSaveConfig?.(),
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
  const app = createHarness({ config: { has_cookie: false, has_api_key: false, expired: false } });
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
  const app = createHarness({ saveError: '保存失败' });
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
  assert.match(app.elements['config-status'].textContent, /连接失败：保存失败/);
});

test('保存成功但认证失败应恢复状态、保留输入和错误', async () => {
  const app = createHarness({ dashboardError: '401 认证失败' });
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
  assert.match(app.elements['config-status'].textContent, /Cookie 无效或已过期，请重新获取/);
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
