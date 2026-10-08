#!/usr/bin/env bash
# 桌面 Linux 安装包核验：deb 与 AppImage 的程序版本、关键功能内容与更新签名。
# 与 verify-desktop-bundle.ps1（Windows 侧）对应，检查口径一致：
# 程序版本等于配置版本、内嵌告警与直连相关功能、更新签名存在（--require-signatures）。
# 更新包就是原始 AppImage：createUpdaterArtifacts: true 的 v2 风格只给原始安装包配 .sig，
# v1 兼容模式的 .AppImage.tar.gz 不产出（v2 客户端两种格式都能装，见 linux-release-and-update 决策页）。
# 用法：scripts/verify-desktop-bundle.sh [--require-signatures]
set -euo pipefail

require_signatures=0
if [[ "${1:-}" == "--require-signatures" ]]; then
  require_signatures=1
fi

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release_dir="$project_root/target/release"
deb_dir="$release_dir/bundle/deb"
appimage_dir="$release_dir/bundle/appimage"

# 同 Windows 侧：安装包内的程序必须包含这些功能内容（IPC 命令名与消息文案）
markers=(告警规则 get_alert_settings set_alert_settings get_alert_channels 官网直连失败)

fail() {
  echo "$1" >&2
  exit 1
}

command -v node >/dev/null || fail "需要 node 读取 tauri.conf.json 的版本号"
version="$(node -p "require('${project_root}/src-tauri/tauri.conf.json').version")"

require_exactly_one() {
  local dir="$1" pattern="$2"
  local matches=()
  while IFS= read -r line; do matches+=("$line"); done < <(find "$dir" -maxdepth 1 -type f -name "$pattern" 2>/dev/null)
  if [[ ${#matches[@]} -ne 1 ]]; then
    fail "期望 $dir 下恰好 1 个 $pattern，实际 ${#matches[@]} 个（目录内容：$(ls -A "$dir" 2>/dev/null | tr '\n' ' '))"
  fi
  printf '%s' "${matches[0]}"
}

[[ -d "$deb_dir" ]] || fail "缺少 deb 目录：$deb_dir"
[[ -d "$appimage_dir" ]] || fail "缺少 AppImage 目录：$appimage_dir"

deb_file="$(require_exactly_one "$deb_dir" "*_${version}_amd64.deb")"
appimage_file="$(require_exactly_one "$appimage_dir" "*_${version}_amd64.AppImage")"

deb_version="$(dpkg-deb -f "$deb_file" Version)"
[[ "$deb_version" == "$version" ]] || fail "deb 版本 $deb_version 与配置版本 $version 不一致"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

check_markers() {
  local binary="$1" label="$2" marker
  for marker in "${markers[@]}"; do
    grep -qaF -- "$marker" "$binary" || fail "$label 缺少功能内容：$marker"
  done
}

# deb 内主程序固定装到 usr/bin/amax（主二进制名取 Cargo 包名）
dpkg-deb -x "$deb_file" "$workdir/deb"
deb_bin="$workdir/deb/usr/bin/amax"
[[ -f "$deb_bin" ]] || fail "deb 内未找到 usr/bin/amax（usr 下文件：$(find "$workdir/deb/usr" -maxdepth 3 -type f 2>/dev/null | head -20 | tr '\n' ' '))"
check_markers "$deb_bin" "deb 内的程序"

# AppImage 展开为 squashfs-root，主程序同样在 usr/bin/amax
(cd "$workdir" && "$appimage_file" --appimage-extract >/dev/null)
appimage_bin="$workdir/squashfs-root/usr/bin/amax"
[[ -f "$appimage_bin" ]] || fail "AppImage 内未找到 usr/bin/amax"
check_markers "$appimage_bin" "AppImage 内的程序"

if [[ "$require_signatures" -eq 1 ]]; then
  [[ -f "$appimage_file.sig" ]] || fail "缺少更新签名：$(basename "$appimage_file").sig"
fi

echo "安装包核验通过：版本 $version；告警功能已嵌入程序；deb：$(basename "$deb_file")；AppImage：$(basename "$appimage_file")"
