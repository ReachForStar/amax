---
title: Linux 登录窗口不显示：系统代理挂起 WebKitGTK 加载
type: query
tags: [桌面端, Linux, WebKitGTK, 代理, 登录窗口]
created: 2026-10-08
updated: 2026-10-08
status: active
---

# Linux 登录窗口不显示：系统代理挂起 WebKitGTK 加载

## 问题

Linux deb 安装版点击「官网登录」后主窗口无反应，登录窗口不出现。Windows 版不复现。

## 根因

GNOME 系统代理（manual，127.0.0.1:7890）被 GLib 的 `GProxyResolverGnome` 读取后，WebKitGTK 经该代理连接 `ai.amaxsmp.com` 时挂起：`GSocketClient: Starting proxy connection` 反复出现、无后续。`curl` 直连与走该代理都能 200，问题出在 WebKit 与本地代理的交互，不是代理不可用。

导航因此停在 provisional 阶段，`on_page_load` 连 `Started` 都不触发；登录窗口以 `visible(false)` 创建、靠加载完成回调才 `show()`，于是形成「点了按钮窗口不出现」的静默失败。

主窗口不受影响：`tauri://localhost` 自定义协议不走网络。Windows 的 WebView2 走 WinINET 代理解析，路径不同，不复现。

对照实验（2026-10-08，本机）：

| 环境 | resolver 实现 | 结果 |
| --- | --- | --- |
| 真实环境（GNOME manual 代理） | GProxyResolverGnome | 挂起：无 Started/Finished，窗口始终 `visible=false` |
| `GSETTINGS_BACKEND=memory`（等价无代理） | - | 立即 Started + Finished，窗口显示 |
| `GIO_USE_PROXY_RESOLVER=dummy` | GDummyProxyResolver | 加载正常，窗口显示 |

## 解法

1. `lib.rs::run()` 最前处：Linux 下 `GIO_USE_PROXY_RESOLVER` 未被显式设置时设为 `dummy`，WebKitGTK 全进程直连，与 `api.rs` 官网客户端的 `no_proxy()` 直连策略一致；用户显式设置（如切回 `gnome`）不被覆盖。`run()` 由 `main` 最先调用、进程单线程，此处的 `set_var` 无并发读取。
2. `login.rs::open_login_window` 兜底：创建后 10 秒（`DISPLAY_FALLBACK`）窗口若仍隐藏则强制 `show()`，覆盖「直连也挂起」的残余场景（如路由黑洞）；正常加载 1-2 秒完成，兜底不介入；窗口已关闭时 `is_visible()` 非 `Ok(false)`，不会误弹。

## 验证

隔离数据目录（`XDG_DATA_HOME` 指向 tmp，不碰真实用户数据）下：

- 真实环境正常加载：t=1s 窗口 `visible=true`、URL 为登录页。
- 挂起场景（临时把 `LOGIN_URL` 改为 `https://192.0.2.1/login`，RFC 5737 文档网段，连接必然挂起；验证后还原）：t=1~9s `visible=false`，t=10s 兜底触发 `visible=true`。
- 站点成功路径完整跑通：站点在登录页加载后自动下发 session Cookie → 轮询判成功 → 前端保存加密 Cookie 与 `expires_at` → 关窗。

## 测试陷阱

本机数据目录的 WebKitGTK Cookie 存储（`~/.local/share/<identifier>/cookies`，Netscape 文本格式）已有有效 session，或站点在登录页加载后自动签发 session（本机约 15-20 秒）时，轮询任务会在首个 tick（立即执行）判成功并 `close()` 窗口，表现为「窗口创建后 1 秒内消失」「`webview_windows()` 里没有 login」「`is_visible()` 返回 `Err(Runtime(FailedToReceiveMessage))`」。这是成功路径而非缺陷；排查时先查 cookie 存储与 DB `config.cookie_expires_at` 是否被前端写入。观察窗口显示链路请用隔离 `XDG_DATA_HOME`（无历史 Cookie）。

区分两种查询结果：窗口存在但隐藏是 `Ok(false)`；窗口已销毁时主线程找不到窗口、消息发送端被丢弃，`is_visible()` 返回 `Err(Runtime(FailedToReceiveMessage))`。

## 涉及模块

- `src-tauri/src/login.rs`：登录窗口创建、轮询判成功、兜底显示。
- `src-tauri/src/lib.rs`：`run()` 的 resolver 覆盖。
- [桌面官网请求受系统代理影响](desktop-direct-network.md)：同一网络问题的 API 客户端侧表现与处理。

## 复发预防

- 改登录窗口行为后按「隔离数据目录 + 黑洞地址」两场景回归（见上）。
- 若 WebKitGTK 版本变化影响 resolver 覆盖，第一现象仍是「窗口不显示」——用 `G_MESSAGES_DEBUG=all` 看是否停在 `Starting proxy connection`。
