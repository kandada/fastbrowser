#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════
# Android 真机测试环境：SDK/NDK/Gradle 全部重定向到外部盘 fastbrowser_local。
# 用法：  source real-device/android/env.sh   （会先 source real-device/env.sh）
# ═══════════════════════════════════════════════════════════════
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../env.sh"

export ANDROID_HOME="${ANDROID_HOME:-$FB_LOCAL/android-sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$FB_LOCAL/android-ndk}"
export NDK_DIR="$ANDROID_NDK_HOME"
export GRADLE_USER_HOME="${GRADLE_USER_HOME:-$FB_LOCAL/gradle-home}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
# 让 gradle wrapper 用外部盘缓存（避免写满内置盘）
export GRADLE_OPTS="${GRADLE_OPTS:--Dorg.gradle.jvmargs=-Xmx2g -Dorg.gradle.daemon=false}"

mkdir -p "$ANDROID_HOME" "$ANDROID_NDK_HOME" "$GRADLE_USER_HOME"

echo "[android/env] ANDROID_HOME=$ANDROID_HOME"
echo "  ANDROID_NDK_HOME=$ANDROID_NDK_HOME"
echo "  GRADLE_USER_HOME=$GRADLE_USER_HOME"
