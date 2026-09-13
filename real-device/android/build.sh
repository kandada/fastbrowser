#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Android 交叉编译：Rust staticlib → $FB_LOCAL/c_dist/，可选 NDK 链接 .so。
#
# 用法:
#   real-device/android/build.sh            # staticlib
#   real-device/android/build.sh --so       # 额外出 libfastbrowser_jni.so
#
# 环境: TARGET / ABI / API / NDK_DIR（大资源均在外部盘，见 env.sh）
# ═══════════════════════════════════════════════════════════════
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FB_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"   # fastbrowser 内核仓库
source "$SCRIPT_DIR/env.sh"

TARGET="${TARGET:-aarch64-linux-android}"
ABI="${ABI:-arm64-v8a}"
API="${API:-26}"
NDK_TOOLCHAIN="$(ls -d "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/* 2>/dev/null | head -1 || true)"
if [ -z "$NDK_TOOLCHAIN" ] || [ ! -d "$NDK_TOOLCHAIN" ]; then
  echo "error: NDK toolchain not found under $ANDROID_NDK_HOME. Set NDK_DIR / install NDK." >&2
  exit 1
fi
export NDK_DIR="$ANDROID_NDK_HOME"

rustup target add "$TARGET" >/dev/null 2>&1 || true

echo "=== [1/2] cargo build --release --target $TARGET --features engine-webview ==="
cargo build --release --target "$TARGET" --features engine-webview --manifest-path "$FB_ROOT/Cargo.toml"

mkdir -p "$FB_LOCAL/c_dist/$ABI"
cp "$FB_LOCAL/target/$TARGET/release/libfastbrowser.a" "$FB_LOCAL/c_dist/$ABI/libfastbrowser.a"
echo "  → $FB_LOCAL/c_dist/$ABI/libfastbrowser.a"

if [ "${1:-}" = "--so" ]; then
  echo "=== [2/2] NDK clang: libfastbrowser_jni.so ==="
  CC="$NDK_TOOLCHAIN/aarch64-linux-android${API}-clang"
  "$CC" -shared -fPIC \
    -I "$FB_ROOT/fastbrowser_c/include" \
    "$FB_ROOT/fastbrowser_c/src/jni_glue.c" \
    "$FB_ROOT/fastbrowser_c/src/jni_webview_bridge.c" \
    "$FB_LOCAL/c_dist/$ABI/libfastbrowser.a" \
    -Wl,--gc-sections -Wl,--exclude-libs,ALL -Wl,--no-undefined \
    -Wl,-z,max-page-size=16384 \
    -o "$FB_LOCAL/c_dist/$ABI/libfastbrowser_jni.so"
  echo "  → $FB_LOCAL/c_dist/$ABI/libfastbrowser_jni.so"
  # 复制到 app jniLibs，随 APK 打包
  mkdir -p "$SCRIPT_DIR/app/app/src/main/jniLibs/$ABI"
  cp "$FB_LOCAL/c_dist/$ABI/libfastbrowser_jni.so" "$SCRIPT_DIR/app/app/src/main/jniLibs/$ABI/"
  echo "  → app/app/src/main/jniLibs/$ABI/libfastbrowser_jni.so"
fi
echo "done."
