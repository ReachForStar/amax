---
type: index
updated: 2026-10-08
---

# Wiki 索引

> 人工/Agent 显式查阅用的完整目录；会话中的导航由 wiki-memory hook 注入的目录树提供。
> 格式：`[页面标题](相对路径) — 一行摘要`。

## 实体 entities

- [桌面安装包与前端资源](entities/desktop-packaging.md) — Tauri 双平台打包（MSI / deb+AppImage）与产物核验。

## 概念 concepts

## 源总结 sources

## 决策 decisions

- [Linux 凭据加密采用本机密钥文件 + AES-256-GCM](decisions/linux-credential-storage.md) — DPAPI 之外的 Unix 方案：密钥文件 + ring AES-GCM 的取舍、迁移口径与影响。
- [Linux 发布 deb + AppImage，仅 AppImage 走应用内自更新](decisions/linux-release-and-update.md) — 双产物分流（manual 状态）、ubuntu-22.04 构建与四作业发布链路。

## 查询沉淀 queries

- [桌面官网请求受系统代理影响](queries/desktop-direct-network.md) — 直连证据、客户端隔离和验证状态。
