# fastbrowser_c

C ABI + JNI 胶水目录（对齐 `fastshell_c/`）。

## 组成

| 文件 | 说明 |
|------|------|
| `include/fastbrowser.h` | 一套头文件，三平台通用 C ABI |
| `src/jni_glue.c` | Android JNI 胶水（方案 B：staticlib + NDK CMake） |
| `CMakeLists.txt` | 把 `jni_glue.c` + `libfastbrowser.a` 打成 `libfastbrowser_jni.so` |
| `build.sh` | 编译 staticlib → `c_dist/`，可选 NDK `.so` 验证 |

## 平台接入路径

- **Android（方案 B）**：
  1. `./fastbrowser_c/build.sh` → 产出 `c_dist/arm64-v8a/libfastbrowser.a`
  2. 在 App 的 `build.gradle.kts` 配置 `externalNativeBuild { cmake { path "...fastbrowser_c/CMakeLists.txt" } }`
  3. Kotlin：`System.loadLibrary("fastbrowser_jni")` → `com.fastbrowser.Sdk`
- **iOS/macOS**：`cargo build --release --target aarch64-apple-ios` 得 `libfastbrowser.a`，Xcode 链接 + `fastbrowser.h` 直接调 C 函数。
- **桌面**：cdylib（`libfastbrowser.dylib/.dll/.so`），`dlopen` 或链接调用。

## C ABI 约定

- 全局单例，进出全为 JSON 字符串，`fastbrowser_free_string` 释放；
- 错误统一为 `{"error":{"kind":"...","message":"..."}}`；
- 事件/帧通过回调推给宿主（Android 上由 `JNI_OnLoad` 转给 Kotlin 静态方法 `Sdk.onPageEvent` / `Sdk.onViewFrame`）；
- 移动端 `engine=webview` 时，宿主用 `FbWebViewOps` 函数表注入真实 WebView 操作。
