#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════
# 真机测试统一环境：把所有大体积资源重定向到外部盘 fastbrowser_local/
# （mac 内置盘空间有限；本脚本一次性导出全部相关环境变量）。
#
# 用法：  source real-device/env.sh
# ═══════════════════════════════════════════════════════════════════
set -euo pipefail

# fastbrowser 仓库根（外部盘）
FB_LOCAL="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export FB_LOCAL

# ── Rust ───────────────────────────────────────────────────────────
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$FB_LOCAL/target}"

# ── 浏览器 / 引擎（vendor，可用 VENDOR_DIR 覆盖）─────────────────
export VENDOR_DIR="${VENDOR_DIR:-$FB_LOCAL/vendor}"
export CEF_ROOT="${CEF_ROOT:-$VENDOR_DIR/cef}"
# 桌面 bundled 引擎：优先使用外部盘上的 Chrome for Testing 可执行文件
CHROME_CANDIDATES="$(find "$VENDOR_DIR" -maxdepth 7 -type f \
  \( -name 'chrome' -o -name 'chrome.exe' -o -name 'Google Chrome for Testing' \) 2>/dev/null | head -1)"
export CHROME_PATH="${CHROME_PATH:-${CHROME_CANDIDATES:-}}"
if [ -z "${CHROME_PATH:-}" ]; then
  unset CHROME_PATH
fi

# ── Android（外部盘）──────────────────────────────────────────────
export ANDROID_HOME="${ANDROID_HOME:-$FB_LOCAL/android-sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$FB_LOCAL/android-ndk}"
export NDK_DIR="${NDK_DIR:-$ANDROID_NDK_HOME}"
export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$FB_LOCAL/gradle-home}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"

# ── 测试报告（外部盘）─────────────────────────────────────────────
export REAL_DEVICE_REPORTS="${REAL_DEVICE_REPORTS:-$FB_LOCAL/real-device-reports}"
mkdir -p "$REAL_DEVICE_REPORTS"

echo "[real-device/env] FB_LOCAL=$FB_LOCAL"
echo "  CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
echo "  CEF_ROOT=$CEF_ROOT"
echo "  ANDROID_HOME=$ANDROID_HOME"
echo "  ANDROID_NDK_HOME=$ANDROID_NDK_HOME"
echo "  GRADLE_USER_HOME=$GRADLE_USER_HOME"
echo "  CHROME_PATH=${CHROME_PATH:-（未设置，用系统 Chrome 或--engine mock）}"
echo "  REAL_DEVICE_REPORTS=$REAL_DEVICE_REPORTS"
