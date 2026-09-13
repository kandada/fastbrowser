// swift-tools-version:5.9
import PackageDescription
import Foundation

// 指向内核预编译库 target/release 的绝对路径。
let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let libSearch = packageDir + "/../../target/release"

let package = Package(
    name: "Fastbrowser",
    products: [
        .library(name: "Fastbrowser", targets: ["Fastbrowser"])
    ],
    targets: [
        .target(name: "Cfastbrowser"),
        .target(
            name: "Fastbrowser",
            dependencies: ["Cfastbrowser"],
            linkerSettings: [
                .unsafeFlags(["-L\(libSearch)"], .when(platforms: [.macOS, .iOS])),
                .linkedLibrary("fastbrowser"),
            ]
        ),
        .testTarget(name: "FastbrowserTests", dependencies: ["Fastbrowser"]),
    ]
)
