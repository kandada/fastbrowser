// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "FastbrowserDeviceTests",
    platforms: [.iOS(.v14), .macOS(.v12)],
    products: [
        .library(name: "FastbrowserDeviceTests", targets: ["FastbrowserDeviceTests"]),
    ],
    targets: [
        // C 头文件（fastbrowser.h 拷贝至此）
        .target(name: "Cfastbrowser"),
        // WKWebView WebViewOps 宿主 + 真机用例 runner
        .target(name: "FastbrowserDeviceTests", dependencies: ["Cfastbrowser"]),
        .testTarget(name: "FastbrowserDeviceTestsTests", dependencies: ["FastbrowserDeviceTests"]),
    ]
)
