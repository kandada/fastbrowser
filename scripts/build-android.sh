#!/bin/bash
# ═══════════════════════════════════════════════════════════
# Android 交叉编译：Rust staticlib → $FB_LOCAL/c_dist/，可选 NDK 链接 .so
#
# 用法:
#   ./scripts/build-android.sh              # 编译 staticlib
#   ./scripts/build-android.sh --so         # 额外用 NDK 链接 libfastbrowser_jni.so
#
# 环境变量: NDK_DIR / TARGET / ABI / API
# ═══════════════════════════════════════════════════════════

set -euo pipefail
cd "$(dirname "$0")/.."
FB_LOCAL="$(cd "$(dirname "$0")/.." && pwd)"   # fastbrowser 仓库根（target / c_dist 存放于此）

TARGET="${TARGET:-aarch64-linux-android}"
ABI="${ABI:-arm64-v8a}"
API="${API:-26}"
NDK_DIR="${NDK_DIR:-$(ls -d /Users/*/RustroverProjects/fastshell_local/android-ndk-*/toolchains/llvm/prebuilt/* 2>/dev/null | head -1)}"

if [ -z "$NDK_DIR" ] || [ ! -d "$NDK_DIR" ]; then
  echo "error: NDK not found. Set NDK_DIR." >&2
  exit 1
fi

# 确保交叉编译 target 已安装
if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET"; then
  echo "==> rustup target add $TARGET"
  rustup target add "$TARGET"
fi

# 移动端默认使用 webview 引擎；如需 mock 可在内核内做本地验证
FEATURES="${FEATURES:-engine-webview}"

echo "=== [1/2] cargo build --release --target $TARGET --features $FEATURES ==="
cargo build --release --target "$TARGET" --features "$FEATURES"

mkdir -p "$FB_LOCAL/c_dist/$ABI"
cp "$FB_LOCAL/target/$TARGET/release/libfastbrowser.a" "$FB_LOCAL/c_dist/$ABI/libfastbrowser.a"
echo "  → $FB_LOCAL/c_dist/$ABI/libfastbrowser.a ($(ls -lh "$FB_LOCAL/c_dist/$ABI/libfastbrowser.a" | awk '{print $5}'))"

if [ "${1:-}" = "--so" ]; then
  echo "=== [2/2] NDK clang: libfastbrowser_jni.so ==="
  CC="$NDK_DIR/aarch64-linux-android${API}-clang"
  "$CC" -shared -fPIC \
    -I fastbrowser_c/include \
    fastbrowser_c/src/jni_glue.c \
    "$FB_LOCAL/c_dist/$ABI/libfastbrowser.a" \
    -Wl,--gc-sections -Wl,--exclude-libs,ALL -Wl,--no-undefined \
    -Wl,-z,max-page-size=16384 \
    -llog -ldl -lm -lz \
    -o "$FB_LOCAL/c_dist/$ABI/libfastbrowser_jni.so"
  "$NDK_DIR/llvm-strip" --strip-unneeded "$FB_LOCAL/c_dist/$ABI/libfastbrowser_jni.so"
  echo "  → $FB_LOCAL/c_dist/$ABI/libfastbrowser_jni.so"
fi

echo "Done."
