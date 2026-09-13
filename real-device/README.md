# fastbrowser 真机测试（real-device）

> 对应 `fastbrowser_local/real-device-test-plan.md` 的落地实现。
> 本目录是**真机/跨平台测试程序**：同一套 JSON 用例（`shared/cases/`）+ 各平台 runner，
> 在 Android / iOS / Windows / Linux / 鸿蒙 / 桌面 CEF 上执行验收。

## 目录

| 目录 | 用途 | 状态 |
|---|---|---|
| `shared/` | **跨平台共享**：JSON 用例 + Rust 参考 runner（桌面 mock/chromium 先跑通同一套用例） | ✅ 可运行 |
| `android/` | Android 真机：Kotlin 宿主（WebViewOps 实现）+ JSON 用例 runner + NDK 构建 | 🔧 需 SDK/NDK |
| `ios/` | iOS 真机：Swift 宿主（WKWebView WebViewOps）+ XCTest 用例 runner | 🔧 需 Xcode |
| `windows/` | Windows：打包 Chromium（Chrome for Testing）集成测试 | 🔧 需 Windows |
| `linux/` | Linux：无头 CI 集成测试 | 🔧 需 Linux |
| `harmony/` | 鸿蒙：ArkWeb WebViewOps 宿主说明与规划 | 📄 规划 |
| `cef/` | 桌面 CEF 嵌入（engine-cef）拉取与验证 | 🔧 需 fetch-cef |

## 存储策略（重要）

**mac 内置盘空间有限（约 8GB 空闲），所有大体积资源一律放到外部盘 `fastbrowser_local/`（约 942GB 空闲）**：

| 资源 | 存放位置（外部盘） | 说明 |
|---|---|---|
| Rust 编译产物 | `fastbrowser_local/target/` | 已由 `.cargo/config.toml` 重定向 |
| Chrome for Testing（mac/win/linux） | `fastbrowser_local/vendor/chromium-*` | `scripts/fetch-chromium.sh` |
| CEF 预编译包 | `fastbrowser_local/vendor/cef/` | `real-device/cef/fetch-cef.sh` |
| Android SDK | `fastbrowser_local/android-sdk/` | `android/env.sh` 导出 `ANDROID_HOME` |
| Android NDK | `fastbrowser_local/android-ndk/` | `android/env.sh` 导出 `ANDROID_NDK_HOME` |
| Gradle 缓存 | `fastbrowser_local/gradle-home/` | `android/env.sh` 导出 `GRADLE_USER_HOME` |
| Android 产物 | `fastbrowser_local/c_dist/`、`fastbrowser_local/dist/` | `android/build.sh` |
| 真机测试报告 | `fastbrowser_local/real-device-reports/` | 各 runner 输出 |

> 一键加载：`source real-device/env.sh`（导出上述全部环境变量）。

## 快速开始（参考 runner，桌面即可跑）

```bash
# 环境变量（大资源重定向到外部盘）
source real-device/env.sh

# 用 mock 引擎跑全部用例（无需浏览器，CI 可用）
cd real-device/shared/runner
cargo run -- --cases ../cases --engine mock

# 用真实 Chromium 跑全部用例（本机有 Chrome 即可）
cargo run --features engine-cdp -- --cases ../cases --engine chromium

# 只跑某分类
cargo run --features engine-cdp -- --cases ../cases --engine chromium --category agent_flow
```

## 工作流

1. **先在桌面验证用例**：`shared/runner`（mock → chromium）跑通全部 JSON 用例。
2. **再上真机**：Android/iOS 宿主 runner 加载同一份 `cases/*.json`，经 C ABI 逐条执行。
3. **报告回填**：各平台 runner 输出 `fastbrowser_local/real-device-reports/<platform>.json`，
   对应 `real-device-test-plan.md` 的验收报告模板。
