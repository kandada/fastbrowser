# 鸿蒙 HarmonyOS（P2，ArkWeb）真机测试规划

ArkWeb 基于 Chromium，但 DevTools/CDP 支持不透明。第一优先级走 **JS 注入通道**（`runJavaScript`），
经 `WebViewOps` 桥复用内核全部逻辑（与 iOS WKWebView 同理）。

## 路线

| 阶段 | 内容 | 状态 |
|---|---|---|
| 1 | ArkTS 实现 `WebViewOps`（重点 `evaluate` 能返回 JSON 字符串）→ 内核 `engine-webview` | 📄 规划 |
| 2 | 复用 `shared/cases/*.json`：鸿蒙侧 CaseRunner（ArkTS）逐条执行 | 📄 规划 |
| 3 | 探测 ArkWeb 对注入脚本（`querySelectorAll('*')` + `getBoundingClientRect`）的性能/权限限制 | 📄 规划 |
| 4 | 长期：调查 `hdc shell`/DevTools 映射 CDP 的可行性 | 📄 规划 |

## 接入点

- C ABI：`fastbrowser_register_webview_ops(&FbWebViewOps)`（与 Android/iOS 相同）。
- ArkTS 经 Node-API / C FFI 调用（鸿蒙支持 C++ 与 ArkTS 混合）。
- `engine: "webview"` 或 `engine: "auto"`（有 ops 即选 webview，无则落 mock 不崩）。

## 验证清单（对应 real-device-test-plan §4）

1. 注入链路：open → snapshot（`data-fb` 标注）→ type/click 生效。
2. 能力收敛：`supports_cdp=false`，工具清单向 LLM 收敛。
3. 降级验证：无 ops 时 `auto` 引擎落到 mock 不崩。
