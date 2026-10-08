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
- [Linux 登录窗口不显示：系统代理挂起 WebKitGTK 加载](queries/linux-webview-proxy-hang.md) — 根因、GIO resolver 直连 + 10 秒兜底修复、测试时「已登录即关窗」陷阱。
- [CI checkout 后处理警告：误提交的 worktree gitlink](queries/ci-worktree-gitlink-warning.md) — 根因（gitlink 无 .gitmodules、git 2.55 更严格）与待定的清理方式。
