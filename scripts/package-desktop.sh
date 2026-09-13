#!/bin/bash
# ═══════════════════════════════════════════════════════════
# 桌面端"打包集成 Chromium"分发脚本。
#
# 思路：把 vendored Chromium（Chrome for Testing）或 CEF 与内核产物
# （libfastbrowser + fastbrowser CLI）一起打进可分发的包。
#
# 用法：
#   ./scripts/package-desktop.sh mac chromium     # macOS + 打包 Chromium（默认）
#   ./scripts/package-desktop.sh mac cef          # macOS + CEF 嵌入
#   ./scripts/package-desktop.sh linux chromium
#   ./scripts/package-desktop.sh win chromium
#
# 环境变量（签名/公证，可选）：
#   APP_NAME      应用名（默认 fastbrowser）
#   CODESIGN_ID   签名身份（如 "Developer ID Application: ..."）
#   NOTARY_APPLE_ID / NOTARY_TEAM / NOTARY_PASSWORD（notarytool）
# ═══════════════════════════════════════════════════════════

set -euo pipefail
cd "$(dirname "$0")/.."
FB_LOCAL="$(cd "$(dirname "$0")/.." && pwd)"   # fastbrowser 仓库根
VENDOR_DIR="${VENDOR_DIR:-$FB_LOCAL/vendor}"

PLATFORM="${1:-mac}"
BUNDLE="${2:-chromium}"
APP_NAME="${APP_NAME:-fastbrowser}"
OUT="$FB_LOCAL/dist"
mkdir -p "$OUT"

echo "=== 1. 构建内核 ==="
if [ "$BUNDLE" = "cef" ]; then
  cargo build --release --features engine-cef
  FEAT="engine-cef"
  ENGINE_HINT="cef（打包预编译 CEF，需先 scripts/fetch-cef.sh）"
else
  cargo build --release --features engine-cdp
  FEAT="engine-cdp"
  ENGINE_HINT="bundled（打包 Chromium，需先 scripts/fetch-chromium.sh）"
fi
echo "  feature=$FEAT"

# 定位 Chromium 可执行（bundled 模式）
CHROME_BIN=""
if [ "$BUNDLE" = "chromium" ]; then
  CHROME_BIN=$(find $VENDOR_DIR/chromium -type f \( -name 'Google Chrome for Testing' -o -name 'chrome' \) 2>/dev/null | head -1)
  [ -n "$CHROME_BIN" ] || { echo "error: $VENDOR_DIR/chromium 缺失，先运行 scripts/fetch-chromium.sh" >&2; exit 1; }
fi

case "$PLATFORM" in
  mac)
    echo "=== 2. 组装 ${APP_NAME}.app (macOS) ==="
    APP="$OUT/${APP_NAME}.app"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"

    # 内核产物
    cp $FB_LOCAL/target/release/libfastbrowser.dylib "$APP/Contents/Frameworks/"
    cp $FB_LOCAL/target/release/fastbrowser "$APP/Contents/MacOS/"

    # 打包 Chromium / CEF
    if [ "$BUNDLE" = "chromium" ]; then
      # 把 vendored Chromium .app 整个拷进 Frameworks
      CHROME_APP=$(dirname "$(dirname "$(dirname "$CHROME_BIN")")") # .../Google Chrome for Testing.app
      cp -R "$CHROME_APP" "$APP/Contents/Frameworks/"
    else
      # CEF：$VENDOR_DIR/cef 的 Chromium Embedded Framework.framework
      cp -R $VENDOR_DIR/cef/* "$APP/Contents/Frameworks/" 2>/dev/null || true
    fi

    cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>com.fastbrowser.app</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleExecutable</key><string>fastbrowser</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
PLIST

    # 签名（可选）
    if [ -n "${CODESIGN_ID:-}" ]; then
      echo "=== 3. 签名 ==="
      codesign --force --deep --sign "$CODESIGN_ID" "$APP"
      # 公证（可选）
      if [ -n "${NOTARY_APPLE_ID:-}" ]; then
        xcrun notarytool submit "$APP" --apple-id "$NOTARY_APPLE_ID" --team-id "${NOTARY_TEAM}" --password "$NOTARY_PASSWORD" --wait || true
        xcrun stapler staple "$APP" || true
      fi
    else
      echo "  （未设置 CODESIGN_ID，跳过签名/公证）"
    fi

    # 打 zip / dmg 占位
    (cd "$OUT" && zip -qr "${APP_NAME}-mac-${BUNDLE}.zip" "${APP_NAME}.app")
    echo "✓ $OUT/${APP_NAME}-mac-${BUNDLE}.zip"
    echo "  引擎: ${ENGINE_HINT}"
    echo "  运行: $APP/Contents/MacOS/fastbrowser --engine $([ "$BUNDLE" = "chromium" ] && echo bundled || echo cef) open https://example.com"
    ;;

  linux)
    echo "=== 2. 组装 Linux 目录 ==="
    LIN="$OUT/${APP_NAME}-linux-${BUNDLE}"
    rm -rf "$LIN"
    mkdir -p "$LIN"
    cp $FB_LOCAL/target/release/libfastbrowser.so "$LIN/" 2>/dev/null || true
    cp $FB_LOCAL/target/release/fastbrowser "$LIN/"
    if [ "$BUNDLE" = "chromium" ]; then
      cp -R $VENDOR_DIR/chromium/* "$LIN/" 2>/dev/null || true
    fi
    echo "  ⚠ Linux：分发时请确保 chrome-sandbox 已 chmod 4755（setuid），否则 Chromium 沙箱会拒绝启动："
    echo "      chmod 4755 \$(find \"$LIN\" -name chrome-sandbox)"
    tar -C "$OUT" -czf "$LIN.tar.gz" "$(basename "$LIN")"
    echo "✓ $LIN.tar.gz"
    ;;

  win)
    echo "=== 2. 组装 Windows 目录（需在 Windows 上执行，signtool 签名）==="
    WIN="$OUT/${APP_NAME}-win-${BUNDLE}"
    rm -rf "$WIN"
    mkdir -p "$WIN"
    cp $FB_LOCAL/target/release/fastbrowser.exe "$WIN/" 2>/dev/null || true
    cp $FB_LOCAL/target/release/fastbrowser.dll "$WIN/" 2>/dev/null || true
    [ "$BUNDLE" = "chromium" ] && cp -R $VENDOR_DIR/chromium/* "$WIN/" 2>/dev/null || true
    echo "  请在 Windows 上用 signtool sign 签名后打成安装器（WiX/Inno Setup）。"
    echo "✓ $WIN"
    ;;

  *) echo "unknown platform: $PLATFORM (mac|linux|win)" >&2; exit 1 ;;
esac

echo "Done."
