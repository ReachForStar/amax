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
function isAuthErrorMessage(msg) { return msg.includes('认证失败'); }

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
    showConfigError('Cookie 太短（少于 50 个字符），请确认已完整复制。\n\n获取方式：浏览器 F12 → Application → Cookies → 双击 session 的 Value 列 → Ctrl+C');
    return;
  }

  configStatus.className = 'status loading';
  configStatus.textContent = '正在验证并获取数据...';
  configStatus.classList.remove('hidden');
  setConfigBusy(true);

  try {
    await invoke('save_config', { cookie, apiKey: apikeyInput.value.trim() });
    if (cookie) hasSavedCookie = true;
    if (await loadDashboard()) {
      setConfigBusy(false);
      resetConfigForm();
    }
  } catch (e) {
    setConfigBusy(false);
    const msg = getErrorMessage(e);
    showConfigError('连接失败：' + msg);
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
      const msg = getErrorMessage(e);
      if (msg.includes('401') || msg.includes('认证失败')) {
        canReturnToDashboard = false;
        backBtn.classList.add('hidden');
        showScreen('config');
        showConfigError('Cookie 无效或已过期，请重新获取');
        setConfigBusy(false);
      } else if (msg.includes('网络') || msg.includes('timeout') || msg.includes('connect')) {
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
  } catch (e) {
    console.error('监听后台刷新事件失败:', e);
  }
}

// ═══ 统计页 ═══
const statsCharts = { trend: null, requests: null, model: null, remaining: null };
let statsUsage = null;   // fetch_usage_stats 结果
let statsLocal = null;   // get_local_stats 结果
let statsSource = 'official'; // 'official' | 'local_estimate'
let statsRange = null;   // { start_date, end_date }

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
  rangeStartInput.value = statsRange.start_date;
  rangeEndInput.value = statsRange.end_date;
}

async function enterStats() {
  if (!statsRange) {
    statsRange = { start_date: daysAgoStr(7), end_date: todayStr() };
    setActivePreset(7);
    syncRangeInputs();
  }
  showScreen('stats');
  await loadStats();
}

function applyPreset(days) {
  statsRange = { start_date: daysAgoStr(days), end_date: todayStr() };
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
  statsRange = { start_date: start, end_date: end };
  hideRangeError();
  loadStats();
}

statsBtn.addEventListener('click', enterStats);
statsBackBtn.addEventListener('click', () => { showScreen('dashboard'); statsBtn.focus(); });
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
    const authHint = isAuthErrorMessage(msg) ? '，可前往设置页重新获取 Cookie' : '';
    showBlockError(trendErrorEl, '官方数据获取失败：' + msg + authHint + '。趋势与汇总已切换为本地估算。');
    showBlockError(modelErrorEl, '官方数据不可用：' + msg + authHint);
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
    { label: 'Tokens', data: tokensData, yAxisID: 'yTokens', borderColor: '#7a8a80', borderDash: dashed ? [5, 4] : [], tension: .25, pointRadius: 2, fill: false },
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
      label: '本地 Tokens', data: labels.map((date) => localByDate.get(date)?.tokens ?? null),
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
        yTokens: { position: 'right', ticks: { color: theme.muted, font: theme.font }, grid: { drawOnChartArea: false }, title: { display: true, text: 'Tokens', color: theme.muted, font: theme.font } },
      },
      plugins: { legend: { labels: { color: theme.muted, font: theme.font, boxWidth: 14 } } },
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

// 占位实现 — Task 7（本地数据渲染）与 Task 8（导出）替换
function renderRequestsChart() {}
function renderRemainingChart() {}
function updateExportButtons() {}

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
