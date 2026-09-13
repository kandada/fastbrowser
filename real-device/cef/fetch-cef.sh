#!/bin/bash
# 拉取 CEF 预编译发行包到外部盘 $VENDOR_DIR/cef/（engine-cef 用；不进 git）。
# 用法:  real-device/cef/fetch-cef.sh [branch] [arch]
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../env.sh"

BRANCH="${1:-stable}"
ARCH="${2:-$(uname -m)}"
mkdir -p "$VENDOR_DIR"

# 用 cef-rs 的官方下载脚本（与 engine-cef 构建一致），指定输出目录到外部盘
echo "=== 拉取 CEF ($BRANCH/$ARCH) → $VENDOR_DIR/cef ==="
cd "$VENDOR_DIR"

# cef-rs 提供 `cef-download`；也可直接用 CEF 官方自动化脚本：
#   curl -LO https://github.com/cefbuilds/CEF/raw/master/tools/automate/automate.py
#   python3 automate.py --download-dir="$VENDOR_DIR/cef" --branch=$BRANCH --no-build
if command -v cef-download >/dev/null 2>&1; then
  cef-download --dir "$VENDOR_DIR/cef" --branch "$BRANCH" --arch "$ARCH"
else
  echo "未找到 cef-download，使用官方 automate.py："
  curl -fsSL --retry 3 -o "$VENDOR_DIR/automate.py" \
    https://raw.githubusercontent.com/cefbuilds/CEF/master/tools/automate/automate.py
  python3 "$VENDOR_DIR/automate.py" \
    --download-dir="$VENDOR_DIR/cef" \
    --branch="$BRANCH" \
    --no-build \
    --minimal-distribution
fi
echo "→ $VENDOR_DIR/cef/（供 engine-cef 构建）"
