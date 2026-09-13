#!/usr/bin/env bash
# iOS 真机测试驱动（构建 Rust staticlib 到外部盘 + xcodebuild test）。
# 用法：  real-device/ios/run.sh <destination>   例：run.sh 'platform=iOS,name=iPhone'
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FB_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"   # fastbrowser 内核仓库
source "$SCRIPT_DIR/../env.sh"

DEST="${1:-platform=iOS,name=iPhone}"

echo "=== Rust staticlib (aarch64-apple-ios, engine-webview) ==="
rustup target add aarch64-apple-ios >/dev/null 2>&1 || true
cargo build --release --target aarch64-apple-ios --features engine-webview --manifest-path "$FB_ROOT/Cargo.toml"

echo "=== xcodebuild test -destination '$DEST' ==="
cd "$SCRIPT_DIR/FastbrowserDeviceTests"
xcodebuild -scheme FastbrowserDeviceTests -destination "$DEST" test 2>&1 | tail -40
