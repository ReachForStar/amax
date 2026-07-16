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

function createHarness({ config, dashboard, confirm = true, configError } = {}) {
  const ids = [
    'config-screen', 'dashboard-screen', 'config-status', 'skeleton', 'error-msg',
    'dashboard-data', 'cookie-input', 'apikey-input', 'save-btn', 'back-btn',
    'config-form', 'refresh-btn', 'settings-btn', 'last-updated', 'user-name',
    'user-requests', 'today-yuan', 'log-count', 'today-tokens', 'token-detail',
    'quota-percent', 'progress-fill', 'quota-remaining', 'quota-total', 'quota-used',
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
  const source = fs.readFileSync(path.join(__dirname, '..', 'dist', 'app.js'), 'utf8');
  vm.runInContext(source, context, { filename: 'dist/app.js' });
  return {
    elements,
    invokeCalls,
    confirmCalls,
    documentListeners,
    focused: () => focusedElement,
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
