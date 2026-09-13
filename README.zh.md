# fastbrowser

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

跨平台浏览器自动化内核，为 AI Agent 而生——是 [fastshell](https://github.com/kandada/fastshell) 在浏览器域的"对等体"：*fastshell* 管系统，*fastbrowser* 管浏览器。

## 为什么

AI Agent 需要"看得见、操作得了"网页。fastbrowser 不是简单的 CDP 封装，而是完整的浏览器能力抽象层：引擎无关、LLM 原生工具、会话/状态管理、多语言绑定——与 fastshell 同一哲学（内核做厚、应用做薄）。

## 特性

- **引擎无关** — 一个 `BrowserEngine` trait 抽象 mock / 打包 Chromium / 外部 Chromium（CDP）/ CEF / 系统 WebView，支持 `auto` 自动降级：外部 CDP → 打包 Chromium → WebView → mock。
- **88 个 LLM 原生工具**（14 个功能域）— 全部 JSON 出入、标准 JSON Schema 参数；工具清单即给 LLM 的说明书。
- **快照即感知入口** — `PageSnapshot`（a/b/c 元素编号 + 滚动/截断元信息 + iframe 子框架 + shadow DOM 元素）。
- **真实浏览器语义（CDP）** — 坐标级点击（**Playwright 风格 Actionability**：自动等待「出现→可见→不被遮挡→位置稳定」，遮挡点名，支持跨同源 iframe / open shadow DOM）、**真实键盘输入**（`Input.insertText`，React 受控组件 / IME 兼容）。
- **真实事件流** — 导航 / console / 网络 / JS 对话框 / **下载落盘**（`Browser.setDownloadBehavior`），以及 **OSR 推送帧流**（`Page.startScreencast` + `start_frame_stream`/`stop_frame_stream`）。
- **真实内容** — 真实 PNG 截图、URL 作用域真实 cookie 与 localStorage、`DOM.setFileInputFiles` 文件上传、`Page.printToPDF`、ARIA 无障碍树、`Fetch` 请求拦截（fulfill/continue/abort）。
- **Profile 隔离** — 每个 Profile 独立浏览器上下文（CDP `BrowserContext`），无头 Chrome 亦可用。
- **快照 DOM 缓存** — MutationObserver 脏标记，大页面高频快照免全量重扫。
- **双壳架构** — 同步壳（C ABI / CLI / Kotlin / Swift）与异步壳（`AsyncFastbrowser`，feature `async-core`：async 工具调用、每标签事件流推送、`wait_any`/`wait_for_navigation` 编排、`run_concurrently`/`open_many` 多标签并发）共享同一浏览器实例。
- **默认跨标签** — 所有工具支持 `{"tab": N}`，跨标签操作不再依赖活动标签。
- **跨平台** — 同一份 Rust 内核编译到 Android / iOS / 鸿蒙 / macOS / Windows / Linux。
- **多端接入** — C ABI + Python / Node / Go / Swift / Kotlin 绑定。
- **审计与诊断** — 内置动作审计日志（`audit` / `clear_audit`）与 `FASTBROWSER_LOG=1` 环境变量日志。

## 安装

```toml
[dependencies]
fastbrowser = "0.1.0"
```

## 快速开始

### Rust（mock 引擎——无需浏览器）

```rust
use fastbrowser::{Fastbrowser, Config};

let sdk = Fastbrowser::new();
sdk.init(Config::for_engine("mock"))?;

let page = sdk.open("https://example.com")?;   // {"tab":1,"title":"Example Page","url":...}
let snap = sdk.snapshot()?;                    // 交互元素快照，供 LLM 使用
sdk.tool_call("extract_links", serde_json::json!({}))?;

sdk.shutdown();
```

### 异步（feature `async-core`）

```rust
use fastbrowser::AsyncFastbrowser;

let b = AsyncFastbrowser::new();
b.init(Config::for_engine("mock")).await?;

let t1 = b.open("https://example.com").await?["tab"].as_u64().unwrap() as u32;
let t2 = b.open("https://example.com/login").await?["tab"].as_u64().unwrap() as u32;

// 多标签并发
let (r1, r2) = tokio::join!(
    b.tool_call("get_page_title", serde_json::json!({"tab": t1})),
    b.tool_call("get_page_title", serde_json::json!({"tab": t2})),
);
```

### CLI

```bash
cargo build --release
cargo run --release -- --engine mock open https://example.com   # 打开页面
cargo run --release -- snapshot                                 # 快照（a/b/c 编号）
cargo run --release -- extract_links                            # 提取链接
cargo run --release -- --repl                                   # 交互模式（跨命令保持会话）
cargo run --release -- audit                                    # 查看动作审计日志
```

## 示例

以下示例均用 mock 引擎（`Config::for_engine("mock")`），无需安装浏览器即可运行。

### 快照驱动的交互（核心 Agent 循环）

```rust
use fastbrowser::{Config, Fastbrowser};
use serde_json::json;

let sdk = Fastbrowser::new();
sdk.init(Config::for_engine("mock"))?;
sdk.open("https://example.com")?;

let snap = sdk.snapshot()?;
for el in &snap.interactive {
    println!("[{}] <{}> {:?}", el.id, el.tag, el.text);  // [a] <h1> Some("Welcome...")
}

// 按快照编号操作
sdk.tool_call("click", json!({"id": "c"}))?;
sdk.tool_call("type", json!({"id": "e", "text": "hello"}))?;
sdk.tool_call("press", json!({"key": "Enter"}))?;

// 或直接用元素引用（css / xpath / text）
sdk.tool_call("click", json!({"ref": {"kind": "css", "value": "button"}}))?;
```

### 内容提取

```rust
let links = sdk.tool_call("extract_links", json!({}))?;    // {"links": [{"text":..., "url":...}]}
let text  = sdk.tool_call("get_page_text", json!({}))?;    // {"text": "..."}
let table = sdk.tool_call("extract_table", json!({}))?;    // {"table": [[...], ...]}
let title = sdk.tool_call("get_page_title", json!({}))?;   // {"title": "Example Page"}
```

### 多标签

所有工具支持 `{"tab": N}` —— 跨标签操作不依赖活动标签。

```rust
let t1 = sdk.open("https://example.com")?["tab"].as_u64().unwrap() as u32;
let t2 = sdk.tool_call("new_tab", json!({"url": "https://example.com/login"}))?["tab"]
    .as_u64().unwrap() as u32;

sdk.tool_call("get_page_title", json!({"tab": t1}))?;  // {"title": "Example Page"}
sdk.tool_call("get_page_title", json!({"tab": t2}))?;  // {"title": "Login"}

sdk.tool_call("switch_tab", json!({"tab": t1}))?;
sdk.tool_call("close_tab", json!({"tab": t2}))?;
```

### 表单填写

```rust
sdk.tool_call("fill_form", json!({"values": {"e": "alice"}}))?;   // 按快照编号填写
sdk.tool_call("type", json!({"id": "e", "text": "bob", "clear": true}))?;
sdk.tool_call("checkbox", json!({"id": "d", "checked": true}))?;
```

### 执行 JS 与等待

```rust
let title = sdk.tool_call("execute_js", json!({"script": "document.title"}))?;  // {"result": "Example Page"}
sdk.tool_call("wait_for_element", json!({"selector": "button", "timeout_ms": 5000}))?;
```

### 截图

```rust
let img = sdk.screenshot()?;                 // Image { width, height, rgba }
assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
sdk.set_viewport(390, 844)?;                 // 手机视口
```

### 会话、cookie 与 storage

```rust
sdk.tool_call("cookie_set", json!({"name": "sid", "value": "abc", "domain": "example.com"}))?;
sdk.tool_call("storage_set", json!({"key": "token", "value": "t1"}))?;
sdk.session_save("/tmp/state.json")?;        // 持久化 cookie + storage + 标签

// ……之后在另一个进程
let sdk2 = Fastbrowser::new();
sdk2.init(Config::for_engine("mock"))?;
sdk2.session_load("/tmp/state.json")?;
sdk2.open("https://example.com")?;
let cookies = sdk2.tool_call("cookie_get", json!({"domain": "example.com"}))?;  // {"cookies": [...]}
```

### 审计日志

```rust
let audit = sdk.audit();                     // serde_json::Value（JSON 数组）
sdk.clear_audit();
```

## 引擎

`Config.engine`：`mock`（默认，内存参考引擎）/ `bundled`（打包 Chromium）/ `chromium`（配 `cdp_url`，连外部 Chrome/Edge）/ `cef`（桌面 CEF 嵌入，windowless）/ `webview` / `webkit`（系统 WebView 桥）/ `auto`（自动降级链）。

### 桌面端：打包 Chromium

内核零 Chromium 定制、**不编译 Chromium**，随应用打包现成引擎：

```bash
./scripts/fetch-chromium.sh            # 下载 Chrome for Testing → vendor/chromium（~150MB，已 gitignore）
./scripts/fetch-cef.sh                 # （可选）下载预编译 CEF → vendor/cef（engine-cef 用）
./scripts/package-desktop.sh mac chromium   # 组装桌面应用（内置 Chromium）+ 签名/公证
```

- `engine: "bundled"`：自动发现打包 Chromium，无头启动并走 CDP（Playwright 同款思路）；
- `engine: "cef"`：CEF 离屏嵌入（windowless），同样走 CDP；
- 打包产物可独立运行（Chromium 已在包内，无需用户安装 Chrome）。

### Profile 隔离

设置 `Config.isolated_profiles = true` 即可为每个 `Profile` 创建独立浏览器上下文（CDP `BrowserContext`）——cookie/storage/localStorage 完全隔离。不支持浏览器上下文的引擎自动降级为共享上下文。

## API

```rust
pub struct Config {
    pub engine: String,                    // mock / bundled / chromium / cef / webview / auto
    pub rendering_mode: RenderingMode,     // Headless / Hosted
    pub profile_name: String,              // 多账号隔离
    pub incognito: bool,                   // 关闭时不落盘 cookie/storage
    pub user_agent: Option<String>,
    pub viewport: Option<Viewport>,
    pub proxy: Option<String>,             // 例如 "http://127.0.0.1:7890"
    pub cache_path: Option<String>,
    pub storage_path: Option<String>,
    pub cdp_url: Option<String>,           // 引擎为 "chromium" 时必填
    pub locale: Option<String>,
    pub accept_downloads: bool,
    pub default_download_path: Option<String>,
    pub extra_browser_args: Vec<String>,
    pub command_timeout_ms: u64,           // 0 = 不限
    pub actionability_timeout_ms: u64,     // 元素就绪自动等待（默认 5000）
    pub network_ask_permission: bool,      // 移动端网络授权
    pub auto_accept_dialogs: bool,
    pub isolated_profiles: bool,           // 每 Profile 独立 CDP BrowserContext
}

impl Fastbrowser {
    pub fn new() -> Self;
    pub fn init(&self, config: Config) -> Result<()>;
    pub fn is_initialized(&self) -> bool;
    pub fn open(&self, url: &str) -> Result<Value>;
    pub fn navigate(&self, url: &str) -> Result<Value>;
    pub fn tool_call(&self, name: &str, params: Value) -> Result<Value>;
    pub fn tool_list(&self) -> Value;
    pub fn tool_count(&self) -> usize;
    pub fn snapshot(&self) -> Result<PageSnapshot>;
    pub fn screenshot(&self) -> Result<Image>;
    pub fn set_viewport(&self, width: u32, height: u32) -> Result<()>;
    pub fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<Value>;
    pub fn stop_frame_stream(&self, tab: TabId) -> Result<Value>;
    pub fn set_rendering_mode(&self, mode: RenderingMode);
    pub fn session_save(&self, path: &str) -> Result<Value>;
    pub fn session_load(&self, path: &str) -> Result<Value>;
    pub fn clear_state(&self) -> Result<Value>;
    pub fn register_event_sink(&self, sink: Arc<dyn PageEventSink>);
    pub fn register_frame_sink(&self, sink: Arc<dyn ViewFrameSink>);
    pub fn get_info(&self) -> SdkInfo;
    pub fn status(&self) -> Value;
    pub fn audit(&self) -> Value;
    pub fn clear_audit(&self);
    pub fn shutdown(&self);
}
```

## 真 Chromium 集成测试

```bash
cargo test --features engine-cdp --test chromium_integration              # 无头 Chrome（协议全链路）
cargo test --features engine-cdp --test chromium_headed_integration       # 有头 Chrome（真实窗口/合成器）
cargo test --features "engine-cdp,async-core" --test async_core_e2e       # 异步壳 + 真实 Chrome
cargo test --features engine-cdp --test bundled_chromium_integration      # 打包的 Chromium
```

- **无头**（`chromium_integration`）：快照 / 输入 / 点击 / 提取 / JS / 截图 / 多标签 / 对话框 / 拦截 / 上下文隔离 / Actionability / 真实键盘 / 下载 / 帧流等。
- **有头**（`chromium_headed_integration`，需 GUI 会话）：真实窗口渲染、`document.hidden` 可见性语义、前台/后台标签、有头下载。
- **异步壳**（`async_core_e2e`）：async 工具调用、`open_many`/`run_concurrently` 并发、事件推送、`wait_any` 编排。
- 无对应浏览器或非 GUI 环境时自动 SKIP，不阻塞 CI。

## 绑定

`bindings/` —— python（ctypes / PyO3）、node（N-API）、go（cgo）、swift（C FFI）、android（Kotlin）。均为 C ABI（`fastbrowser_c/`）的薄翻译层。

## 目录

```
src/engine/       BrowserEngine trait + 类型（无平台依赖）
src/engines/      mock / bundled / chromium(cdp) / cef / webview
src/cdp/          CDP 客户端 + 端点发现
src/tools/        88 个 Agent 工具（14 功能域，JSON Schema 参数）
src/session/      Profile / Cookie / Storage / 持久化 + BrowserContext 隔离
src/bridge/       Runtime 装配 + 动作审计日志
src/sdk/          对外 SDK + C ABI（ffi.rs）
src/png/          零依赖 PNG 解码器（真实截图）
fastbrowser_c/    C ABI 头文件 + JNI 胶水 + CMake
bindings/         python / node / go / swift / android
docs/             设计文档
scripts/          fetch-chromium / fetch-cef / package-desktop / build-android / gen-c-headers
```

编译产物（`target/`、`dist/`、`vendor/`）已 gitignore，位于仓库内（可用 `CARGO_TARGET_DIR` / `VENDOR_DIR` 覆盖）。

## 文档

- 架构：`docs/architecture.md`
- 接口文档：`docs/api-reference.zh.md`（Config / SDK / 类型 / 引擎契约 / 88 个工具全清单）
- 引擎契约：`docs/engine-trait.md`
- 快照规范：`docs/snapshot.md`
- 工具协议：`docs/tool-protocol.md`
- C ABI：`docs/c-api.md`、`fastbrowser_c/include/fastbrowser.h`

## 设计理念

- **引擎无关** — 一个 trait、多种引擎；LLM 看到的是统一工具集，与平台无关。
- **AI 原生** — JSON Schema 参数 + JSON 输出；快照是感知入口，工具清单是说明书。
- **真实浏览器语义** — Playwright 风格 Actionability 与真实输入，而非模拟 DOM 事件。
- **跨平台** — 同一份 Rust 内核，Android / iOS / 鸿蒙 / macOS / Windows / Linux。
- **薄绑定** — 各语言绑定只翻译 C ABI，不承载业务逻辑。

## 许可证

Apache 2.0 © xiefujin (490021684@qq.com)

Chromium/CEF 为 BSD，随包分发需保留 license 与 `about:license`。
