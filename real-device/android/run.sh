#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Android 真机驱动：构建安装 + 执行用例 + 拉取报告。
# 用法:
#   real-device/android/run.sh install     # gradle installDebug
#   real-device/android/run.sh run         # 打开 app 执行用例（结果写回外部盘）
#   real-device/android/run.sh all         # install + run
# ═══════════════════════════════════════════════════════════════
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/env.sh"

ACTION="${1:-all}"
cd "$SCRIPT_DIR/app"

if ! command -v adb >/dev/null; then
  ADB="$ANDROID_HOME/platform-tools/adb"
else
  ADB=adb
fi
if ! command -v gradle >/dev/null; then
  GRADLE="$ANDROID_HOME/gradle/bin/gradle"
else
  GRADLE=gradle
fi
# 优先用 wrapper（若已生成）；否则系统 gradle
[ -x ./gradlew ] && GRADLE=./gradlew

case "$ACTION" in
  install|all)
    echo "=== gradle installDebug ==="
    "$GRADLE" installDebug
    "$ADB" shell am start -n com.fastbrowser.device/.MainActivity
    ;;
esac

case "$ACTION" in
  run|all)
    echo "=== 等待 app 执行用例（Logcat 观察）==="
    echo "  app 完成用例后会把报告写入 /sdcard/Download/fb-realdevice/android.json"
    echo "  手动触发：adb shell am force-stop com.fastbrowser.device; adb shell am start ..."
    "$ADB" wait-for-device
    echo "  报告将拉取到 $REAL_DEVICE_REPORTS/android.json"
    "$ADB" pull /sdcard/Download/fb-realdevice/android.json "$REAL_DEVICE_REPORTS/android.json" 2>/dev/null || \
      echo "（报告尚未生成：请先在设备上点“运行”按钮）"
    ;;
esac

echo "done."
