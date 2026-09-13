#!/bin/bash
# ═══════════════════════════════════════════════════════════
# 下载 CEF 预编译发行包到 $VENDOR_DIR/cef/（供 engine-cef 使用，不进 git）。
#
# 用法:
#   ./scripts/fetch-cef.sh [branch] [arch]
#     默认 branch=stable, arch 由本机架构决定 (arm64/x86_64)
# ═══════════════════════════════════════════════════════════

set -euo pipefail
cd "$(dirname "$0")/.."
FB_LOCAL="$(cd "$(dirname "$0")/.." && pwd)"   # fastbrowser 仓库根
VENDOR_DIR="${VENDOR_DIR:-$FB_LOCAL/vendor}"

BRANCH="${1:-stable}"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) CEF_ARCH="arm64" ;;
  x86_64) CEF_ARCH="x86_64" ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

# 用 cef-rs 的 export-cef-dir 工具拉取（与 engine-cef 构建一致）
echo "=== 拉取 CEF ($BRANCH / $CEF_ARCH) 到 $VENDOR_DIR/cef ==="
mkdir -p $VENDOR_DIR
if ! command -v cargo >/dev/null; then
  echo "error: cargo not found" >&2; exit 1
fi

# 先安装 cmake/ninja 检查
for tool in cmake ninja; do
  if ! command -v "$tool" >/dev/null; then
    echo "warning: '$tool' not found — engine-cef 需要 cmake + ninja（brew install cmake ninja）"
  fi
done

echo "提示: 直接运行以下命令让 cef crate 自动下载匹配的 CEF 预编译包："
echo "  cargo check --features engine-cef"
echo
echo "若需手动管理 $VENDOR_DIR/cef，可参考 https://cef-builds.spotifycdn.com/index.html"
echo "把解压后的目录放到 $VENDOR_DIR/cef，并在构建时设置 CEF_PATH=$VENDOR_DIR/cef"
