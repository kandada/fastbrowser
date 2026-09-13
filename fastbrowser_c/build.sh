#!/bin/bash
# ═══════════════════════════════════════════════════════════
# fastbrowser_c 构建脚本 (Android 集成方案 B: staticlib + NDK CMake)
#
# 步骤:
#   1. cargo 编译 Rust 静态库 libfastbrowser.a (aarch64-linux-android)
#   2. 拷贝到 c_dist/arm64-v8a/libfastbrowser.a  (供 CMake 链接)
#   3. (可选) 用 NDK 直接编译 libfastbrowser_jni.so 做本地验证
#
# 用法:
#   ./fastbrowser_c/build.sh              # 编译 .a 并放入 c_dist/
#   ./fastbrowser_c/build.sh --so         # 额外用 NDK 编译 .so 验证
#
# 说明: 正常集成时无需 --so，Android Studio 的 CMake 会自动编译 .so。
# ═══════════════════════════════════════════════════════════

set -euo pipefail
cd "$(dirname "$0")/.."
FB_LOCAL="$(cd "$(dirname "$0")/.." && pwd)"   # fastbrowser 仓库根（target / c_dist 存放于此）

TARGET="${TARGET:-aarch64-linux-android}"
ABI="${ABI:-arm64-v8a}"
API="${API:-26}"
NDK_DIR="${NDK_DIR:-android-ndk-r27c/toolchains/llvm/prebuilt/darwin-x86_64/bin}"
CDIST="$FB_LOCAL/c_dist/${ABI}"

if [ ! -d "$(dirname "$NDK_DIR")" ]; then
  echo "error: NDK not found at ${NDK_DIR}. Set NDK_DIR to your NDK toolchain bin." >&2
  exit 1
fi
export PATH="$(pwd)/${NDK_DIR}:${PATH}"

echo "=== [1/2] cargo build staticlib (${TARGET}) ==="
cargo build --release --target "${TARGET}" --features engine-webview

STATICLIB="$FB_LOCAL/target/${TARGET}/release/libfastbrowser.a"
mkdir -p "${CDIST}"
cp "${STATICLIB}" "${CDIST}/libfastbrowser.a"
echo "  → ${CDIST}/libfastbrowser.a ($(ls -lh "${CDIST}/libfastbrowser.a" | awk '{print $5}'))"

if [ "${1:-}" = "--so" ]; then
    echo "=== [2/2] NDK clang: compile + link libfastbrowser_jni.so (verify) ==="
    CC="${NDK_DIR}/aarch64-linux-android${API}-clang"
    OUT="$FB_LOCAL/c_dist/${ABI}/libfastbrowser_jni.so"
    "${CC}" -shared -fPIC \
        -I fastbrowser_c/include \
        fastbrowser_c/src/jni_glue.c \
        "${CDIST}/libfastbrowser.a" \
        -Wl,--gc-sections -Wl,--exclude-libs,ALL -Wl,--no-undefined \
        -Wl,-z,max-page-size=16384 \
        -llog -ldl -lm -lz \
        -o "${OUT}"
    "${NDK_DIR}/llvm-strip" --strip-unneeded "${OUT}"
    echo "  → ${OUT} ($(ls -lh "${OUT}" | awk '{print $5}'))"
fi

echo "Done."
