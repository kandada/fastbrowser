#!/bin/bash
# ═══════════════════════════════════════════════════════════
# 下载 Chrome for Testing（Chromium 发行版）到 $VENDOR_DIR/chromium/，
# 供桌面端"打包集成 Chromium + CDP"使用（Playwright 同款思路）。
# 产物：$VENDOR_DIR/chromium/chrome-mac-*/…（含可执行文件，约 150MB）
#
# 用法：
#   ./scripts/fetch-chromium.sh                 # 最新 stable，自动选平台/架构
#   CHROME_CFT_VERSION=133.0.6943.53 ./scripts/fetch-chromium.sh   # 指定版本
# ═══════════════════════════════════════════════════════════

set -euo pipefail
cd "$(dirname "$0")/.."
FB_LOCAL="$(cd "$(dirname "$0")/.." && pwd)"   # fastbrowser 仓库根
# 浏览器存放目录（默认仓库内 vendor/；可用 VENDOR_DIR 覆盖，例如宿主指向外部盘）
VENDOR_DIR="${VENDOR_DIR:-$FB_LOCAL/vendor}"
mkdir -p "$VENDOR_DIR/chromium"

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) PLATFORM="mac-arm64" ;;
  Darwin/x86_64) PLATFORM="mac-x64" ;;
  Linux/x86_64) PLATFORM="linux64" ;;
  Linux/aarch64) PLATFORM="linux-arm64" ;;
  *) echo "unsupported platform $(uname -s)/$(uname -m)" >&2; exit 1 ;;
esac

echo "=== 解析 Chrome for Testing 版本 (${PLATFORM}) ==="
JSON="${CHROME_CFT_VERSION:-}"
if [ -z "$JSON" ]; then
  # 取 latest stable
  INDEX="https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json"
  echo "  fetch $INDEX"
  DATA="$(curl -fsSL --retry 3 --max-time 60 "$INDEX")"
  VERSION="$(echo "$DATA" | python3 -c "import json,sys;print(json.load(sys.stdin)['channels']['Stable']['version'])")"
  URL="$(echo "$DATA" | python3 -c "import json,sys;d=json.load(sys.stdin)['channels']['Stable']['downloads']['chrome'];print([x['url'] for x in d if x['platform']=='$PLATFORM'][0])")"
else
  VERSION="$JSON"
  URL="https://storage.googleapis.com/chrome-for-testing-public/${VERSION}/${PLATFORM}/chrome-${PLATFORM}.zip"
fi
echo "  version=$VERSION"
echo "  url=$URL"

ZIP="$VENDOR_DIR/chromium/chrome-${PLATFORM}-${VERSION}.zip"
TOTAL="$(curl -fsSI --retry 3 --max-time 60 "$URL" | awk 'tolower($1)=="content-length:"{print $2}' | tr -d '\r')"
echo "=== 下载 (${TOTAL} bytes) ==="
if [ -f "$ZIP" ] && [ "$(stat -f%z "$ZIP" 2>/dev/null || stat -c%s "$ZIP" 2>/dev/null || echo 0)" = "$TOTAL" ]; then
  echo "  已存在，跳过"
else
  rm -f "$ZIP"
  for i in $(seq 1 60); do
    CUR="$(stat -f%z "$ZIP" 2>/dev/null || stat -c%s "$ZIP" 2>/dev/null || echo 0)"
    if [ -n "$TOTAL" ] && [ "$CUR" = "$TOTAL" ]; then break; fi
    echo "  try $i: $CUR/${TOTAL:-?}"
    curl --http1.1 -fsSL -C - --max-time 90 -o "$ZIP" "$URL" >/dev/null 2>&1 || true
  done
fi

echo "=== 解压 ==="
TMPD="$VENDOR_DIR/chromium/.unpack-$$"
mkdir -p "$TMPD"
unzip -oq "$ZIP" -d "$TMPD"
# 清理旧产物，把新目录提升为 $VENDOR_DIR/chromium/（保留 zip）
find $VENDOR_DIR/chromium -maxdepth 1 -type d ! -name chromium ! -name '.unpack-*' -exec rm -rf {} +
mv "$TMPD"/*/ $VENDOR_DIR/chromium/ 2>/dev/null || mv "$TMPD"/* $VENDOR_DIR/chromium/
rm -rf "$TMPD" "$ZIP"

BIN=$(find $VENDOR_DIR/chromium -type f \( -name 'Google Chrome for Testing' -o -name 'chrome' -o -name 'chromium' \) 2>/dev/null | head -1)
if [ -n "$BIN" ] && [ -x "$BIN" ]; then
  echo "✓ 就绪：$BIN"
  echo "  用 engine: chromium / auto 时内核会自动发现并启动它（见 engines/bundled.rs）。"
else
  echo "⚠ 未找到可执行文件，请检查 $VENDOR_DIR/chromium" >&2
fi
