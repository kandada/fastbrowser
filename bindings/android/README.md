# Android 绑定（Kotlin）

可复用的 `com.fastbrowser.Sdk`（App 无关）。JNI 胶水在 `fastbrowser_c/src/jni_glue.c`。

## 接入步骤

1. 构建内核 staticlib：
   ```bash
   ./fastbrowser_c/build.sh          # 产出 c_dist/arm64-v8a/libfastbrowser.a
   ```
2. 在 `app/build.gradle.kts`：
   ```kotlin
   android {
     externalNativeBuild {
       cmake { path = file("../../fastbrowser_c/CMakeLists.txt") }
     }
   }
   ```
3. 拷贝 `Sdk.kt` 到 `app/src/main/java/com/fastbrowser/Sdk.kt`。
4. 使用：
   ```kotlin
   val fb = Sdk
   Sdk.eventListener = { tab, ev -> /* 驱动对话流 */ }
   Sdk.init(engine = "webview")       // 或 "mock"
   Sdk.open("https://example.com")
   val links = Sdk.toolCall("extract_links")
   ```

## 说明

- `engine="webview"` 时，宿主需在 `Sdk.init` 前通过 JNI 注册 `FbWebViewOps`
  （在原生层实现 WebView 的真实操作），或使用 `fastbrowser_register_webview_ops`。
- 事件/帧回调：`Sdk.onPageEvent(tab, json)` / `Sdk.onViewFrame(tab, json)`。
