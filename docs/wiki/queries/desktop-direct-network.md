---
title: 桌面官网请求受系统代理影响
type: query
tags: [桌面端, 网络, 代理, reqwest]
created: 2026-09-29
updated: 2026-09-29
status: active
---

# 桌面官网请求受系统代理影响

## 问题

浏览器可访问 `https://ai.amaxsmp.com/`，但桌面应用在停用代理后无法获取账户数据。

## 根因

2026-09-29 在本机读取到系统代理设置仍为 `127.0.0.1:7890`。`src-tauri/src/api.rs` 的原有 `reqwest::Client::builder()` 未禁用自动代理。禁用代理的匿名 GET `https://ai.amaxsmp.com/api/user/self` 返回 HTTP 401；同样直连的 POST `/v1/logs/token-usage/by-model` 用空 JSON 返回 HTTP 422，说明两个请求路径均已到达服务端。这些状态码分别对应缺少 Cookie 与请求体不完整。现有证据支持系统代理设置与官网数据请求发生冲突；用户原始请求的具体错误日志尚未取得。

## 解法

官网数据客户端通过 `no_proxy()` 明确直连。MeoW 投递使用独立客户端，继续遵循系统网络配置。官网请求的网络错误文案改为“官网直连失败”，避免误导用户检查代理。

## 涉及模块

- `src-tauri/src/api.rs`：构造官网和投递两个 HTTP 客户端。
- `src-tauri/src/lib.rs`：官网查询与 MeoW 投递分别使用对应客户端。

## 复发预防

发布时核对新程序中包含直连错误标记和告警功能内容。2026-09-29 的 `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace` 已通过（Rust 单元测试 121 项）。
