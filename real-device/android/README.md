# Android 真机测试（P0）

把 fastbrowser 内核以 `engine: "webview"`（系统 WebView = Chromium）跑在 Android 真机上，
经 `WebViewOps` 宿主桥 + `jni_glue.c` + Kotlin 封装，逐条执行 `shared/cases/*.json` 用例。

## 环境（大资源全部在外部盘 fastbrowser_local）

| 资源 | 位置 | 获取 |
|---|---|---|
| Android SDK | `$FB_LOCAL/android-sdk/` | Android Studio 或 `sdkmanager`，`--sdk_root=$FB_LOCAL/android-sdk` |
| Android NDK | `$FB_LOCAL/android-ndk/` | `sdkmanager --sdk_root=$FB_LOCAL/android-sdk "ndk;27.x"` |
| Gradle 缓存 | `$FB_LOCAL/gradle-home/` | 自动（`GRADLE_USER_HOME`） |
| Rust 产物 | `$FB_LOCAL/target/` | 自动 |
| JNI 产物 | `$FB_LOCAL/c_dist/` | `build.sh` |
| 测试报告 | `$FB_LOCAL/real-device-reports/android.json` | app 内 runner 输出 |

```bash
source real-device/env.sh          # 导出 ANDROID_HOME/NDK/GRADLE_USER_HOME 等
# 准备 SDK/NDK（若未装）
#   $ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager "platforms;android-34" "build-tools;34.0.0" "ndk;27.2.12479018"

# 1) 交叉编译内核 staticlib + JNI .so
real-device/android/build.sh --so

# 2) 构建并安装 app
real-device/android/run.sh install

# 3) 连真机后执行用例（结果写回 $REAL_DEVICE_REPORTS/android.json）
real-device/android/run.sh run
```

## 目录

- `app/` — Gradle 工程：`MainActivity`（宿主）、`WebViewOpsImpl`（WebViewOps C 表实现）、
  `CaseRunner`（加载 `assets/cases/*.json` 逐条执行并报告）。
- `build.sh` — NDK 交叉编译（`--so` 出 `libfastbrowser_jni.so`）。
- `run.sh` — `gradle installDebug` + `adb` 驱动 + 报告拉取。

## 验证要点（对应 real-device-test-plan §2）

1. 注入链路：WebViewOps `evaluate` 返回 JSON。
2. 快照：`runtime_extract_js` 返回元素 + iframe `frames[]`。
3. 元素操作：type/click/select/checkbox（JS 注入降级路径，`supports_coordinate_input=false`）。
4. 会话持久化：cookie/storage → session_save → 重开 → session_load。
5. 多标签并发：per-tab 锁，多 WebView 实例并行。
6. 性能：`perf.*` 用例出报告（snapshot 均值等）。
