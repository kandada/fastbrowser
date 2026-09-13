# fastbrowser

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

A cross-platform browser-automation kernel built for AI agents — the browser-domain counterpart of [fastshell](https://github.com/kandada/fastshell): *fastshell* manages the system, *fastbrowser* manages the browser.

## Why

AI agents need to *see* and *operate* web pages. Instead of a thin CDP wrapper, fastbrowser is a full browser-capability abstraction layer: engine-agnostic, LLM-native tools, session/state management, and multi-language bindings — the same philosophy as fastshell ("thick kernel, thin app").

## Features

- **Engine-agnostic** — a single `BrowserEngine` trait abstracts mock / bundled Chromium / external Chromium (CDP) / CEF / system WebView, with `auto` fallback: external CDP → bundled Chromium → WebView → mock.
- **88 LLM-native tools** (14 domains) — every tool is JSON-in/JSON-out with standard JSON-Schema params; the tool manifest *is* the LLM's instruction manual.
- **Snapshot as the perception entry** — `PageSnapshot` with a/b/c element ids, scroll/truncation meta, iframe frames, and shadow-DOM elements.
- **Real-browser semantics (CDP)** — coordinate-level click with **Playwright-style actionability** (auto-waits for visible, not-covered, stable; reports the blocker; works across same-origin iframes and open shadow roots), **real keyboard input** (`Input.insertText`, React-controlled-input & IME compatible).
- **Real event stream** — navigation / console / network / JS dialogs / **downloads-to-disk** (`Browser.setDownloadBehavior`), plus **push OSR frame streaming** (`Page.startScreencast` + `start_frame_stream`/`stop_frame_stream`).
- **Real content** — real PNG screenshots, URL-scoped real cookies & localStorage, file upload via `DOM.setFileInputFiles`, `Page.printToPDF`, ARIA accessibility tree, `Fetch` request interception with fulfill/continue/abort.
- **Profile isolation** — per-profile isolated browser contexts (CDP `BrowserContext`), works in headless Chrome too.
- **Snapshot DOM caching** — MutationObserver dirty-flag avoids full re-scans on big pages.
- **Dual-shell architecture** — the sync shell (C ABI / CLI / Kotlin / Swift) and the async shell (`AsyncFastbrowser`, feature `async-core`: async tool calls, per-tab event-stream push, `wait_any`/`wait_for_navigation` orchestration, `run_concurrently`/`open_many` multi-tab concurrency) share the same browser instance.
- **Cross-tab by default** — every tool accepts `{"tab": N}`; cross-tab operations no longer depend on the active tab.
- **Cross-platform** — one Rust core compiles to Android / iOS / HarmonyOS / macOS / Windows / Linux.
- **Multi-language** — C ABI + Python / Node / Go / Swift / Kotlin bindings.
- **Audit & diagnostics** — built-in action audit log (`audit` / `clear_audit`) and env-gated logging (`FASTBROWSER_LOG=1`).

## Installation

```toml
[dependencies]
fastbrowser = "0.1.0"
```

## Quick Start

### Rust (mock engine — no browser required)

```rust
use fastbrowser::{Fastbrowser, Config};

let sdk = Fastbrowser::new();
sdk.init(Config::for_engine("mock"))?;

let page = sdk.open("https://example.com")?;   // {"tab":1,"title":"Example Page","url":...}
let snap = sdk.snapshot()?;                    // interactive-element snapshot for LLMs
sdk.tool_call("extract_links", serde_json::json!({}))?;

sdk.shutdown();
```

### Async (feature `async-core`)

```rust
use fastbrowser::AsyncFastbrowser;

let b = AsyncFastbrowser::new();
b.init(Config::for_engine("mock")).await?;

let t1 = b.open("https://example.com").await?["tab"].as_u64().unwrap() as u32;
let t2 = b.open("https://example.com/login").await?["tab"].as_u64().unwrap() as u32;

// concurrent multi-tab
let (r1, r2) = tokio::join!(
    b.tool_call("get_page_title", serde_json::json!({"tab": t1})),
    b.tool_call("get_page_title", serde_json::json!({"tab": t2})),
);
```

### CLI

```bash
cargo build --release
cargo run --release -- --engine mock open https://example.com   # open a page
cargo run --release -- snapshot                                 # snapshot (a/b/c ids + meta)
cargo run --release -- extract_links                            # extract links
cargo run --release -- --repl                                   # interactive REPL (session persists)
cargo run --release -- audit                                    # inspect the action audit trail
```

## Examples

All examples below use the mock engine (`Config::for_engine("mock")`), so they run with no browser installed.

### Snapshot-driven interaction (the core agent loop)

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

// act on snapshot ids
sdk.tool_call("click", json!({"id": "c"}))?;
sdk.tool_call("type", json!({"id": "e", "text": "hello"}))?;
sdk.tool_call("press", json!({"key": "Enter"}))?;

// or use an element ref directly (css / xpath / text)
sdk.tool_call("click", json!({"ref": {"kind": "css", "value": "button"}}))?;
```

### Content extraction

```rust
let links = sdk.tool_call("extract_links", json!({}))?;    // {"links": [{"text":..., "url":...}]}
let text  = sdk.tool_call("get_page_text", json!({}))?;    // {"text": "..."}
let table = sdk.tool_call("extract_table", json!({}))?;    // {"table": [[...], ...]}
let title = sdk.tool_call("get_page_title", json!({}))?;   // {"title": "Example Page"}
```

### Multi-tab

Every tool accepts `{"tab": N}` — cross-tab operations don't depend on the active tab.

```rust
let t1 = sdk.open("https://example.com")?["tab"].as_u64().unwrap() as u32;
let t2 = sdk.tool_call("new_tab", json!({"url": "https://example.com/login"}))?["tab"]
    .as_u64().unwrap() as u32;

sdk.tool_call("get_page_title", json!({"tab": t1}))?;  // {"title": "Example Page"}
sdk.tool_call("get_page_title", json!({"tab": t2}))?;  // {"title": "Login"}

sdk.tool_call("switch_tab", json!({"tab": t1}))?;
sdk.tool_call("close_tab", json!({"tab": t2}))?;
```

### Form filling

```rust
sdk.tool_call("fill_form", json!({"values": {"e": "alice"}}))?;   // fill by snapshot id
sdk.tool_call("type", json!({"id": "e", "text": "bob", "clear": true}))?;
sdk.tool_call("checkbox", json!({"id": "d", "checked": true}))?;
```

### Execute JS & wait

```rust
let title = sdk.tool_call("execute_js", json!({"script": "document.title"}))?;  // {"result": "Example Page"}
sdk.tool_call("wait_for_element", json!({"selector": "button", "timeout_ms": 5000}))?;
```

### Screenshot

```rust
let img = sdk.screenshot()?;                 // Image { width, height, rgba }
assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
sdk.set_viewport(390, 844)?;                 // mobile viewport
```

### Session, cookies & storage

```rust
sdk.tool_call("cookie_set", json!({"name": "sid", "value": "abc", "domain": "example.com"}))?;
sdk.tool_call("storage_set", json!({"key": "token", "value": "t1"}))?;
sdk.session_save("/tmp/state.json")?;        // persist cookies + storage + tabs

// ... later, in another process
let sdk2 = Fastbrowser::new();
sdk2.init(Config::for_engine("mock"))?;
sdk2.session_load("/tmp/state.json")?;
sdk2.open("https://example.com")?;
let cookies = sdk2.tool_call("cookie_get", json!({"domain": "example.com"}))?;  // {"cookies": [...]}
```

### Audit trail

```rust
let audit = sdk.audit();                     // serde_json::Value (a JSON array)
sdk.clear_audit();
```

## Engines

`Config.engine`: `mock` (default, in-memory reference) / `bundled` (vendored Chromium) / `chromium` (external Chrome/Edge via `cdp_url`) / `cef` (desktop CEF embedding, windowless) / `webview` / `webkit` (system WebView bridge) / `auto` (degradation chain).

### Desktop: bundled Chromium

The kernel does **zero Chromium customization and never compiles Chromium** — it ships a prebuilt engine alongside the app:

```bash
./scripts/fetch-chromium.sh            # download Chrome for Testing → vendor/chromium (~150MB, git-ignored)
./scripts/fetch-cef.sh                 # (optional) download prebuilt CEF → vendor/cef (for engine-cef)
./scripts/package-desktop.sh mac chromium   # assemble the desktop app (with Chromium inside) + codesign/notarize
```

- `engine: "bundled"` — auto-discovers the vendored Chromium, launches headless, drives it over CDP (same approach as Playwright).
- `engine: "cef"` — CEF windowless embedding, also driven over CDP.
- The packaged product runs standalone (Chromium ships inside the app; no user-installed Chrome required).

### Profile isolation

Set `Config.isolated_profiles = true` to give each `Profile` its own isolated browser context (CDP `BrowserContext`) — cookies/storage/localStorage are fully separated between profiles. On engines without browser-context support it gracefully falls back to shared contexts.

## API

```rust
pub struct Config {
    pub engine: String,                    // mock / bundled / chromium / cef / webview / auto
    pub rendering_mode: RenderingMode,     // Headless / Hosted
    pub profile_name: String,              // multi-account isolation
    pub incognito: bool,                   // no cookie/storage persistence on exit
    pub user_agent: Option<String>,
    pub viewport: Option<Viewport>,
    pub proxy: Option<String>,             // e.g. "http://127.0.0.1:7890"
    pub cache_path: Option<String>,
    pub storage_path: Option<String>,
    pub cdp_url: Option<String>,           // required for engine "chromium"
    pub locale: Option<String>,
    pub accept_downloads: bool,
    pub default_download_path: Option<String>,
    pub extra_browser_args: Vec<String>,
    pub command_timeout_ms: u64,           // 0 = no limit
    pub actionability_timeout_ms: u64,     // element-ready auto-wait (default 5000)
    pub network_ask_permission: bool,      // mobile network authorization
    pub auto_accept_dialogs: bool,
    pub isolated_profiles: bool,           // per-profile CDP BrowserContext
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

## Real-Chromium integration tests

```bash
cargo test --features engine-cdp --test chromium_integration              # headless Chrome (full protocol)
cargo test --features engine-cdp --test chromium_headed_integration       # headed Chrome (real window/compositor)
cargo test --features "engine-cdp,async-core" --test async_core_e2e       # async shell + real Chrome
cargo test --features engine-cdp --test bundled_chromium_integration      # vendored Chromium
```

- **Headless** (`chromium_integration`): snapshot / type / click / extract / JS / screenshot / multi-tab / dialogs / interception / context isolation / actionability / real keyboard / downloads / frame stream.
- **Headed** (`chromium_headed_integration`, needs a GUI session): real window compositor rendering, `document.hidden` visibility semantics, foreground/background tabs, headed downloads.
- **Async shell** (`async_core_e2e`): async tool calls, `open_many`/`run_concurrently` concurrency, event push, `wait_any` orchestration.
- All auto-SKIP when no browser / no GUI is available, so CI stays green without one.

## Bindings

`bindings/` — python (PyO3; published to PyPI as `fastbrowser`), node (N-API), go (cgo), swift (C FFI), android (Kotlin). Each is a thin translation of the C ABI (`fastbrowser_c/`).

## Directory

```
src/engine/       BrowserEngine trait + types (no platform deps)
src/engines/      mock / bundled / chromium(cdp) / cef / webview
src/cdp/          CDP client + endpoint discovery
src/tools/        88 Agent tools (14 domains, JSON-Schema params)
src/session/      Profile / Cookie / Storage / persistence + BrowserContext isolation
src/bridge/       Runtime assembly + action audit log
src/sdk/          Public SDK + C ABI (ffi.rs)
src/png/          dependency-free PNG decoder (real screenshots)
fastbrowser_c/    C ABI header + JNI glue + CMake
bindings/         python / node / go / swift / android
docs/             design docs
scripts/          fetch-chromium / fetch-cef / package-desktop / build-android / gen-c-headers
```

Build artifacts (`target/`, `dist/`, `vendor/`) are git-ignored and live inside the repo (override with `CARGO_TARGET_DIR` / `VENDOR_DIR`).

## Docs

- Architecture: `docs/architecture.md`
- API reference: `docs/api-reference.md` (Config / SDK / types / engine contract / all 88 tools)
- Engine contract: `docs/engine-trait.md`
- Snapshot spec: `docs/snapshot.md`
- Tool protocol: `docs/tool-protocol.md`
- C ABI: `docs/c-api.md`, `fastbrowser_c/include/fastbrowser.h`

## Design Principles

- **Engine-agnostic** — one trait, many engines; the LLM sees a uniform toolset regardless of platform.
- **AI-native** — JSON-Schema params and JSON outputs; the snapshot is the perception entry, the tool manifest is the instruction manual.
- **Real-browser semantics** — Playwright-style actionability and real input, not simulated DOM events.
- **Cross-platform** — the same Rust core on Android / iOS / HarmonyOS / macOS / Windows / Linux.
- **Thin bindings** — every language binding is a translation of the C ABI, never business logic.

## License

Apache 2.0 © xiefujin (490021684@qq.com)

Chromium/CEF are BSD; keep their license and the `about:license` entry when redistributing.
