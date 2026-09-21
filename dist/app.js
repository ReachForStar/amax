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
const loginBtn = $('#login-btn');
const cookieExpiryEl = $('#cookie-expiry');
const saveBtn = $('#save-btn');
const configForm = $('#config-form');
const backBtn = $('#back-btn');
const settingsBtn = $('#settings-btn');
const statsScreen = $('#stats-screen');
const statsBtn = $('#stats-btn');
const statsBackBtn = $('#stats-back-btn');
const rangeStartInput = $('#range-start');
const rangeEndInput = $('#range-end');
const rangeErrorEl = $('#range-error');
const summaryTitleEl = $('#summary-title');
const trendNoteEl = $('#trend-note');
const trendErrorEl = $('#trend-error');
const modelErrorEl = $('#model-error');
const modelBasisNoteEl = $('#model-basis-note');
const modelListEl = $('#model-list');
const remainingBlock = $('#remaining-block');
const exportCsvBtn = $('#export-csv-btn');
const exportJsonBtn = $('#export-json-btn');
const exportXlsxBtn = $('#export-xlsx-btn');

let hasDashboardData = false;
let hasSavedCookie = false;
let lastUpdatedAt = null;
let updateTimeTimer = null;
let dashboardRequest = null;
let canReturnToDashboard = false;
let configBaseline = { cookie: '', apiKey: '' };
let isSavingConfig = false;
let loginPending = false;

// ═══ 工具 ═══
function formatTokens(n) {
  if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
  return String(n);
}

// Rust 侧命令以 { code, message } 结构体 reject（见 src-tauri/src/error.rs）：
// 分支判定一律用 code，message 只用于展示
function getErrorCode(error) {
  return error && typeof error === 'object' && typeof error.code === 'string'
    ? error.code
    : null;
}

function getErrorMessage(error) {
  if (typeof error === 'string') return error;
  return error?.message || error?.toString?.() || '未知错误';
}

const screenRegistry = { config: configScreen, dashboard: dashboardScreen, stats: statsScreen };
function showScreen(screen) {
  for (const [name, el] of Object.entries(screenRegistry)) {
    const hidden = name !== screen;
    el.classList.toggle('hidden', hidden);
    el.setAttribute('aria-hidden', String(hidden));
  }
}

function setConfigBusy(isBusy) {
  isSavingConfig = isBusy;
  saveBtn.disabled = isBusy;
  loginBtn.disabled = isBusy || loginPending;
  backBtn.disabled = isBusy;
  configForm.setAttribute('aria-busy', String(isBusy));
}

function setLoginPending(pending) {
  loginPending = pending;
  loginBtn.disabled = pending || isSavingConfig;
  loginBtn.textContent = pending ? '等待登录…' : '使用官网登录获取';
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

function toDateStr(date) {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}
function todayStr() { return toDateStr(new Date()); }
function daysAgoStr(days) {
  const date = new Date();
  date.setDate(date.getDate() - (days - 1)); // 含当日，7 天即今天往前 6 天
  return toDateStr(date);
}
// ═══ 配置页 ═══
// 认证失败统一指向登录按钮：一键重登复用 open_login_window 与 login:// 事件状态机
function authHint(msg) { return msg + '。请点击「使用官网登录获取」'; }

function focusRelogin(msg) {
  canReturnToDashboard = false;
  backBtn.classList.add('hidden');
  showScreen('config');
  setLoginPending(false);
  setConfigBusy(false);
  showConfigError(authHint(msg));
  loginBtn.focus();
}

// 只展示服务端真实下发的到期时间；无值时明确说明失效由官网判定，不做本地倒计时
function renderCookieExpiry(config) {
  if (!config || !config.has_cookie) {
    cookieExpiryEl.classList.add('hidden');
    cookieExpiryEl.textContent = '';
    return;
  }
  const when = config.cookie_expires_at ? new Date(config.cookie_expires_at) : null;
  cookieExpiryEl.textContent = when && !Number.isNaN(when.getTime())
    ? `凭据有效期至 ${when.toLocaleString('zh-CN')}（官网下发，仅供展示）`
    : '官网未下发过期时间，失效由服务端判定';
  cookieExpiryEl.classList.remove('hidden');
}

configForm.addEventListener('submit', async (event) => {
  event.preventDefault();
  let cookie = cookieInput.value.trim();
  if (!cookie && !hasSavedCookie) { showConfigError('请填写 Session Cookie'); return; }
  if (cookie && !cookie.toLowerCase().startsWith('session=')) {
    cookie = 'session=' + cookie;
    cookieInput.value = cookie;
  }
  if (cookie && cookie.length < 50) {
    showConfigError('Cookie 太短（少于 50 个字符），请确认已完整复制。\n\n获取方式：浏览器 F12 → Application → Cookies → 双击 session 的 Value 列 → Ctrl+C');
    return;
  }

  configStatus.className = 'status loading';
  configStatus.textContent = '正在验证并获取数据...';
  configStatus.classList.remove('hidden');
  setConfigBusy(true);

  try {
    // 手动粘贴取不到到期时间，传 null 清掉上一次登录留下的值
    await invoke('save_config', { cookie, apiKey: apikeyInput.value.trim(), expiresAt: null });
    if (cookie) hasSavedCookie = true;
    renderCookieExpiry({ has_cookie: hasSavedCookie, cookie_expires_at: null });
    if (await loadDashboard()) {
      setConfigBusy(false);
      resetConfigForm();
    }
  } catch (e) {
    setConfigBusy(false);
    const msg = getErrorMessage(e);
    showConfigError(getErrorCode(e) === 'auth' ? authHint(msg) : '连接失败：' + msg);
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
      return true;
    } catch (e) {
      const code = getErrorCode(e);
      const msg = getErrorMessage(e);
      if (code === 'auth') {
        focusRelogin(msg);
      } else if (code === 'network') {
        showError((hasDashboardData ? '刷新失败，当前显示上次数据：' : '网络连接失败：') + msg);
      } else {
        showError((hasDashboardData ? '刷新失败，当前显示上次数据：' : '获取数据失败：') + msg);
      }
      return false;
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
  canReturnToDashboard = true;
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

loginBtn.addEventListener('click', async () => {
  configStatus.classList.add('hidden');
  setLoginPending(true);
  configStatus.className = 'status loading';
  configStatus.textContent = '已打开官网登录窗口，请在窗口中完成登录…';
  configStatus.classList.remove('hidden');
  try {
    await invoke('open_login_window');
  } catch (e) {
    setLoginPending(false);
    showConfigError('无法打开登录窗口：' + getErrorMessage(e));
  }
});

async function openSettings() {
  try {
    const cfg = await invoke('get_config');
    resetConfigForm();
    renderCookieExpiry(cfg);
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

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && !configScreen.classList.contains('hidden')) {
    leaveSettings();
  }
});

// ═══ 后台刷新事件 ═══
async function setupEventListener() {
  if (!listen || window._unlistenDashboard) return;

  try {
    window._unlistenDashboard = await listen('dashboard-updated', (event) => {
      if (event.payload && hasDashboardData) renderDashboard(event.payload);
    });

    // 官网 WebView 登录：Rust 侧提取 Cookie 成功后，走现有保存验证链路
    window._unlistenLoginSuccess = await listen('login://success', async (event) => {
      if (!loginPending) return;
      const payload = event.payload || {};
      const cookie = payload.cookie;
      if (!cookie) { setLoginPending(false); showConfigError('登录成功但未取到 Cookie，请重试或改用手动粘贴'); return; }
      configStatus.className = 'status loading';
      configStatus.textContent = '已获取 Cookie，正在验证并获取数据...';
      setConfigBusy(true);
      try {
        await invoke('save_config', {
          cookie,
          apiKey: apikeyInput.value.trim(),
          // 官网未下发 Expires 时为 null，配置页显示"未提供"
          expiresAt: payload.expires_at || null,
        });
        hasSavedCookie = true;
        setLoginPending(false);
        renderCookieExpiry({ has_cookie: true, cookie_expires_at: payload.expires_at || null });
        if (await loadDashboard()) { setConfigBusy(false); resetConfigForm(); }
      } catch (e) {
        setConfigBusy(false);
        setLoginPending(false);
        const msg = getErrorMessage(e);
        showConfigError(getErrorCode(e) === 'auth' ? authHint(msg) : '连接失败：' + msg);
      }
    });

    window._unlistenLoginCancelled = await listen('login://cancelled', () => {
      if (!loginPending) return;
      setLoginPending(false);
      configStatus.className = 'status';
      configStatus.textContent = '已取消登录。可重新点击上方按钮，或改为手动粘贴。';
      configStatus.classList.remove('hidden');
    });

    window._unlistenLoginTimeout = await listen('login://timeout', () => {
      if (!loginPending) return;
      setLoginPending(false);
      showConfigError('登录超时，请重新发起或改为手动粘贴。');
    });

    // 定时/托盘后台刷新撞上凭据失效：前端否则无从得知，看板会长期停在陈旧数据
    window._unlistenAuthExpired = await listen('auth://expired', () => {
      if (loginPending || isSavingConfig) return;
      if (!configScreen.classList.contains('hidden')) return; // 已在配置页，保留更具体的状态
      hasSavedCookie = false;
      focusRelogin('登录状态已失效');
    });
  } catch (e) {
    console.error('监听后台刷新事件失败:', e);
  }
}

// ═══ 统计页 ═══
const statsCharts = { trend: null, requests: null, model: null, remaining: null };
let statsUsage = null;   // fetch_usage_stats 结果
let statsLocal = null;   // get_local_stats 结果
let statsSource = 'official'; // 'official' | 'local_estimate'
let statsRange = null;   // { startDate, endDate }，内部状态用驼峰键，invoke 入参须为 camelCase

function chartTheme() {
  return {
    font: { family: '"Cascadia Mono", "SFMono-Regular", Consolas, monospace', size: 10 },
    muted: '#5a6460',
    grid: 'rgba(190, 201, 193, .5)',
    accent: '#2c6b4b',
  };
}

function destroyStatsCharts() {
  for (const key of Object.keys(statsCharts)) {
    if (statsCharts[key]) { statsCharts[key].destroy(); statsCharts[key] = null; }
  }
}

function showRangeError(msg) { rangeErrorEl.textContent = msg; rangeErrorEl.classList.remove('hidden'); }
function hideRangeError() { rangeErrorEl.classList.add('hidden'); }
function showBlockError(el, msg) { el.textContent = msg; el.classList.remove('hidden'); }
function hideBlockError(el) { el.classList.add('hidden'); }

function setActivePreset(daysOrNull) {
  document.querySelectorAll('.range-preset').forEach((btn) => {
    btn.classList.toggle('is-active', daysOrNull !== null && Number(btn.dataset.days) === daysOrNull);
  });
}

function syncRangeInputs() {
  const today = todayStr();
  rangeStartInput.max = today;
  rangeEndInput.max = today;
  rangeStartInput.value = statsRange.startDate;
  rangeEndInput.value = statsRange.endDate;
}

async function enterStats() {
  if (!statsRange) {
    statsRange = { startDate: daysAgoStr(7), endDate: todayStr() };
    setActivePreset(7);
    syncRangeInputs();
  }
  showScreen('stats');
  await loadStats();
}

function applyPreset(days) {
  statsRange = { startDate: daysAgoStr(days), endDate: todayStr() };
  setActivePreset(days);
  syncRangeInputs();
  hideRangeError();
  loadStats();
}

function applyCustomRange() {
  const start = rangeStartInput.value;
  const end = rangeEndInput.value;
  if (!start || !end) return;
  setActivePreset(null);
  if (start > end) { showRangeError('起始日期不能晚于结束日期'); return; }
  if (end > todayStr()) { showRangeError('结束日期不能超过今天'); return; }
  statsRange = { startDate: start, endDate: end };
  hideRangeError();
  loadStats();
}

statsBtn.addEventListener('click', enterStats);
statsBackBtn.addEventListener('click', () => {
  destroyStatsCharts();
  showScreen('dashboard');
  statsBtn.focus();
});
document.querySelectorAll('.range-preset').forEach((btn) =>
  btn.addEventListener('click', () => applyPreset(Number(btn.dataset.days))));
rangeStartInput.addEventListener('change', applyCustomRange);
rangeEndInput.addEventListener('change', applyCustomRange);

async function loadStats() {
  const [usageResult, localResult] = await Promise.allSettled([
    invoke('fetch_usage_stats', statsRange),
    invoke('get_local_stats', statsRange),
  ]);

  statsLocal = localResult.status === 'fulfilled' ? localResult.value : null;

  if (usageResult.status === 'fulfilled') {
    statsUsage = usageResult.value;
    statsSource = 'official';
    hideBlockError(trendErrorEl);
    hideBlockError(modelErrorEl);
    renderSummary(statsUsage.summary, 'official');
    renderModelBlock(statsUsage);
  } else {
    statsUsage = null;
    statsSource = 'local_estimate';
    const msg = getErrorMessage(usageResult.reason);
    const authHintText = getErrorCode(usageResult.reason) === 'auth'
      ? '，可在设置页点「使用官网登录获取」重新登录'
      : '';
    showBlockError(trendErrorEl, '官方数据获取失败：' + msg + authHintText + '。趋势与汇总已切换为本地估算。');
    showBlockError(modelErrorEl, '官方数据不可用：' + msg + authHintText);
    renderSummary(statsLocal ? statsLocal.summary : null, 'local_estimate');
    renderModelBlock(null);
  }

  renderTrendChart();
  renderRequestsChart();
  renderRemainingChart();
  updateExportButtons();
}

function renderSummary(summary, source) {
  summaryTitleEl.textContent = source === 'local_estimate'
    ? 'RANGE SUMMARY（本地估算）'
    : 'RANGE SUMMARY';
  const set = (id, text) => { $(id).textContent = text; };
  if (!summary) {
    ['#sum-total-yuan', '#sum-avg-yuan', '#sum-peak', '#sum-days', '#sum-requests']
      .forEach((id) => set(id, '--'));
    return;
  }
  set('#sum-total-yuan', '¥' + summary.total_yuan.toFixed(6));
  set('#sum-avg-yuan', '¥' + summary.avg_yuan.toFixed(6));
  set('#sum-peak', summary.peak_date
    ? '¥' + summary.peak_yuan.toFixed(6) + '（' + summary.peak_date + '）'
    : '--');
  const days = source === 'official' ? summary.days_with_usage : summary.days_with_data;
  set('#sum-days', String(days));
  set('#sum-requests', summary.request_count != null ? String(summary.request_count) : '--');
}

function renderTrendChart() {
  const theme = chartTheme();
  if (statsCharts.trend) { statsCharts.trend.destroy(); statsCharts.trend = null; }

  let labels = [];
  let yuanData = [];
  let tokensData = [];
  let dashed = false;

  if (statsUsage) {
    labels = statsUsage.daily.map((d) => d.date);
    yuanData = statsUsage.daily.map((d) => d.yuan);
    tokensData = statsUsage.daily.map((d) => d.tokens);
    trendNoteEl.textContent = '';
  } else if (statsLocal && statsLocal.daily.length) {
    labels = statsLocal.daily.map((d) => d.date);
    yuanData = statsLocal.daily.map((d) => d.yuan);
    tokensData = statsLocal.daily.map((d) => d.tokens);
    dashed = true;
    trendNoteEl.textContent = '本地估算（快照日末累计值）';
  } else {
    trendNoteEl.textContent = '暂无数据';
  }

  const datasets = [
    { label: '费用 ¥', data: yuanData, yAxisID: 'yYuan', borderColor: theme.accent, borderDash: dashed ? [5, 4] : [], tension: .25, pointRadius: 2, fill: false },
    { label: 'Tokens (M)', data: tokensData, yAxisID: 'yTokens', borderColor: '#7a8a80', borderDash: dashed ? [5, 4] : [], tension: .25, pointRadius: 2, fill: false },
  ];

  // 官方模式下叠加本地对比数据集（默认隐藏，图例点击开启）
  if (statsUsage && statsLocal && statsLocal.daily.length) {
    const localByDate = new Map(statsLocal.daily.map((d) => [d.date, d]));
    datasets.push({
      label: '本地费用 ¥', data: labels.map((date) => localByDate.get(date)?.yuan ?? null),
      yAxisID: 'yYuan', borderColor: 'rgba(44, 107, 75, .55)', borderDash: [3, 3],
      tension: .25, pointRadius: 1, hidden: true, spanGaps: true,
    });
    datasets.push({
      label: '本地 Tokens (M)', data: labels.map((date) => localByDate.get(date)?.tokens ?? null),
      yAxisID: 'yTokens', borderColor: 'rgba(122, 138, 128, .55)', borderDash: [3, 3],
      tension: .25, pointRadius: 1, hidden: true, spanGaps: true,
    });
  }

  statsCharts.trend = new Chart($('#trend-chart'), {
    type: 'line',
    data: { labels, datasets },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      interaction: { mode: 'index', intersect: false },
      scales: {
        x: { ticks: { color: theme.muted, font: theme.font, maxTicksLimit: 8, maxRotation: 0 }, grid: { color: theme.grid } },
        yYuan: { position: 'left', ticks: { color: theme.muted, font: theme.font }, grid: { color: theme.grid }, title: { display: true, text: '¥', color: theme.muted, font: theme.font } },
        yTokens: { position: 'right', ticks: { color: theme.muted, font: theme.font, callback: (value) => (value / 1e6).toFixed(1) + 'M' }, grid: { drawOnChartArea: false }, title: { display: true, text: 'Tokens (M)', color: theme.muted, font: theme.font } },
      },
      plugins: {
        legend: { labels: { color: theme.muted, font: theme.font, boxWidth: 14 } },
        // 仅显示层换算：Tokens 轴以百万（M）为单位，费用轴保持原始数值；导出走 buildCsv/buildExportData，不受影响
        tooltip: {
          callbacks: {
            label: (item) => item.dataset.yAxisID === 'yTokens'
              ? `${item.dataset.label}: ${(item.parsed.y / 1e6).toFixed(2)}M`
              : `${item.dataset.label}: ${item.parsed.y != null ? item.parsed.y.toFixed(6) : '--'}`,
          },
        },
      },
    },
  });
}

const MODEL_PALETTE = ['#2c6b4b', '#7a8a80', '#b08d57', '#5a6460', '#a3544f', '#4f6d7a', '#8b3e3e', '#6b7d5e'];

function renderModelBlock(usage) {
  if (statsCharts.model) { statsCharts.model.destroy(); statsCharts.model = null; }
  modelListEl.innerHTML = '';

  if (!usage || !usage.models.length) {
    modelBasisNoteEl.textContent = usage ? '暂无模型数据' : '';
    return;
  }

  modelBasisNoteEl.textContent = usage.percent_basis === 'tokens'
    ? '占比口径：Tokens（quota 全为 0）'
    : '占比口径：quota';

  const basisValue = (m) => (usage.percent_basis === 'tokens' ? m.tokens : m.yuan);

  statsCharts.model = new Chart($('#model-chart'), {
    type: 'doughnut',
    data: {
      labels: usage.models.map((m) => m.model),
      datasets: [{
        data: usage.models.map(basisValue),
        backgroundColor: usage.models.map((_, i) => MODEL_PALETTE[i % MODEL_PALETTE.length]),
        borderColor: '#e8ece8',
        borderWidth: 1,
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      cutout: '62%',
      plugins: {
        legend: { display: false },
        tooltip: {
          callbacks: {
            label: (item) => {
              const m = usage.models[item.dataIndex];
              return `${m.model}: ${m.percent.toFixed(1)}%`;
            },
          },
        },
      },
    },
  });

  usage.models.forEach((m) => {
    const li = document.createElement('li');
    const name = document.createElement('span');
    name.className = 'model-name';
    name.textContent = m.model;
    name.title = m.model;
    const nums = document.createElement('span');
    nums.className = 'model-nums';
    nums.textContent = '¥' + m.yuan.toFixed(4) + ' / ' + m.percent.toFixed(1) + '% / ' + m.request_count + ' 次';
    li.append(name, nums);
    modelListEl.appendChild(li);
  });
}

function renderRequestsChart() {
  const theme = chartTheme();
  if (statsCharts.requests) { statsCharts.requests.destroy(); statsCharts.requests = null; }

  const rows = (statsLocal && statsLocal.daily) || [];
  // 无任何有效差分（全为 null，如全新安装或老数据）时整块隐藏
  const block = $('#requests-chart').closest('.stats-block');
  if (!rows.some((d) => d.new_requests != null)) {
    block.classList.add('hidden');
    return;
  }
  block.classList.remove('hidden');

  statsCharts.requests = new Chart($('#requests-chart'), {
    type: 'line',
    data: {
      labels: rows.map((d) => d.date),
      datasets: [{
        label: '新增请求数',
        data: rows.map((d) => d.new_requests), // null 点自动留空
        borderColor: theme.accent,
        tension: .25,
        pointRadius: 2,
        fill: false,
        spanGaps: false, // 缺档日断开，避免误导
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      scales: {
        x: { ticks: { color: theme.muted, font: theme.font, maxTicksLimit: 8, maxRotation: 0 }, grid: { color: theme.grid } },
        y: { beginAtZero: true, ticks: { color: theme.muted, font: theme.font, precision: 0 }, grid: { color: theme.grid } },
      },
      plugins: { legend: { display: false } },
    },
  });
}

function renderRemainingChart() {
  const theme = chartTheme();
  if (statsCharts.remaining) { statsCharts.remaining.destroy(); statsCharts.remaining = null; }

  const rows = (statsLocal && statsLocal.daily) || [];
  if (!rows.length) {
    remainingBlock.classList.add('hidden');
    return;
  }
  remainingBlock.classList.remove('hidden');

  statsCharts.remaining = new Chart($('#remaining-chart'), {
    type: 'line',
    data: {
      labels: rows.map((d) => d.date),
      datasets: [{
        label: '剩余额度 ¥',
        data: rows.map((d) => d.remaining),
        borderColor: '#b08d57',
        tension: .25,
        pointRadius: 2,
        fill: false,
        spanGaps: true, // 余额为状态量，缺口连接
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      scales: {
        x: { ticks: { color: theme.muted, font: theme.font, maxTicksLimit: 8, maxRotation: 0 }, grid: { color: theme.grid } },
        y: { ticks: { color: theme.muted, font: theme.font }, grid: { color: theme.grid } },
      },
      plugins: { legend: { display: false } },
    },
  });
}

// ═══ 数据导出（CSV / JSON / XLSX） ═══

function updateExportButtons() {
  const hasData = Boolean(statsUsage || (statsLocal && statsLocal.daily.length));
  [exportCsvBtn, exportJsonBtn, exportXlsxBtn].forEach((btn) => { btn.disabled = !hasData; });
}

// 汇总官方与本地数据为统一导出结构
function buildExportData() {
  const official = statsSource === 'official' && statsUsage;
  const localByDate = new Map((statsLocal && statsLocal.daily || []).map((d) => [d.date, d]));
  const baseDaily = official ? statsUsage.daily : (statsLocal && statsLocal.daily || []);
  return {
    app: 'amax-dashboard',
    exported_at: new Date().toISOString(),
    // 导出格式遵循 spec 保持 snake_case，不随内部驼峰状态改变
    range: { start_date: statsRange.startDate, end_date: statsRange.endDate },
    source: statsSource,
    percent_basis: official ? statsUsage.percent_basis : null,
    summary: official ? statsUsage.summary : (statsLocal ? statsLocal.summary : null),
    daily: baseDaily.map((d) => ({
      date: d.date,
      yuan: d.yuan,
      tokens: d.tokens,
      input_tokens: official ? d.input_tokens : null,
      output_tokens: official ? d.output_tokens : null,
      new_requests: localByDate.has(d.date) ? localByDate.get(d.date).new_requests : null,
      remaining: localByDate.has(d.date) ? localByDate.get(d.date).remaining : null,
    })),
    models: official ? statsUsage.models : [],
  };
}

function exportFileName(ext) {
  return `amax-stats_${statsRange.startDate}_to_${statsRange.endDate}.${ext}`;
}

function downloadBlob(blob, filename) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function csvEscape(value) {
  const text = value == null ? '' : String(value);
  return /[",\n\r]/.test(text) ? '"' + text.replace(/"/g, '""') + '"' : text;
}

function buildCsv(data) {
  const lines = [];
  lines.push('[区间汇总]');
  lines.push('指标,值');
  if (data.summary) {
    const s = data.summary;
    lines.push(`累计费用(元),${csvEscape(s.total_yuan)}`);
    lines.push(`日均费用(元),${csvEscape(s.avg_yuan)}`);
    lines.push(`单日峰值(元),${csvEscape(s.peak_yuan)}`);
    lines.push(`峰值日期,${csvEscape(s.peak_date)}`);
    lines.push(`累计Tokens,${csvEscape(s.total_tokens)}`);
    if (s.request_count != null) lines.push(`区间总请求数,${csvEscape(s.request_count)}`);
  }
  lines.push(`数据来源,${data.source}`);
  lines.push('');
  lines.push('[每日趋势]');
  lines.push('date,yuan,tokens,input_tokens,output_tokens,new_requests,remaining');
  data.daily.forEach((d) => lines.push(
    [d.date, d.yuan, d.tokens, d.input_tokens, d.output_tokens, d.new_requests, d.remaining]
      .map(csvEscape).join(',')));
  lines.push('');
  lines.push('[模型分布]');
  lines.push('model,yuan,tokens,request_count,percent,percent_basis');
  data.models.forEach((m) => lines.push(
    [m.model, m.yuan, m.tokens, m.request_count, m.percent, data.percent_basis]
      .map(csvEscape).join(',')));
  // BOM（U+FEFF）防 Excel 中文乱码；用 fromCharCode 避免源码中不可见字符
  return String.fromCharCode(0xFEFF) + lines.join('\r\n');
}

exportCsvBtn.addEventListener('click', () => {
  const data = buildExportData();
  downloadBlob(new Blob([buildCsv(data)], { type: 'text/csv;charset=utf-8' }), exportFileName('csv'));
});

exportJsonBtn.addEventListener('click', () => {
  const data = buildExportData();
  downloadBlob(
    new Blob([JSON.stringify(data, null, 2)], { type: 'application/json;charset=utf-8' }),
    exportFileName('json'));
});

exportXlsxBtn.addEventListener('click', () => {
  const data = buildExportData();
  const wb = XLSX.utils.book_new();

  const summaryRows = [['指标', '值']];
  if (data.summary) {
    const s = data.summary;
    summaryRows.push(['累计费用(元)', s.total_yuan], ['日均费用(元)', s.avg_yuan],
      ['单日峰值(元)', s.peak_yuan], ['峰值日期', s.peak_date], ['累计Tokens', s.total_tokens]);
    if (s.request_count != null) summaryRows.push(['区间总请求数', s.request_count]);
  }
  summaryRows.push(['数据来源', data.source], ['区间起', data.range.start_date], ['区间止', data.range.end_date]);
  XLSX.utils.book_append_sheet(wb, XLSX.utils.aoa_to_sheet(summaryRows), '区间汇总');

  XLSX.utils.book_append_sheet(wb, XLSX.utils.json_to_sheet(
    data.daily.length ? data.daily : [{ date: '', yuan: '', tokens: '' }]), '每日趋势');

  XLSX.utils.book_append_sheet(wb, XLSX.utils.json_to_sheet(
    data.models.length ? data.models : [{ model: '', yuan: '', tokens: '', request_count: '', percent: '' }]), '模型分布');

  XLSX.writeFile(wb, exportFileName('xlsx'));
});

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
    canReturnToDashboard = false;
    backBtn.classList.add('hidden');
    hasSavedCookie = cfg.has_cookie;
    renderCookieExpiry(cfg);
    if (cfg.has_cookie) {
      await loadDashboard();
    } else {
      /* 首次使用 */
      showScreen('config');
    }
  } catch (e) {
    showScreen('config');
    showConfigError('初始化失败：' + getErrorMessage(e));
  }
}

init();
