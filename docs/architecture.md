# fastbrowser 架构

## 分层

```
┌─ 应用层（宿主 App，独立仓库）───────────────────────────────┐
│  Android(Kotlin) · iOS(Swift) · 桌面(Tauri) · Python/Node/Go  │
│            │  进程内链接 C ABI（libfastbrowser.{a,dylib,so}） │
├─ 内核仓库 fastbrowser/（Apache-2.0）─────────────────────────────┤
│  async_core/ 异步壳 AsyncFastbrowser（feature async-core）    │
│      async 工具调用(spawn_blocking) + 每标签事件流广播         │
│      + 编排（wait_any/wait_for_navigation）+ 多标签并发       │
│  sdk/      对外 SDK + C ABI（Fastbrowser 结构 / ffi / plugin）│
│  bridge/   Runtime 装配（引擎 + 会话 + 工具 + 审计）          │
│  tools/    88 个 Agent 工具（13 功能域，JSON Schema 参数）     │
│  session/  Profile / Cookie / Storage / BrowserContext 隔离   │
│  engine/   BrowserEngine trait + 类型（无平台依赖）           │
│  engines/  mock / bundled / chromium(cdp) / cef / webview     │
│  cdp/      共享 CDP 客户端（engine-cdp）                      │
│  png/      零依赖 PNG 解码器（真实截图）                       │
└──────────────────────────────────────────────────────────────┘
```

## 双壳架构（异步核心 + 同步壳 + 异步壳）

| 壳 | 形态 | 调用方 | 机制 |
|---|---|---|---|
| **同步壳** | `Fastbrowser` / C ABI / CLI / bindings | 移动端宿主、桌面壳、脚本 | 引擎操作直接同步；CDP 传输内部异步 + `block_on` |
| **异步壳** | `AsyncFastbrowser`（`async_core/`） | async agent runtime | async fn；引擎操作 `spawn_blocking`（不阻塞执行器）；事件推送 + 编排为纯 tokio |

- **共享实例**：双壳包同一个 `Fastbrowser`；异步壳持 `Arc<Fastbrowser>`，引擎操作调度到 tokio 阻塞池
  （per-tab 锁 + CDP 流水线保证不同标签页在阻塞池上真并发）。
- **事件推送**：后台泵任务（tokio task）把每标签页的 `PageEvent` 推送到 `tokio::sync::broadcast`；
  **仅当存在订阅者时才抽取事件**，避免与同步壳 `drain_events` 争抢。
- **编排**：`wait_for_event` / `wait_any`（`tokio::time::timeout` + select 语义）、`wait_for_navigation` /
  `wait_for_load_state` / `wait_for_condition`（状态轮询 + 事件双通道）、`run_concurrently` / `open_many`（`JoinSet`）。
- **工具多标签化**：所有工具接受 `{"tab": N}`（`ToolContext::target_tab`），跨标签操作不依赖活动标签。

## 设计原则

1. **引擎无关，协议统一**：`BrowserEngine` trait 是唯一契约；工具层只面向它。
   新增平台 = 新增一个引擎实现，工具层零改动。
2. **内核做厚，应用做薄**：浏览器控制、会话、快照、工具全在内核；宿主只做 UI。
3. **AI 原生**：工具入参/出参全 JSON（标准 JSON Schema），`description` 即给 LLM 的说明书；
   `PageSnapshot`（a/b/c 编号 + meta 滚动/截断提示）是感知入口。
4. **本地优先**：敏感数据（cookie/storage/历史）默认不出设备。

## 真实浏览器语义（CDP 引擎）

CDP 引擎不再只是 JS 注入封装，而是完整驱动真实浏览器：

- **点击**：Playwright 风格 Actionability（自动等待「出现→可见→不被遮挡→位置稳定」）→ 元素中心点 →
  `elementFromPoint` hit-test → 被遮挡时**点名遮挡者** → `Input.dispatchMouseEvent`（真实鼠标事件，含双击/右击）；
  支持跨同源 iframe 与 open shadow DOM（坐标沿 iframe 链换算、遮挡判定沿 shadow host 链验证）。
- **输入**：`type` 走真实键盘通道 `Input.insertText`（React 受控组件 / IME 兼容），不再只是 `el.value=` 直赋。
- **快照**：V3 注入 JS 递归采集同源 iframe（`frames[]`）+ open shadow DOM，跨域 iframe 记为 `cross_origin` 帧；
  `PageSnapshot` 带 `meta`（滚动/截断）。MutationObserver 脏标记做 **DOM 缓存**：页面未变时快照复用缓存，
  大页面高频快照免全量重扫（滚动/URL 变化也会触发重扫）。
- **截图**：`Page.captureScreenshot` → 自研零依赖 PNG 解码器（`src/png.rs`，flate2 inflate + 去滤波 + 色彩转换）→ RGBA 位图；
  另有 `capture_encoded` 直出 png/jpeg 字节。
- **帧流**：`Page.startScreencast`（PNG）+ 后台泵线程 → `frame_sink`，`start_frame_stream`/`stop_frame_stream`
  （预览窗实时画面）；非 screencast 事件经 `CdpClient::reingest` 回灌，不打断 `drain_events` 拉取语义。
- **事件**：`Page/Runtime/Network/Console` 事件按 `sessionId` 路由到对应标签页（`session_to_tab`），转成 `PageEvent` 供 `drain_events` / `wait_*` 消费。
- **Cookie/Storage**：URL 作用域 `Network.getCookies`/`setCookie`/`deleteCookies` + 真实 `localStorage`（不跨浏览器上下文泄漏）。
- **文件上传**：`DOM.querySelector` 定位 `[data-fb]` → `DOM.setFileInputFiles`。
- **对话框**：`Page.javascriptDialogOpening` → `dialog_accept`/`dialog_dismiss`。
- **PDF**：`Page.printToPDF`。
- **下载**：`Browser.setDownloadBehavior`（`accept_downloads` + `default_download_path`）→ 无头 Chrome 也落盘。
- **无障碍**：`Accessibility.getFullAXTree` → `get_accessibility_tree`。
- **网络拦截**：`Fetch.enable` → `requestPaused` 收集 → `fulfill/continue/abort` 工具（自定义响应体真机已验证）。
- **多账号隔离**：`Target.createBrowserContext` → `Config.isolated_profiles` 时每 Profile 独立上下文；非默认上下文用 `newWindow` + `Page.bringToFront`/`setWebLifecycleState` 创建并激活，**无头 Chrome 亦可用**（有头更快；无头下后台窗口渲染器会冻结、真实导航慢，但 cookie/上下文隔离不受影响）。

## 引擎矩阵

| feature | 引擎 | 平台 | 控制通道 |
|---------|------|------|---------|
| `engine-mock`（默认） | 内存参考引擎 | 全平台 | 脚本化 DOM |
| `engine-cdp` | Chromium/CDP 引擎 | 连真实 Chrome/Chromium/CEF | CDP（remote-debugging） |
| `engine-cef` | CEF 嵌入 | Win/macOS/Linux | CDP（engine-cdp 之上的宿主） |
| `engine-webview` | 系统 WebView | Android/iOS/鸿蒙 | 宿主 `WebViewOps` + 注入 JS |

`Config.engine` 可选：`mock` / `chromium`（需 `cdp_url`）/ `cef` / `webview`。
真 Chromium 集成测试见 `tests/chromium_integration.rs`。

详见 `engine-trait.md`、`snapshot.md`、`tool-protocol.md`、`c-api.md`。

## 依赖方向（不可违反）

```
engine ← tools ← bridge ← sdk ← bin / fastbrowser_c / bindings
engine ← engines（mock/cef/webview）
cdp 独立，被 cef 复用
```

- `engine/` 不依赖任何引擎/平台库；
- `tools/` 只依赖 `engine` 的 trait；
- 新增平台只加 `engines/` 适配。
