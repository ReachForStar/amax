// ═══ Tauri IPC 封装 ═══
const invoke = (() => {
  if (window.__TAURI__?.core?.invoke) {
    return (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
  }
  if (window.__TAURI_INTERNALS__?.invoke) {
    return (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
  }
  if (window.__TAURI__?.invoke) {
    return (cmd, args) => window.__TAURI__.invoke(cmd, args);
  }
  return null;
})();

// Tauri 事件监听
const listen = window.__TAURI__?.event?.listen
  || window.__TAURI__?.core?.event?.listen;

// ═══ DOM ═══
const $ = (sel) => document.querySelector(sel);
const configScreen = $('#config-screen');
const dashboardScreen = $('#dashboard-screen');
const configStatus = $('#config-status');
const skeleton = $('#skeleton');
const errorEl = $('#error-msg');
const dashboardData = $('#dashboard-data');
const cookieInput = $('#cookie-input');
const apikeyInput = $('#apikey-input');
const saveBtn = $('#save-btn');
const configForm = $('#config-form');
const backBtn = $('#back-btn');
const settingsBtn = $('#settings-btn');

let hasDashboardData = false;
let hasSavedCookie = false;
let lastUpdatedAt = null;
let updateTimeTimer = null;
let dashboardRequest = null;
let canReturnToDashboard = false;
let configBaseline = { cookie: '', apiKey: '' };
let isSavingConfig = false;

// ═══ 工具 ═══
function formatTokens(n) {
  if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
  return String(n);
}

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

function showSkeleton() {
  skeleton.classList.remove('hidden');
  dashboardData.classList.add('hidden');
  errorEl.classList.add('hidden');
}

function showError(msg) {
  errorEl.textContent = msg;
  errorEl.classList.remove('hidden');
  skeleton.classList.add('hidden');
  if (!hasDashboardData) dashboardData.classList.add('hidden');
}

function formatRelativeTime(date) {
  const seconds = Math.max(0, Math.floor((Date.now() - date.getTime()) / 1000));
  if (seconds < 60) return '刚刚更新';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  return `${Math.floor(hours / 24)} 天前`;
}

function updateLastUpdated() {
  const el = $('#last-updated');
  if (!lastUpdatedAt) {
    el.textContent = '等待同步';
    el.removeAttribute('title');
    return;
  }
  el.textContent = formatRelativeTime(lastUpdatedAt);
  el.title = '最后更新: ' + lastUpdatedAt.toLocaleString('zh-CN');
}

function markUpdated() {
  lastUpdatedAt = new Date();
  updateLastUpdated();
  if (!updateTimeTimer) {
    updateTimeTimer = window.setInterval(updateLastUpdated, 30_000);
  }
}

function showConfigError(msg) {
  configStatus.className = 'status error';
  configStatus.textContent = msg;
  configStatus.classList.remove('hidden');
}

// ═══ 配置页 ═══
configForm.addEventListener('submit', async (event) => {
  event.preventDefault();
  let cookie = cookieInput.value.trim();
  if (!cookie && !hasSavedCookie) { showConfigError('请填写 Session Cookie'); return; }
  if (cookie && !cookie.toLowerCase().startsWith('session=')) {
    cookie = 'session=' + cookie;
    cookieInput.value = cookie;
  }
  if (cookie && cookie.length < 50) {
    showConfigError('Cookie 太短 (< 50 字符), 请确认已完整复制。\n\n获取方式: 浏览器 F12 → Application → Cookies → 双击 session 的 Value 列 → Ctrl+C');
    return;
  }

  configStatus.className = 'status loading';
  configStatus.textContent = '正在验证并获取数据...';
  configStatus.classList.remove('hidden');
  setConfigBusy(true);

  try {
    await invoke('save_config', { cookie, apiKey: apikeyInput.value.trim() });
    if (cookie) hasSavedCookie = true;
    await loadDashboard();
  } catch (e) {
    setConfigBusy(false);
    const msg = getErrorMessage(e);
    showConfigError('连接失败: ' + msg);
  }
});

// ═══ 看板 ═══
async function loadDashboard() {
  if (dashboardRequest) return dashboardRequest;

  dashboardRequest = (async () => {
    showScreen('dashboard');
    if (!hasDashboardData) showSkeleton();
    errorEl.classList.add('hidden');
    const refreshBtn = $('#refresh-btn');
    refreshBtn.classList.add('is-loading');
    refreshBtn.disabled = true;

    try {
      const data = await invoke('fetch_dashboard');
      renderDashboard(data);
    } catch (e) {
      const msg = getErrorMessage(e);
      if (msg.includes('401') || msg.includes('认证失败')) {
        showScreen('config');
        showConfigError('Cookie 无效或已过期, 请重新获取');
        setConfigBusy(false);
      } else if (msg.includes('网络') || msg.includes('timeout') || msg.includes('connect')) {
        showError((hasDashboardData ? '刷新失败，当前显示上次数据：' : '网络连接失败：') + msg);
      } else {
        showError((hasDashboardData ? '刷新失败，当前显示上次数据：' : '获取数据失败：') + msg);
      }
    } finally {
      refreshBtn.classList.remove('is-loading');
      refreshBtn.disabled = false;
    }
  })();

  try {
    return await dashboardRequest;
  } finally {
    dashboardRequest = null;
  }
}

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

// ═══ 按钮 ═══
$('#refresh-btn').addEventListener('click', () => loadDashboard());
backBtn.addEventListener('click', leaveSettings);

settingsBtn.addEventListener('click', async () => {
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
    canReturnToDashboard = hasDashboardData;
    backBtn.classList.toggle('hidden', !canReturnToDashboard);
  } catch (_) {}
  showScreen('config');
});

// ═══ 后台刷新事件 ═══
async function setupEventListener() {
  if (!listen || window._unlistenDashboard) return;

  try {
    window._unlistenDashboard = await listen('dashboard-updated', (event) => {
      if (event.payload) renderDashboard(event.payload);
    });
  } catch (e) {
    console.error('监听后台刷新事件失败:', e);
  }
}

// ═══ 启动 ═══
async function init() {
  if (!invoke) {
    document.body.innerHTML =
      '<div style="display:flex;align-items:center;justify-content:center;height:100vh;color:#888;font-family:sans-serif">请在 Tauri 环境中运行此应用</div>';
    return;
  }

  await setupEventListener();

  try {
    const cfg = await invoke('get_config');
    hasSavedCookie = cfg.has_cookie;
    if (cfg.has_cookie && !cfg.expired) {
      await loadDashboard();
    } else {
      if (!cfg.has_cookie) { /* 首次使用 */ }
      else if (cfg.expired) { showConfigError('Cookie 已过期（超过 15 天），请重新获取'); }
      showScreen('config');
    }
  } catch (e) {
    showScreen('config');
    showConfigError('初始化失败: ' + (e?.message || e?.toString?.() || e));
  }
}

init();
