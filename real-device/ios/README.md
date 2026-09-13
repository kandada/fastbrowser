# iOS 真机测试（P1，WKWebView）

Swift Package `FastbrowserDeviceTests`：
- `Cfastbrowser`：`fastbrowser.h`（已从 `fastbrowser_c/include/` 拷入）。
- `FastbrowserDeviceTests`：`WebViewBridge.swift`（WKWebView 实现 FbWebViewOps）+ `Fastbrowser.swift`（C ABI 封装）。
- `Tests/`：XCTest 用例（smoke / type-click）。

## 构建（大资源在外部盘）

```bash
source real-device/env.sh
# 把 Rust 编译为 iOS aarch64 staticlib 并链接进测试 target（Xcode 工程需配置）：
#   rustup target add aarch64-apple-ios
#   cargo build --release --target aarch64-apple-ios --features engine-webview
#   产物 $FB_LOCAL/target/aarch64-apple-ios/release/libfastbrowser.a
xcodebuild test -scheme FastbrowserDeviceTests -destination 'platform=iOS,name=<你的真机>'
```

> 说明：iOS App Store 2.5.6 限制只能用 WKWebView（WebKit）；内核 `engine-webview` 走
> `WebViewOps`（`supports_cdp=false`，能力向 LLM 收敛）。`evaluateJavaScript` 的 JSON 返回值
> 需确保不被二次包装（WebViewBridge 已用 `JSON.stringify(eval(...))` 统一序列化）。
