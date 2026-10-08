---
title: 桌面安装包与前端资源
type: entity
tags: [Tauri, Windows, Linux, MSI, deb, AppImage, 发布]
created: 2026-09-29
updated: 2026-10-08
status: active
---

# 桌面安装包与前端资源

## 职责

桌面端由 Tauri 打包，`src-tauri/tauri.conf.json` 的 `build.frontendDist` 指向仓库根目录的 `dist/`，安装包从当前源码和静态前端生成。Windows 出 MSI（WiX 含 zh-CN / en-US），Linux 出 deb + AppImage（`src-tauri/tauri.linux.conf.json` 覆盖层切换打包目标）。

## 关键文件与接口

- `src-tauri/tauri.conf.json`：应用版本、前端目录、CSP、Windows MSI 配置与更新公钥。
- `src-tauri/tauri.linux.conf.json`：Linux 平台覆盖层（只覆盖 `bundle.targets: ["deb","appimage"]`，构建时自动合并）。
- `src-tauri/Cargo.toml`：Rust crate 版本；`ring` 供 Linux 凭据加密，`windows-sys` 仅在 `cfg(windows)` 依赖段。
- `dist/index.html`：设置页的告警规则和通知渠道区域。
- `.github/actions/verify/action.yml`：唯一的检查命令列表（双平台调用）。
- `.github/workflows/release.yml`：`guard` → `build-windows` + `build-linux` → `publish` 四作业；清单由 `scripts/generate-updater-manifest.mjs` 生成，Release Notes 由 `scripts/release-notes.mjs` 提取。
- `scripts/verify-desktop-bundle.ps1` / `scripts/verify-desktop-bundle.sh`：Windows / Linux 安装包核验（版本、功能标记、tag 构建的更新签名）。

## 上下游依赖

桌面设置页的告警入口在看板右上角“设置”内；`dist/index.html` 的告警规则区域位于凭据表单之后，需要在设置页向下滚动。前端通过 Tauri IPC 调用 `get_alert_settings`、`set_alert_settings` 等后端命令。

凭据存储两平台各一套：Windows `dpapi:v1:`（DPAPI），Linux `aes:v1:`（本机密钥文件 + AES-256-GCM），见 [Linux 凭据加密](../decisions/linux-credential-storage.md)。Linux 更新通道见 [Linux 发布产物与更新通道](../decisions/linux-release-and-update.md)。

## Linux 构建约束（2026-10-08 首次全量编译确认）

- 默认窗口图标必须是 RGBA：`generate_context!` 在非 Windows 目标取 `bundle.icon` 里第一个 `.png`（当前是 `icons/32x32.png`）作窗口图标，PNG 颜色类型不是 RGBA 时编译期直接 panic（`tauri-codegen` 的 `Image::new_png`，提示 `is not RGBA`）。Windows 分支走 `.ico`、从不解析 PNG，所以同一个图标在 Windows 上一直没暴露问题。改图标后用 `file` 确认 `8-bit/color RGBA`。
- `[target.'cfg(windows)'.dependencies]` 必须排在 `[dependencies]` 全部键之后：插在中间会把其后的通用依赖误归入 Windows 段（曾把 `lettre`、`url` 卷进去），Linux 编译报 unresolved crate，Windows 反而正常。
- `Db::open` 的密钥目录回退分支（`:memory:` 无父目录时退到进程专属临时目录）创建目录必须把 `io::Error` 显式映射为 `AppError::storage`：`AppError` 只实现了 `From<rusqlite::Error>`，裸 `?` 编译不过（分支两平台共用，Windows 同样失败）。
- AppImage 首次打包会从 GitHub 下载 linuxdeploy / AppRun / 插件脚本（缓存于 `~/.cache/tauri/`）；本开发机直连 GitHub 超时，构建要带本机代理（`https_proxy=http://127.0.0.1:7890 http_proxy=...`），否则报 `io: Connection reset by peer`。

## 重要变更记录

- 2026-09-29：工作区现存 MSI 为 0.2.3，源码配置与本机已安装程序为 0.2.5。已安装的 `amax.exe` 内可检出“告警规则”和相应命令字符串，因此不能仅凭工作区旧 MSI 推断已安装的 0.2.5 缺少告警代码。构建后需要同时核对 MSI 版本和嵌入的功能标记。
- 2026-09-29：Tauri CLI 2.11.5 首次打包在自动下载 WiX 3.14 时超时。`tauri-bundler` 源码指定 `wix314-binaries.zip` 的 SHA256 为 `6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31`，手动下载后校验一致，解压到当前用户缓存的 `tauri/WixTools314`。这是打包工具缓存，不属于项目源码。
- 2026-09-29：源码版本升级到 0.2.6。`cargo tauri build` 生成中英文 MSI 与对应 `.sig`；`scripts/verify-desktop-bundle.ps1` 已核对 `amax.exe` 的版本和关键功能标记，以及两个 MSI 内的 `ProductVersion`、`File` 表中的 `amax.exe` 版本。标签构建加 `-RequireSignatures` 检查签名文件，手动构建不强制签名。签名使用本地现有密钥和空口令生成；正式发布仍由发布工作流核对密钥标识与线上资产。
- 2026-09-29：接入 `ReachForStar/amax` 的 `master` 时发现，远端已有告警后端，但 `dist/index.html` 缺少“告警规则”和“通知渠道”区域；本地 `dist/app.js`、`dist/style.css` 和前端测试也存在未提交的配套改动。CI 从远端检出源码，故此前本地构建有功能、远端发布安装包却缺功能。发布前必须把前端配套文件与 `Cargo.lock` 一同提交。
- 2026-09-29：告警前端、对应测试、`Cargo.lock` 与 0.2.6 版本文件提交为 `9aa3dc4`；官网直连、打包校验和知识库提交为 `6ffb51c`。两个提交已推送到远端 `master`，发布源码现已包含告警界面。
- 2026-09-29：代码提交 `6ffb51c` 的 GitHub Windows Test 工作流 `36546013697` 已成功；本地中英文 MSI 由同一工作区源码构建，并通过版本、功能内容和签名文件核验。
- 2026-09-29：发布前核对 `master` 的本地与远端提交同为 `19660bd`，应用与 crate 版本均为 0.2.6；远端配置了 `TAURI_SIGNING_PRIVATE_KEY`，发布前尚无 `v0.2.6` 标签。当前网络直连 GitHub 超时，代理 `127.0.0.1:7890` 可连接 GitHub API，发布命令需显式使用该代理。
- 2026-09-29：已推送 `v0.2.6` 注记标签，远端标签指向 `19660bd`；GitHub Release 工作流 `36547050572` 已启动。
- 2026-09-29：`v0.2.6` 的 Release 工作流 `36547050572` 成功。公开 Release 含中英文 MSI、各自的 `.sig` 及 `latest.json`；工作流通过产物内容、签名与更新清单一致性、匿名访问校验。独立读取 `releases/latest/download/latest.json` 得到版本 0.2.6，下载 URL 指向本次中文版 MSI。
- 2026-10-08：开发机迁到 Ubuntu 22.04，完成 Linux 完整适配：桌面代码（`crypto.rs`/`db.rs`/`lib.rs`/`Cargo.toml`/`dist/app.js`）、双平台 CI 与发布链路（`release.yml` 四作业、`action.yml` Linux 依赖步骤、`test.yml` 双平台）、三个发布脚本与 `tauri.linux.conf.json`。产物名与 Windows 同规则：磁盘名空格改点后上传（deb 形如 `AMAX.Dashboard_0.2.6_amd64.deb`；更新包即原始 AppImage，签名同目录 `.AppImage.sig`，`.AppImage.tar.gz` 只在 v1 兼容模式产出）。本地构建验证待装系统依赖后进行；`cargo fmt` / 前端 66 条回归 / 脚本语法均已通过。
- 2026-10-08：仓库索引为 LF，但工作区残留 Windows 时期的 CRLF 文件；Linux 上无 autocrlf 时这些文件被 git 显示为「全文件改动」。已把工作区归一为 LF（与索引一致，无内容变化），避免提交时把整文件行尾重写带进历史。
- 2026-10-08：系统依赖装齐后首次全量编译：`cargo fmt --check`、`clippy -D warnings`、`cargo test`（Rust 126 项）与前端 66 条回归全部通过。首轮编译暴露三处问题并修复（详见「Linux 构建约束」）：`icons/32x32.png` 由调色板 PNG 转 RGBA（`compare -metric AE` 逐像素比对为 0）、`lettre`/`url` 移回 `[dependencies]`、密钥目录创建错误显式映射 `AppError::storage`。首次 `workflow_dispatch` 验证运行 37740472558 正是倒在这三处上（Windows 作业 E0277、Linux 作业 11 个错误），修复后需重新触发验证。
- 2026-10-08：本地打包链验证：`cargo tauri build` 产出 `AMAX.Dashboard_0.2.6_amd64.deb` 与 `AMAX.Dashboard_0.2.6_amd64.AppImage`（各自带 `.sig`）；`scripts/verify-desktop-bundle.sh --require-signatures` 通过（程序版本、功能标记、签名齐全）。从 `target/release/amax` 冒烟：进程存活、WebView 正常初始化（生成 WebKitCache）、数据库五张表建齐、无错误输出。`generate-updater-manifest.mjs` 用线上 v0.2.6 真实 MSI+签名与本地 Linux 产物做了端到端实测：输出正确，keyid 比对通过——同时交叉证实本机私钥与线上签名出自同一对密钥。
