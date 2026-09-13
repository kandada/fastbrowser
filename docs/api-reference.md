# fastbrowser API Reference

> Version 0.1.0 · This document describes every interface the kernel exposes: configuration,
> sync/async SDKs, core data types, the engine contract, and all 88 Agent tools.

## 1. Overview

The kernel is layered; only the top-level entry points are exposed:

```
Host app / bindings
  ├── Sync SDK  Fastbrowser        (src/sdk)         ← default
  ├── Async SDK AsyncFastbrowser   (src/async_core)  ← feature "async-core"
  └── C ABI    fastbrowser_*       (src/sdk/ffi)     ← non-Rust hosts
                │
        Runtime (src/bridge) — engine + session + tools + audit
                │
     ┌──────────┼──────────┐
  BrowserEngine trait   88 tools (14 domains)
     ┌────┼────┐
   mock  cdp   webview  (+ bundled/cef)
```

- **Every tool accepts `{"tab": N}`** — cross-tab operations don't depend on the active tab.
- **Every tool is JSON-in/JSON-out** — input is a `params` object, output is a result object
  (or `{"error": {...}}`).
- Engine-agnostic: upper layers only depend on the `BrowserEngine` trait.

## 2. Configuration

`Config` is the `init()` argument; all fields are optional (`#[serde(default)]`).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `engine` | string | `"mock"` | `mock` / `bundled` / `chromium` / `cef` / `webview` / `webkit` / `auto` |
| `rendering_mode` | string | `"headless"` | `"hosted"` (windowed) or `"headless"` (offscreen) |
| `profile_name` | string | `"default"` | Profile name for multi-account isolation |
| `incognito` | bool | `false` | Don't persist cookies/storage on exit |
| `user_agent` | string? | `null` | Custom User-Agent |
| `viewport` | object? | `null` | Initial `{width, height, device_scale_factor}` |
| `proxy` | string? | `null` | HTTP proxy, e.g. `"http://127.0.0.1:7890"` |
| `cache_path` | string? | `null` | Disk cache dir (empty = memory cache) |
| `storage_path` | string? | `null` | Session state dir (ignored when incognito) |
| `cdp_url` | string? | `null` | Required for engine `"chromium"` |
| `locale` | string? | `null` | Browser locale, e.g. `"zh-CN"` |
| `accept_downloads` | bool | `false` | Auto-accept downloads |
| `default_download_path` | string? | `null` | Default download dir |
| `extra_browser_args` | string[] | `[]` | Extra browser launch args |
| `command_timeout_ms` | u64 | `0` | Tool-call timeout (ms), 0 = no limit |
| `actionability_timeout_ms` | u64 | `5000` | Element-ready auto-wait (visible/not-covered/stable) |
| `network_ask_permission` | bool | `false` | Ask host before network access (mobile) |
| `auto_accept_dialogs` | bool | `true` | Auto-accept dialogs/downloads |
| `isolated_profiles` | bool | `false` | Per-profile CDP BrowserContext |

## 3. Sync SDK: Fastbrowser

Process-wide singleton; all methods take `&self`, so multiple threads share one instance
and operate on different tabs concurrently.

```rust
impl Fastbrowser {
    pub fn new() -> Self;
    pub fn init(&self, config: Config) -> Result<()>;
    pub fn is_initialized(&self) -> bool;
    pub fn shutdown(&self);

    pub fn open(&self, url: &str) -> Result<Value>;      // {"tab","title","url"}
    pub fn navigate(&self, url: &str) -> Result<Value>;

    pub fn tool_call(&self, name: &str, params: Value) -> Result<Value>;
    pub fn tool_list(&self) -> Value;                    // capabilities-filtered tool list
    pub fn tool_count(&self) -> usize;

    pub fn snapshot(&self) -> Result<PageSnapshot>;
    pub fn screenshot(&self) -> Result<Image>;
    pub fn set_viewport(&self, width: u32, height: u32) -> Result<()>;

    pub fn get_view(&self) -> Option<ViewHandle>;
    pub fn set_rendering_mode(&self, mode: RenderingMode);
    pub fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<Value>;
    pub fn stop_frame_stream(&self, tab: TabId) -> Result<Value>;

    pub fn session_save(&self, path: &str) -> Result<Value>;
    pub fn session_load(&self, path: &str) -> Result<Value>;
    pub fn clear_state(&self) -> Result<Value>;

    pub fn register_event_sink(&self, sink: Arc<dyn PageEventSink>);
    pub fn register_frame_sink(&self, sink: Arc<dyn ViewFrameSink>);

    pub fn get_info(&self) -> SdkInfo;
    pub fn status(&self) -> Value;
    pub fn audit(&self) -> Value;
    pub fn audit_script(&self) -> String;                 // replayable Python script
    pub fn clear_audit(&self);
}
```

## 4. Async SDK: AsyncFastbrowser

Feature `async-core`. Shares the same `Fastbrowser` instance; engine ops run on tokio's
blocking pool, event push and orchestration are pure tokio async.

```rust
impl AsyncFastbrowser {
    pub fn spawn(inner: Arc<Fastbrowser>) -> Self;       // must run inside a tokio runtime
    pub fn subscribe_events(&self) -> broadcast::Receiver<(TabId, PageEvent)>;

    pub async fn init(&self, config: Config) -> Result<Value>;
    pub async fn shutdown(&self);
    pub async fn open(&self, url: String) -> Result<Value>;
    pub async fn navigate(&self, url: String) -> Result<Value>;
    pub async fn tool_call(&self, name: String, params: Value) -> Result<Value>;
    pub async fn snapshot(&self) -> Result<PageSnapshot>;
    pub async fn snapshot_on(&self, tab: TabId) -> Result<PageSnapshot>;
    pub async fn screenshot(&self) -> Result<Image>;
    pub async fn set_viewport(&self, w: u32, h: u32) -> Result<()>;
    pub async fn session_save / session_load / clear_state;
    pub async fn start_frame_stream / stop_frame_stream;

    pub fn tool_list(&self) -> Value;
    pub fn tool_count(&self) -> usize;
    pub fn status(&self) -> Value;
    pub fn audit(&self) -> Value;

    pub async fn page_url / page_title / drain_events;
    pub async fn run_concurrently(&self, tasks: Vec<(String, Value)>) -> Result<Vec<Value>>;
    pub async fn open_many(&self, urls: Vec<String>) -> Result<Vec<Value>>;
    pub async fn wait_for_event / wait_any / wait_for_navigation / wait_for_load_state / wait_for_condition;
}
```

## 5. Core Data Types

```rust
pub struct PageSnapshot { pub title: String, pub url: String, pub viewport: Viewport,
                          pub interactive: Vec<InteractiveElement>, pub frames: Vec<FrameSnapshot>,
                          pub timestamp_ms: u64, pub meta: SnapshotMeta }

pub struct InteractiveElement { pub id: char, pub tag: String, pub role: Option<String>,
    pub text: Option<String>, pub href: Option<String>, pub rect: Rect,
    pub refs: Vec<ElementRef>, pub attrs: HashMap<String,String>, pub value: Option<String>,
    pub input_type: Option<String>, pub checked: Option<bool>,
    pub selectable_options: Option<Vec<String>>, pub selected_option: Option<String>, pub visible: bool }
// InteractiveElement::matches_ref(&ElementRef) — matches by any ref kind (incl. role)

pub enum RefKind { Css, Xpath, Text, Snapshot, Role }
pub struct ElementRef { pub kind: RefKind, pub value: String }

pub struct Image { pub width: u32, pub height: u32, pub rgba: Vec<u8> }
pub struct ViewFrame { pub width: u32, pub height: u32, pub rgba: Vec<u8>, pub seq: u64 }
pub struct Viewport { pub width: u32, pub height: u32, pub device_scale_factor: f64 }
pub struct Rect { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }
pub enum ViewHandle { Native(u64), Osr(u64) }
pub enum RenderingMode { Hosted, Headless }
pub struct FrameStreamOptions { pub fps: u32, pub max_width: u32, pub max_height: u32 }

pub struct TabId(u32); pub struct ContextId(u32);
pub struct TabInfo { pub id: TabId, pub url: String, pub title: String,
                     pub loading: bool, pub pinned: bool, pub created_ms: u64 }
pub struct TabOptions { pub active: bool, pub user_agent: Option<String>,
                        pub viewport: Option<Viewport>, pub incognito: bool, pub referrer: Option<String> }

pub struct Cookie { pub name: String, pub value: String, pub domain: String, pub path: String,
                    pub expires: Option<i64>, pub secure: bool, pub http_only: bool,
                    pub same_site: Option<String> }

pub enum PageEvent { NavigationStarted{url}, NavigationCompleted{url,status}, Loaded{url},
    TitleChanged{title}, DomChanged, Console{level,message}, Request{url,method},
    Response{url,status}, TabOpened{tab}, TabClosed{tab}, Download{url},
    Dialog{message,kind}, Error{message} }

pub struct DialogInfo { pub message: String, pub kind: String, pub default_prompt: Option<String> }
pub struct SdkInfo { pub version, platform, engine: String, initialized: bool,
                     tabs: usize, active_tab: Option<u32>, profiles: usize, tools: usize }
```

## 6. Engine Contract: BrowserEngine

Every engine (`mock` / `chromium` / `cef` / `webview`) implements this trait.

```rust
pub trait BrowserEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> EngineCapabilities;
    fn cdp_endpoint(&self, tab: TabId) -> Option<String>;

    // tabs
    fn create_tab / close_tab / list_tabs / switch_tab / active_tab;
    fn new_window(&self, url, opts) -> Result<TabId>;   // default = create_tab
    // navigation
    fn navigate / back / forward / reload / stop;
    // DOM / content
    fn snapshot / get_page_text / get_page_html / get_links / get_images / get_table / execute_xpath;
    // element-level ops
    fn click_element / set_element_value / select_option / check_element / set_file_input;
    fn inject_css / inject_event / evaluate;
    // view / render
    fn set_viewport / screenshot / view_handle / view_frame / capture_encoded;
    // device emulation (CDP; default Unsupported)
    fn set_touch_emulation / set_geolocation / set_timezone;
    // session state
    fn cookie_get / cookie_set / cookie_clear / storage_get / storage_set / storage_all / storage_clear;
    // network
    fn block_requests / intercept_requests;
    fn set_basic_auth(&self, tab, username, password) -> Result<()>;  // default Unsupported
    // events (pull model + event-driven wait)
    fn drain_events / event_generation / wait_event / any_event_generation / wait_any_event;
    // host callbacks
    fn set_event_sink / set_frame_sink;
    // JS dialogs (default Unsupported)
    fn pending_dialog / dialog_accept / dialog_dismiss;
    // element action variants (default = single click)
    fn double_click_element / right_click_element;
    // real keyboard (default = set_element_value)
    fn type_text(&self, tab, element, text, clear) -> Result<()>;
    // OSR frame stream (default Unsupported)
    fn start_frame_stream / stop_frame_stream;
    // PDF / history / AX (default Unsupported)
    fn print_to_pdf / get_history / accessibility_tree;
    // isolated contexts (default Unsupported)
    fn create_context / dispose_context / create_tab_in_context;
    // request interception (default Unsupported)
    fn pending_requests / fulfill_request / modify_response / continue_request / abort_request;
}
```

### EngineCapabilities

```rust
pub struct EngineCapabilities {
    pub supports_cdp: bool,
    pub supports_coordinate_input: bool,
    pub supports_osr: bool,
    pub supports_windowed: bool,
    pub supports_touch: bool,
    pub supports_dom_injection: bool,
    pub supports_storage: bool,
    pub supports_cookies: bool,
    pub supports_network_control: bool,
}
// EngineCapabilities::full()         — real Chromium/CDP
// EngineCapabilities::js_injection() — mobile webview (degraded)
// EngineCapabilities::supports(Capability) — Capability::{Cdp, NetworkControl}
```

## 7. Tool Manifest (88 tools, 14 domains)

Tools requiring CDP-only capabilities are hidden from `tool_list()` when the engine doesn't
support them (see capability filtering). The 12 capability-gated tools are: `block_request`,
`intercept_request`, `list_pending_requests`, `fulfill_request`, `modify_response`,
`continue_request`, `abort_request`, `save_as_pdf`, `set_touch_emulation`, `set_geolocation`,
`set_timezone`, `set_basic_auth`.

### Navigation (6)
`navigate` `back` `forward` `reload` `stop` `get_history`

### Interaction (13)
`click` `dblclick` `right_click` `type` `press` `send_keys` `hover` `drag` `scroll` `swipe` `focus` `blur` `clear_input`
— most accept `{"id": "a"}` or `{"ref": {"kind": "css|xpath|text|role", "value": "..."}}`

### Extraction (8)
`extract_text` `extract_html` `extract_links` `extract_images` `extract_table` `extract_json` `search` `find_elements`

### Wait & Assert (9)
`wait_for_element` `wait_for_navigation` `wait_for_load_state` `wait_for_condition` `wait_for_text`
`assert_element_exists` `assert_text_contains` `assert_url_contains` `assert_title`

### Forms (6)
`fill_form` `select_option` `upload_file` `checkbox` `radio` `extract_forms`

### Page info (17)
`screenshot` `screenshot_element` `get_page_title` `get_current_url` `get_page_text` `get_element_info`
`get_element_text` `get_attributes` `is_visible` `is_enabled` `get_focused_element` `get_selected_text`
`get_page_meta` `get_scroll_position` `set_scroll_position` `get_performance_metrics` `get_accessibility_tree`

### Cookies & Storage (8)
`cookie_get` `cookie_set` `cookie_clear` `clear_cookies` `storage_get` `storage_set` `storage_get_all` `clear_storage`

### Advanced (6)
`execute_js` `evaluate_xpath` `inject_css` `block_request` `intercept_request` `set_basic_auth`

### Tabs (8)
`new_tab` `new_window` `close_tab` `switch_tab` `list_tabs` `get_tab` `duplicate_tab` `close_other_tabs`

### Dialogs (3)
`pending_dialog` `dialog_accept` `dialog_dismiss`

### Network interception (5)
`list_pending_requests` `fulfill_request` `modify_response` `continue_request` `abort_request`

### PDF (1)
`save_as_pdf`

### Device emulation (3)
`set_touch_emulation` `set_geolocation` `set_timezone`

### Agent helpers (2)
`done` `export_replay`

## 8. Error Model

```rust
pub enum ErrorKind { TabNotFound, Navigation, Snapshot, View, Input, Evaluate, Dom,
                     Io, Timeout, Unsupported, Plugin, InvalidArgument, NotInitialized, Internal }
pub struct EngineError { pub kind: ErrorKind, pub message: String }
// to_json() → {"error": {"kind": "tab_not_found", "message": "..."}}
// Display  → "fastbrowser[tab_not_found]: ..."
pub type Result<T> = std::result::Result<T, EngineError>;
```

## 9. C ABI

Global singleton, JSON strings in/out, `fastbrowser_free_string` to release. See
[`c-api.md`](c-api.md).

| Symbol | Description |
|--------|-------------|
| `fastbrowser_version()` | version |
| `fastbrowser_init(json)` / `fastbrowser_shutdown()` | lifecycle |
| `fastbrowser_open(url)` / `fastbrowser_navigate(url)` | navigation |
| `fastbrowser_tool_call(name, params)` | tool call |
| `fastbrowser_tool_list()` | tool manifest |
| `fastbrowser_snapshot()` / `fastbrowser_screenshot()` | perception |
| `fastbrowser_get_view()` / `fastbrowser_set_viewport(w,h)` | view |
| `fastbrowser_start_frame_stream(tab,...)` / `fastbrowser_stop_frame_stream(tab)` | frame stream |
| `fastbrowser_get_info()` / `fastbrowser_status()` | info |
| `fastbrowser_audit()` / `fastbrowser_clear_audit()` | audit |
| `fastbrowser_set_permission(resource, allowed)` | mobile permissions |
| `fastbrowser_register_event_callback(cb)` / `fastbrowser_register_viewframe_callback(cb)` | callbacks |
| `fastbrowser_register_webview_ops(ops)` | WebView host bridge |
| `fastbrowser_free_string(ptr)` | free returned string |
