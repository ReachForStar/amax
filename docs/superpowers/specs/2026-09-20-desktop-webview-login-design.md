# 桌面端 WebView 登录获取 Cookie 设计

日期：2026-09-20　状态：已实施（桌面端取消 URL 快路径，原因见下）　关联：`harmony/entry/src/main/ets/pages/LoginPage.ets`（手机端同能力）

## 目标

桌面端配置页新增"使用官网登录获取 Cookie"能力：应用内打开官网登录窗口，用户登录后自动提取完整 Session Cookie 并走现有保存验证链路。获取方式与手机端对齐（手机端为 WebView 登录 + `WebCookieManager.fetchCookieSync` 自动提取，桌面端此前仅支持 F12 手动粘贴）。

范围边界：

- 不做跨设备凭据同步。桌面凭据仍经 DPAPI 绑定本机用户+机器加密存储，手机端仍用 HUKS，两端各自独立。
- 不修改存储层（`db.rs` 加密格式、`save_config`/`get_cookie` 接口）与 API 层（`api.rs`）。
- 手动粘贴保留为兜底路径。

## 关键可行性结论

Tauri 2.11.5 的 `WebviewWindow::cookies_for_url(url)` 可读取运行时 Cookie 存储中的**全部** Cookie（含 HTTP-only、Secure），即手机端 `fetchCookieSync` 的桌面等价物。无需引入 `webview2-com` 等额外依赖。

## 设计

### 1. 登录窗口（`src-tauri/src/login.rs`）

新增 IPC command `open_login_window(app)`（声明为 `async`，与 Tauri 内置 `create_webview_window` 同形态）：

- 若 `label = "login"` 窗口已存在，仅 `set_focus()` 并返回，防止重复开窗与重复轮询。
- 否则用 `WebviewWindowBuilder` 创建：URL `https://ai.amaxsmp.com/login`，1000×720，`center()`，初始 `visible(false)`（防空白闪烁），`on_page_load` 回调在 `PageLoadEvent::Finished` 时 `show()` + `set_focus()`。
- WebView2 使用系统 Chrome UA，官网无 ArkWeb 异常分支，无需像手机端那样伪装 UA。
- 窗口创建只发生在 Rust command 内；`capabilities/default.json` 的 `windows: ["main"]` 保持不变，因此 `login` 窗口不在任何 capability 内，官网页面不具备任何 IPC 权限。

### 2. 登录判定与 Cookie 提取

- **判定通道**：`open_login_window` 内 `tauri::async_runtime::spawn` 轮询任务，每 800ms（对齐 `LOGIN_POLL_INTERVAL_MS`）调用 `login_window.cookies_for_url(官网URL)`，用与手机端 `parseSessionCookie` 同口径的判定找 `session` 值。
- **不实现手机端的 URL 快路径**（设计阶段曾列为"页面加载完成时立即检查一次"，实施时取消）：Tauri 文档明确 `cookies_for_url` 在 Windows 上**于同步 command 或事件处理器中调用会死锁**（wry#583），只能在 async command 与独立线程读取；`on_page_load` 正是事件处理器。轮询 800ms 的检测延迟已满足交互需求，且官网实测不发生 `/dashboard` 重定向，快路径收益本就有限。
- **命中后**：把该 URL 返回的全部 Cookie 组装为完整请求头串 `name=value; name2=value2; …`（请求头需要完整串，手机端同此），emit 事件 `login://success`，载荷 `{ cookie }`，并关闭登录窗口。
- **看门狗**：轮询总计约 10 分钟未出现 session，emit `login://timeout`，关闭登录窗口，轮询任务退出。
- **取消**：用户在成功前关闭窗口 → `on_window_event` 收到 `CloseRequested` → emit `login://cancelled`。
- **终止与防重入**：共享 `settled: Arc<AtomicBool>`，成功/取消/超时三条路径经 `settle()` 原子抢占，保证只广播一条结果、轮询任务随即退出（取消后不再空转到超时）。
- 轮询中 `cookies_for_url` 抛错（窗口销毁竞态等瞬态错误）：本轮跳过、下轮重试，与手机端轮询容错一致。

### 3. 前端衔接（`dist/index.html` + `dist/app.js`）

- 配置页 Cookie 输入区上方新增主按钮"使用官网登录获取"；手动粘贴区降级为兜底，说明文案改为"或在浏览器中手动复制"（原 F12 指引保留为次级提示）。
- 点击 → `invoke('open_login_window')` → 按钮禁用、状态区显示"等待登录…"。
- `listen('login://success', …)`：收到 Cookie 后调用现有保存流程（等价于 `invoke('save_config', { cookie, apiKey: '' })`），复用现有"验证并获取数据"路径（含 DPAPI 加密入库与 401 回退提示）。
- `login://cancelled` / `login://timeout`：恢复按钮，状态区提示重新发起或改用手动粘贴。

### 4. 事件与权限

- 事件方向为 Rust → main 窗口，前端 `core:default` 已覆盖 `event.listen/unemit`，无需新增 capability。
- 广播经 `app_handle.emit(..)`（`login` 窗口自身无监听器，按窗口 emit 会丢失）。
- command 经现有 `invoke_handler` 注册。

## 影响范围

| 文件 | 改动 |
|---|---|
| `src-tauri/src/login.rs` | **新增**：`open_login_window` command、Cookie 组装/判定纯函数、轮询任务、三个事件广播、`login` 窗口事件处理 |
| `src-tauri/src/lib.rs` | `mod login;` + `invoke_handler` 注册 |
| `dist/index.html` | 登录按钮 + 手动粘贴降级为兜底文案 |
| `dist/app.js` | `loginBtn` 引用、`loginPending` 状态、按钮处理、三个事件监听、成功后走现有保存流程 |
| `tests/frontend-navigation.test.js` | 登录行为断言；补齐 harness 缺失的统计页 `#id` 与 `querySelectorAll`（原 harness 缺元素导致 12 个既有测试在 `dist/app.js` 演进后全部报错） |
| `CLAUDE.md` / `CHANGELOG.md` | 文档同步（IPC command 数 5→6、新增 `login.rs` 段落、事件清单） |

不改动：`db.rs`、`crypto.rs`、`api.rs`、`tauri.conf.json`、`capabilities/default.json`、API 请求头口径、手机端任何文件。

## 测试

1. Rust 纯函数单测（`login.rs` 的 `#[cfg(test)]`）：整串 Cookie 组装（`Vec<Cookie>` → 请求头串，含空数组）与 session 判定（对照 `parseSessionCookie` 口径，覆盖 session 在首/在中/缺失/空值）。
2. 前端测试（`tests/frontend-navigation.test.js`）：登录按钮位置与兜底文案、点击调用 `open_login_window` 并进入等待态、开窗失败恢复、`login://success` 保存并进看板、成功后认证失败回配置页、取消与超时恢复、非等待态事件不误触。
3. 手工验证清单（依赖真实登录与 WebView，无法自动化）：正常登录→自动保存→看板可用；中途关窗→取消提示；超时提示；已有 Cookie 时重新登录覆盖；手动粘贴路径回归。

## 验证结果

```bash
cargo fmt --all -- --check                      # 通过
cargo test --workspace                          # 28 passed（含 login.rs 5 项新增）
cargo clippy --workspace --all-targets -- -D warnings
node tests/frontend-navigation.test.js          # 21/21 通过（基线为 1/13，顺带修复）
```

`clippy -D warnings` 仍有 1 处报错，位于既有 `crypto.rs:110` 的 `chunks_exact`（新版 clippy 对未改动代码引入的新 lint，已用 `git stash` 验证在本次改动前即存在），`login.rs` 本身无告警。是否修该处属独立决策，未纳入本次范围。
