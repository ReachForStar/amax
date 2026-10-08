---
title: Linux 凭据加密采用本机密钥文件 + AES-256-GCM
type: decision
tags: [Linux, 凭据, 加密, DPAPI, AES-GCM, ring]
created: 2026-10-08
updated: 2026-10-08
status: active
---

# Linux 凭据加密采用本机密钥文件 + AES-256-GCM

## 背景（现状与约束）

- 应用原为 Windows 专属：`src-tauri/src/crypto.rs` 用 DPAPI 把 Cookie / API Key / SMTP 授权码绑定当前用户与机器，密文前缀 `dpapi:v1:`，数据库里只存密文。
- 适配 Linux 需要等价的「本机加密存储」，且遵守既有口径：**非 Windows 构建不得提供不安全的明文降级**（原实现直接返回 Err）。
- 硬约束：CI 与本地都要有真实可跑的加解密路径（不能只在开发机成立、runner 上退化成跳过）；不引入第二套加密实现（依赖树里 `ring` 已存在，来自 reqwest / lettre 的 rustls 后端）；`db.rs` 必须继续兼容旧明文记录与已存的 `dpapi:v1:` 密文。

## 备选方案

1. **系统钥匙串（secret-service / D-Bus，`keyring` crate）**
   - 优点：密钥交给系统守护进程（GNOME Keyring / KWallet）保管，安全性依赖桌面会话隔离。
   - 缺点：无桌面会话环境（headless、CI runner、部分窗口管理器）不可用，加解密直接失败；可用性还取决于 keyring 是否解锁，用户没登录桌面时凭据读不出；引入新依赖。
2. **本机密钥文件 + AES-256-GCM（ring）**
   - 优点：无守护进程依赖，任何 Linux 环境（含 CI）都能真实跑通；`ring` 已在依赖树，零新增加密实现；0600 密钥与 SQLite 同目录，与常见桌面应用做法一致。
   - 缺点：密钥与密文同一台机器同一用户下均可读——拿到用户账户（或 root）即可解密；应用数据目录被整体复制到别的机器后仍可解密。这两点均弱于 DPAPI 的「绑定用户 + 机器」语义。
3. **明文存储**
   - 优点：无实现成本。
   - 缺点：直接违反「不做不安全降级」的既有口径，凭据泄露面最大。否决。

## 决策（选定方案）

方案 2，且与 DPAPI 共用一个模块接口：

- 接口统一为 `encrypt(key_dir, plaintext) -> hex` / `decrypt(key_dir, hex)`；Windows 忽略 `key_dir`（保留 DPAPI 路径），Unix 用 `key_dir` 下的密钥文件。
- 密钥：应用数据目录中 `secret.key`，32 字节随机、权限 0600、首个写入者用 `create_new` 竞争落盘（并发时后来者读取已建密钥）；解密路径缺密钥文件时也会补建一把新密钥，供后续写入使用（旧的密文按「解不开」上报）。
- 密文：AES-256-GCM，随机 12 字节 nonce，存储为 `nonce || ciphertext+tag` 的 hex，前缀 `aes:v1:`。
- `db.rs` 用 `ENCRYPTED_PREFIXES = ["dpapi:v1:", "aes:v1:"]` 双前缀识别：写侧按平台取单前缀，读侧两种都认；读到对方平台的密文且解密失败 → 按「配过但本机解不开」提示重新填写，**绝不回退明文**。
- 旧明文记录照旧兼容（`decrypt_secret` 无前缀时按明文返回）。

## 理由（决策依据）

- 钥匙串方案会把「最需要验证的加解密路径」变成环境特供分支：CI 与 headless 场景不可用，本地测试与发布验证都会失真。软件密钥 + 0600 文件在任何 Linux 环境（包括 ubuntu-22.04 runner）都能跑真实路径，测试不需要 mock。
- `ring` 已在依赖树内，复用它不增加体积与审计面，也避免同一二进制里出现两套 AEAD 实现。
- 与 DPAPI 的强度差异（同用户可读、目录拷贝后可解）如实承认：产品口径本就是「凭据绑定本机加密保存、不做跨设备同步」，Linux 方案落在同一承诺面内，不夸大也不缩水。

## 后果（影响与后续）

- 两平台密文互不通用：换平台或换设备都要重新登录/重新填写，界面提示「无法解密，请重新填写」。
- `secret.key` 与 `amax_dashboard.db` 同目录（Linux `~/.local/share/com.amax.dashboard/`）：备份、迁移应用数据时两者一起走；单独删除密钥会让现有密文不可解。
- 2026-10-08 完成实现并随 Linux 适配提交；`crypto.rs` 新增 5 条 Unix 单元测试（中文往返、随机 nonce、换密钥失败、篡改密文失败、0600 权限），CI 双平台都在真实路径上执行。
- 相关页面：[桌面安装包与前端资源](../entities/desktop-packaging.md)、[Linux 发布产物与更新通道](linux-release-and-update.md)。
