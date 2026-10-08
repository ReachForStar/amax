---
title: Linux 发布 deb + AppImage，仅 AppImage 走应用内自更新
type: decision
tags: [Linux, 发布, AppImage, deb, 自动更新, CI, glibc]
created: 2026-10-08
updated: 2026-10-08
status: active
---

# Linux 发布 deb + AppImage，仅 AppImage 走应用内自更新

## 背景（现状与约束）

- Windows 侧已有完整链路：MSI（WiX zh-CN/en-US）+ `tauri-plugin-updater` 应用内自更新（minisign 签名、`latest.json` 挂 GitHub Release、端点匿名可读）。
- Linux 适配需要三件事一起定：出什么安装包、更新怎么发、CI 怎么扩（原仅 `windows-latest` 单平台）。
- 技术约束：`createUpdaterArtifacts: true` 是 v2 风格——更新包就是原始安装包本身并整体签名（Linux 下 deb 与 AppImage 各得一个 `.sig`），`.AppImage.tar.gz` 只在 v1 兼容模式产出（`tauri-bundler 2.10.1` 的 `updater_bundle` 分支），v2 客户端对原始 AppImage 与 tar.gz 两种更新包都原生支持；`tauri-plugin-updater` 对 deb 安装会走 `pkexec dpkg -i` 提权替换；二进制链接的 glibc 版本构成产物的兼容下限；更新端点必须匿名可读（仓库保持 public）。

## 备选方案

1. **仅 deb**：分发规范、可被包管理器管理；但应用内自更新要经 `pkexec` 弹提权窗口，把「无人值守的空闲安装」变成交互式提权，失败面大；用户也只能手动下载升级。
2. **仅 AppImage**：单文件，官方支持的自动更新路径（替换文件本身）；但没有桌面集成（菜单项、图标需 AppImageLauncher 之类辅助），部分用户不习惯。
3. **deb + AppImage 双产物**：AppImage 走应用内自更新，deb 只提示手动升级。

## 决策（选定方案）

方案 3，各环节落点：

- **打包**：新增 `src-tauri/tauri.linux.conf.json` 平台覆盖层（构建时自动合并），只覆盖 `bundle.targets` 为 `["deb","appimage"]`；主配置不动。
- **AppImage 更新**：下载验签暂存 → 空闲安装（插件替换 AppImage 文件后返回，不退出）→ 应用从 `APPIMAGE` 路径以 `sh -c 'sleep 2; exec "$1"'` 延迟 2 秒拉起新实例，再 `app.exit(0)`。延迟是为避开单实例插件的退出竞争（立即拉起会被旧实例当成二次启动吞掉）。
- **deb 更新**：`update_is_manual()` 以 `APPIMAGE` 环境变量缺失判定 deb 安装；检查照常执行，发现新版本只发 `update://status` 的 `manual` 状态（提示到 GitHub Releases 手动下载），不下载、不暂存。
- **CI**：`test.yml` 扩成 Verify (Windows) + Verify (Linux) 两作业；`release.yml` 拆四作业——`guard`（标签与版本硬校验，ubuntu-22.04）→ `build-windows` + `build-linux`（并行）→ `publish`（合并双平台产物、生成清单、发布 Release）。构建机固定 **ubuntu-22.04**。
- **清单**：`publish` 调 `scripts/generate-updater-manifest.mjs` 生成双平台 `latest.json`——`windows-x86_64` 取 zh-CN MSI，`linux-x86_64` 取原始 AppImage（签名取同目录 `.AppImage.sig`；deb 不进更新通道，其 `.sig` 不上传）。

## 理由（决策依据）

- deb 的静默自更新必须经系统包管理器提权，与「空闲安装、不打扰用户」的产品口径冲突；而 AppImage 替换文件即完成、无需提权，是 updater 官方支持路径。两者混跑同一更新通道做不到，索性按安装形态分流：能自更新的自更新，不能的明说手动。
- 22.04 而非 24.04：产物链接的 glibc 必须 ≤ 用户机器；24.04 上构建的二进制在 22.04 用户机器上报「version GLIBC_2.x not found」直接起不来。glibc 兼容下限 = 构建机 glibc，这件事由构建机选择决定。
- `publish` 独立成作业：双平台产物必须在同一处汇合后才能生成清单并发布，否则清单 url 指向的资产在发布时点还不存在。
- webkit2gtk 等系统依赖只装在 Linux runner（composite action 内按 `runner.os` 条件执行），Windows 侧无额外负担。

## 后果（影响与后续）

- `latest.json` 平台键从 1 个变 2 个；发布 CI 的匿名探测、url/签名反查都按平台逐项执行（`gh api` 不支持 `--arg`，值须内插进 jq 表达式）。
- 新增脚本三个：`generate-updater-manifest.mjs`（清单生成 + 版本/产物名/密钥 id 校验，keyid 比对为硬门槛，CLI 自己只 `log::warn`）、`release-notes.mjs`（CHANGELOG 小节提取与 `--check`）、`verify-desktop-bundle.sh`（deb 与 AppImage 的版本、功能标记、tag 构建的签名核验，与 Windows 侧 `.ps1` 同口径）。
- 产物名规则延续 Windows 教训：磁盘名空格改点（= GitHub 资产名规则 = 清单 url）；上传资产为 MSI/.sig、deb、AppImage/.sig、latest.json。
- **更新包形态已按实测校正**：本地 `cargo tauri build` 证实 v2 风格产物是原始 AppImage + `.AppImage.sig`（另有未被使用的 `.deb.sig`，不上传；CLI 的签名包类型列表含 Deb），`.AppImage.tar.gz` 不产出。最初的脚本按 v1 风格的 `.tar.gz` 写（`verify-desktop-bundle.sh`、`generate-updater-manifest.mjs`、`release.yml` 上传/发布清单），已全部改为原始 AppImage；发布清单生成与核验都会在缺件时硬失败，不会再发出更新坏掉的版本。
- deb 用户升级路径为「应用内提示 → 手动下载覆盖安装」；`update://status` 新增 `manual` 状态（前端按钮与文案已按该状态分支，不出现「立即安装」入口）。
- 从解压目录直接运行 AppImage（无 `APPIMAGE` 环境变量）时同样落 `manual` 分支——这是有意行为，解压态无法自我替换。
- 手工 `workflow_dispatch` 只验证双平台构建链路（产物传 artifact），不创建 Release。
- 相关页面：[桌面安装包与前端资源](../entities/desktop-packaging.md)、[Linux 凭据加密](linux-credential-storage.md)。
