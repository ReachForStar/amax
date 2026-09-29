---
title: 桌面安装包与前端资源
type: entity
tags: [Tauri, Windows, MSI, 发布]
created: 2026-09-29
updated: 2026-09-29
status: active
---

# 桌面安装包与前端资源

## 职责

桌面端由 Tauri 打包，`src-tauri/tauri.conf.json` 的 `build.frontendDist` 指向仓库根目录的 `dist/`，MSI 从当前源码和静态前端生成。

## 关键文件与接口

- `src-tauri/tauri.conf.json`：应用版本、前端目录和 MSI 配置。
- `src-tauri/Cargo.toml`：Rust crate 版本。
- `dist/index.html`：设置页的告警规则和通知渠道区域。
- `.github/workflows/release.yml`：标签版本检查和发布构建。

## 上下游依赖

桌面设置页的告警入口在看板右上角“设置”内；`dist/index.html` 的告警规则区域位于凭据表单之后，需要在设置页向下滚动。前端通过 Tauri IPC 调用 `get_alert_settings`、`set_alert_settings` 等后端命令。

## 重要变更记录

- 2026-09-29：工作区现存 MSI 为 0.2.3，源码配置与本机已安装程序为 0.2.5。已安装的 `amax.exe` 内可检出“告警规则”和相应命令字符串，因此不能仅凭工作区旧 MSI 推断已安装的 0.2.5 缺少告警代码。构建后需要同时核对 MSI 版本和嵌入的功能标记。
- 2026-09-29：Tauri CLI 2.11.5 首次打包在自动下载 WiX 3.14 时超时。`tauri-bundler` 源码指定 `wix314-binaries.zip` 的 SHA256 为 `6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31`，手动下载后校验一致，解压到当前用户缓存的 `tauri/WixTools314`。这是打包工具缓存，不属于项目源码。
- 2026-09-29：源码版本升级到 0.2.6。`cargo tauri build` 生成中英文 MSI 与对应 `.sig`；`scripts/verify-desktop-bundle.ps1` 已核对 `amax.exe` 的版本和关键功能标记，以及两个 MSI 内的 `ProductVersion`、`File` 表中的 `amax.exe` 版本。标签构建加 `-RequireSignatures` 检查签名文件，手动构建不强制签名。签名使用本地现有密钥和空口令生成；正式发布仍由发布工作流核对密钥标识与线上资产。
- 2026-09-29：接入 `ReachForStar/amax` 的 `master` 时发现，远端已有告警后端，但 `dist/index.html` 缺少“告警规则”和“通知渠道”区域；本地 `dist/app.js`、`dist/style.css` 和前端测试也存在未提交的配套改动。CI 从远端检出源码，故此前本地构建有功能、远端发布安装包却缺功能。发布前必须把前端配套文件与 `Cargo.lock` 一同提交。
- 2026-09-29：告警前端、对应测试、`Cargo.lock` 与 0.2.6 版本文件提交为 `9aa3dc4`；官网直连、打包校验和知识库提交为 `6ffb51c`。两个提交已推送到远端 `master`，发布源码现已包含告警界面。
