# fastbrowser 接口文档（API Reference）

> 版本：0.1.0 · 本文档描述内核对外暴露的全部接口：配置、同步/异步 SDK、核心数据类型、
> 引擎契约与 88 个 Agent 工具。工具契约以 JSON-Schema 形式生成（`tool_list()`），本文档为
> 人类可读的完整参考。

## 目录

1. [总览](#1-总览)
2. [配置 Config](#2-配置-config)
3. [同步 SDK：Fastbrowser](#3-同步-sdkfastbrowser)
4. [异步 SDK：AsyncFastbrowser](#4-异步-sdkasyncfastbrowser)
5. [核心数据类型](#5-核心数据类型)
6. [引擎契约：BrowserEngine](#6-引擎契约browserengine)
7. [工具清单（88 个）](#7-工具清单95-个)
8. [错误模型](#8-错误模型)
9. [C ABI](#9-c-abi)

---

## 1. 总览

内核分四层，对外只暴露最上层的三个入口：

```
应用 / 宿主 / 语言绑定
  ├── 同步壳  Fastbrowser        （src/sdk）         ← 默认
  ├── 异步壳  AsyncFastbrowser   （src/async_core）  ← feature "async-core"
  └── C ABI   fastbrowser_*      （src/sdk/ffi）     ← 供非 Rust 宿主
                │
        Runtime（src/bridge）——引擎 + 会话 + 工具 + 审计
                │
     ┌──────────┼──────────┐
  BrowserEngine trait   88 个工具（14 域）
     ┌────┼────┐
   mock  cdp   webview  （+ bundled/cef）
```

- **所有工具都接受 `{"tab": N}` 参数**，跨标签操作不依赖活动标签。
- **所有工具 JSON 出入**：输入是 `params` 对象，输出是结果对象（或 `{"error": {...}}`）。
- 引擎无关：上层只依赖 `BrowserEngine` trait，新增平台只加一个引擎实现。

---

## 2. 配置 Config

`init(config)` 的入参，字段均可省略（`#[serde(default)]`），部分 JSON 即可覆盖默认值。

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `engine` | string | `"mock"` | `mock` / `bundled` / `chromium` / `cef` / `webview` / `webkit` / `auto` |
| `rendering_mode` | string | `"headless"` | `"hosted"`（托管/有窗口）或 `"headless"`（离屏） |
| `profile_name` | string | `"default"` | Profile 名，多账号隔离 |
| `incognito` | bool | `false` | 无痕：关闭不落盘 cookie/storage |
| `user_agent` | string? | `null` | 自定义 UA |
| `viewport` | object? | `null` | 初始视口 `{width, height, device_scale_factor}` |
| `proxy` | string? | `null` | HTTP 代理，如 `"http://127.0.0.1:7890"` |
| `cache_path` | string? | `null` | 磁盘缓存目录（空则内存缓存） |
| `storage_path` | string? | `null` | 会话状态落盘目录（incognito 忽略） |
| `cdp_url` | string? | `null` | 引擎 `"chromium"` 时必填的 CDP 端点 |
| `locale` | string? | `null` | 浏览器 locale，如 `"zh-CN"` |
| `accept_downloads` | bool | `false` | 自动接受下载 |
| `default_download_path` | string? | `null` | 默认下载目录 |
| `extra_browser_args` | string[] | `[]` | 传给底层浏览器的额外启动参数 |
| `command_timeout_ms` | u64 | `0` | 工具调用超时（毫秒），0 = 不限 |
| `actionability_timeout_ms` | u64 | `5000` | 元素就绪（出现→可见→不遮挡→稳定）自动等待上限 |
| `network_ask_permission` | bool | `false` | 移动端网络访问先询问宿主 |
| `auto_accept_dialogs` | bool | `true` | 自动接受无必要弹窗/下载 |
| `isolated_profiles` | bool | `false` | 每 Profile 独立 CDP BrowserContext |

---

## 3. 同步 SDK：Fastbrowser

进程级单例，所有方法 `&self`，多线程可共享并发操作不同标签页。

```rust
pub struct Fastbrowser { /* 内部 RwLock<Runtime> */ }

impl Fastbrowser {
    pub fn new() -> Self;
    pub fn init(&self, config: Config) -> Result<()>;
    pub fn is_initialized(&self) -> bool;
    pub fn shutdown(&self);

    // 导航
    pub fn open(&self, url: &str) -> Result<Value>;     // {"tab","title","url"}
    pub fn navigate(&self, url: &str) -> Result<Value>;

    // 工具
    pub fn tool_call(&self, name: &str, params: Value) -> Result<Value>;
    pub fn tool_list(&self) -> Value;                   // 88 个工具 JSON 数组
    pub fn tool_count(&self) -> usize;

    // 感知 / 截图
    pub fn snapshot(&self) -> Result<PageSnapshot>;
    pub fn screenshot(&self) -> Result<Image>;
    pub fn set_viewport(&self, width: u32, height: u32) -> Result<()>;

    // 渲染 / 帧流
    pub fn get_view(&self) -> Option<ViewHandle>;
    pub fn set_rendering_mode(&self, mode: RenderingMode);
    pub fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<Value>;
    pub fn stop_frame_stream(&self, tab: TabId) -> Result<Value>;

    // 会话
    pub fn session_save(&self, path: &str) -> Result<Value>;
    pub fn session_load(&self, path: &str) -> Result<Value>;
    pub fn clear_state(&self) -> Result<Value>;

    // 回调（锁外独立线程派发，可重入）
    pub fn register_event_sink(&self, sink: Arc<dyn PageEventSink>);
    pub fn register_frame_sink(&self, sink: Arc<dyn ViewFrameSink>);

    // 状态 / 审计
    pub fn get_info(&self) -> SdkInfo;
    pub fn status(&self) -> Value;
    pub fn audit(&self) -> Value;
    pub fn clear_audit(&self);
}
```

---

## 4. 异步 SDK：AsyncFastbrowser

feature `async-core`。与同步壳共享同一 `Fastbrowser` 实例；引擎操作经 tokio 阻塞池调度，
事件推送与编排为纯 tokio 异步。

```rust
pub struct AsyncFastbrowser { /* Arc<Fastbrowser> + 事件广播 */ }

impl AsyncFastbrowser {
    pub fn spawn(inner: Arc<Fastbrowser>) -> Self;          // 需在 tokio runtime 内
    pub fn inner(&self) -> &Arc<Fastbrowser>;
    pub fn subscribe_events(&self) -> broadcast::Receiver<(TabId, PageEvent)>;
    pub fn has_event_subscribers(&self) -> bool;

    pub async fn init(&self, config: Config) -> Result<Value>;
    pub async fn shutdown(&self);

    pub async fn open(&self, url: String) -> Result<Value>;
    pub async fn navigate(&self, url: String) -> Result<Value>;
    pub async fn tool_call(&self, name: String, params: Value) -> Result<Value>;
    pub async fn snapshot(&self) -> Result<PageSnapshot>;
    pub async fn snapshot_on(&self, tab: TabId) -> Result<PageSnapshot>;
    pub async fn screenshot(&self) -> Result<Image>;
    pub async fn screenshot_on(&self, tab: TabId) -> Result<Image>;
    pub async fn set_viewport(&self, width: u32, height: u32) -> Result<()>;
    pub async fn session_save(&self, path: String) -> Result<Value>;
    pub async fn session_load(&self, path: String) -> Result<Value>;
    pub async fn clear_state(&self) -> Result<Value>;
    pub async fn start_frame_stream(&self, tab: TabId, opts: FrameStreamOptions) -> Result<Value>;
    pub async fn stop_frame_stream(&self, tab: TabId) -> Result<Value>;

    // 同步的廉价读取
    pub fn tool_list(&self) -> Value;
    pub fn tool_count(&self) -> usize;
    pub fn status(&self) -> Value;
    pub fn audit(&self) -> Value;

    // 异步事件 / 编排
    pub async fn page_url(&self, tab: TabId) -> Result<String>;
    pub async fn page_title(&self, tab: TabId) -> Result<String>;
    pub async fn drain_events(&self, tab: TabId) -> Vec<PageEvent>;
    pub async fn run_concurrently(&self, tasks: Vec<(String, Value)>) -> Result<Vec<Value>>;
    pub async fn open_many(&self, urls: Vec<String>) -> Result<Vec<Value>>;
    pub async fn wait_for_event(&self, tab: TabId, pred, timeout) -> ...;
    pub async fn wait_any(&self, tabs, timeout) -> ...;
    pub async fn wait_for_navigation(&self, tab: TabId, timeout: Duration) -> Result<()>;
    pub async fn wait_for_load_state(&self, tab: TabId, state, timeout) -> Result<()>;
    pub async fn wait_for_condition(&self, tab: TabId, script, timeout) -> Result<()>;
}
```

---

## 5. 核心数据类型

### PageSnapshot（快照，LLM 感知入口）

```rust
pub struct PageSnapshot {
    pub title: String,
    pub url: String,
    pub viewport: Viewport,
    pub interactive: Vec<InteractiveElement>,   // 只含可交互元素，a/b/c… 编号
    pub frames: Vec<FrameSnapshot>,             // iframe 子框架
    pub timestamp_ms: u64,
    pub meta: SnapshotMeta,                     // 滚动 / 截断 / stale 元信息
}

pub struct InteractiveElement {
    pub id: char,                    // 'a'..'z'
    pub tag: String,
    pub role: Option<String>,        // ARIA role
    pub text: Option<String>,
    pub href: Option<String>,
    pub rect: Rect,
    pub refs: Vec<ElementRef>,       // css/xpath/text 兜底引用
    pub attrs: HashMap<String, String>,
    pub value: Option<String>,       // 输入框当前值
    pub input_type: Option<String>,
    pub checked: Option<bool>,
    pub selectable_options: Option<Vec<String>>,
    pub selected_option: Option<String>,
    pub visible: bool,
}

pub struct SnapshotMeta {
    pub total: usize, pub truncated: bool,
    pub viewport_h: u32, pub scroll_h: u32, pub scroll_y: u32,
    pub stale: bool,   // true = 可能过期，应等待后重拍
}
```

### ElementRef（元素引用）

```rust
pub struct ElementRef { pub kind: RefKind, pub value: String }
pub enum RefKind { Css, Xpath, Text, Snapshot }
// 工具里对应 {"ref": {"kind": "css|xpath|text", "value": "..."}}
```

### 图像 / 帧 / 视口

```rust
pub struct Image    { pub width: u32, pub height: u32, pub rgba: Vec<u8> }  // RGBA8
pub struct ViewFrame{ pub width: u32, pub height: u32, pub rgba: Vec<u8>, pub seq: u64 }
pub struct Viewport { pub width: u32, pub height: u32, pub device_scale_factor: f64 }
pub struct Rect     { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }
pub enum  ViewHandle { Native(u64), Osr(u64) }
pub enum  RenderingMode { Hosted, Headless }
pub struct FrameStreamOptions { pub fps: u32, pub max_width: u32, pub max_height: u32 }
```

### 标签页 / 会话

```rust
pub struct TabId(u32);
pub struct ContextId(u32);          // 隔离浏览器上下文（对应 CDP BrowserContext）
pub struct TabInfo { pub id: TabId, pub url: String, pub title: String,
                     pub loading: bool, pub pinned: bool, pub created_ms: u64 }
pub struct TabOptions { pub active: bool, pub user_agent: Option<String>,
                        pub viewport: Option<Viewport>, pub incognito: bool,
                        pub referrer: Option<String> }
pub struct HistoryEntry { pub url: String, pub title: String, pub transition: String }
```

### Cookie

```rust
pub struct Cookie {
    pub name: String, pub value: String, pub domain: String, pub path: String,
    pub expires: Option<i64>, pub secure: bool, pub http_only: bool,
    pub same_site: Option<String>,
}
```

### 页面事件 PageEvent

```rust
pub enum PageEvent {
    NavigationStarted { url }, NavigationCompleted { url, status }, Loaded { url },
    TitleChanged { title }, DomChanged,
    Console { level, message }, Request { url, method }, Response { url, status },
    TabOpened { tab }, TabClosed { tab }, Download { url },
    Dialog { message, kind }, Error { message },
}
// to_json() → {"type": "navigation_completed", "url": ..., "status": 200}
```

### DialogInfo / SdkInfo

```rust
pub struct DialogInfo { pub message: String, pub kind: String, pub default_prompt: Option<String> }
pub struct SdkInfo { pub version, pub platform, pub engine: String, pub initialized: bool,
                     pub tabs: usize, pub active_tab: Option<u32>, pub profiles: usize, pub tools: usize }
```

---

## 6. 引擎契约：BrowserEngine

所有具体引擎（`mock` / `chromium` / `cef` / `webview`）实现本 trait，工具层只依赖此抽象。

```rust
pub trait BrowserEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> EngineCapabilities;
    fn cdp_endpoint(&self, tab: TabId) -> Option<String>;

    // 标签页
    fn create_tab(&self, url, opts) -> Result<TabId>;
    fn close_tab(&self, tab) -> Result<()>;
    fn list_tabs(&self) -> Vec<TabInfo>;
    fn switch_tab(&self, tab) -> Result<()>;
    fn active_tab(&self) -> Option<TabId>;

    // 导航
    fn navigate(&self, tab, url) -> Result<()>;
    fn back / forward / reload / stop(&self, tab) -> Result<()>;

    // DOM / 内容
    fn snapshot(&self, tab) -> Result<PageSnapshot>;
    fn get_page_text / get_page_html / get_links / get_images / get_table / execute_xpath;

    // 元素级操作（webview JS 注入也基于此）
    fn click_element / set_element_value / select_option / check_element / set_file_input;
    fn inject_css / inject_event / evaluate;

    // 视图 / 渲染
    fn set_viewport / screenshot / view_handle / view_frame / capture_encoded;

    // 轻量内省（默认走整页快照，引擎可覆盖为廉价读取）
    fn page_title(&self, tab) -> Result<String>;
    fn page_url(&self, tab) -> Result<String>;

    // 会话状态
    fn cookie_get / cookie_set / cookie_clear / storage_get / storage_set / storage_all / storage_clear;

    // 网络控制
    fn block_requests(&self, tab, patterns, enabled) -> Result<()>;
    fn intercept_requests(&self, tab, patterns, enabled) -> Result<()>;

    // 事件（拉取模型 + 事件驱动等待）
    fn drain_events(&self, tab) -> Vec<PageEvent>;
    fn event_generation(&self, tab) -> u64;
    fn wait_event(&self, tab, since, timeout) -> (u64, bool);
    fn any_event_generation(&self) -> u64;
    fn wait_any_event(&self, since, timeout) -> (u64, bool);

    // 宿主回调
    fn set_event_sink / set_frame_sink(&self, tab, Option<Arc<...>>) -> Result<()>;

    // JS 对话框（默认 Unsupported）
    fn pending_dialog(&self, tab) -> Option<DialogInfo>;
    fn dialog_accept(&self, tab, prompt_text) -> Result<()>;
    fn dialog_dismiss(&self, tab) -> Result<()>;

    // 元素动作变体（默认退化为单击）
    fn double_click_element / right_click_element;

    // 真实键盘输入（默认退化为 set_element_value）
    fn type_text(&self, tab, element, text, clear) -> Result<()>;

    // 离屏帧流（默认 Unsupported）
    fn start_frame_stream / stop_frame_stream;

    // PDF / 历史 / AX（默认 Unsupported）
    fn print_to_pdf(&self, tab) -> Result<Vec<u8>>;
    fn get_history(&self, tab) -> Result<Vec<HistoryEntry>>;
    fn accessibility_tree(&self, tab) -> Result<Value>;

    // 隔离上下文（默认 Unsupported）
    fn create_context / dispose_context / create_tab_in_context;

    // 请求拦截处理（默认 Unsupported）
    fn pending_requests(&self, tab) -> Vec<Value>;
    fn fulfill_request(&self, tab, request_id, status, body, headers) -> Result<()>;
    fn continue_request / abort_request;
}
```

### EngineCapabilities（能力位）

工具层依据这些位向 LLM 收敛可用工具。

```rust
pub struct EngineCapabilities {
    pub supports_cdp: bool,
    pub supports_coordinate_input: bool,   // 坐标级输入（真实引擎有，webview 无）
    pub supports_osr: bool,
    pub supports_windowed: bool,
    pub supports_touch: bool,
    pub supports_dom_injection: bool,      // webview JS 注入路径
    pub supports_storage: bool,
    pub supports_cookies: bool,
    pub supports_network_control: bool,    // webview 无
}
// EngineCapabilities::full()          —— 真实 Chromium/CDP
// EngineCapabilities::js_injection()  —— 移动端 webview 降级
```

---

## 7. 工具清单（88 个）

参数列标 `*` 为必填；`id` 为快照编号（a..z），`ref` 为 `{kind: css|xpath|text|role, value}`。所有工具都可用 `{"tab": N}` 指定目标标签页（缺省用活动标签）。

### 7.1 导航（6）

| 工具 | 参数 | 说明 / 返回 |
|------|------|------------|
| `navigate` | `url*` | 导航到 URL，返回 `{"url","title"}` |
| `back` | — | 后退（历史首条 no-op） |
| `forward` | — | 前进（末条 no-op） |
| `reload` | — | 刷新 |
| `stop` | — | 停止加载 |
| `get_history` | — | 返回 `{"history":[{url,title,transition}]}` |

### 7.2 交互（13）

| 工具 | 参数 | 说明 |
|------|------|------|
| `click` | `id` 或 `ref` | 点击，返回元素信息 |
| `dblclick` | `id` 或 `ref` | 双击 |
| `right_click` | `id` 或 `ref` | 右键 |
| `type` | `text*`, `id`/`ref`, `clear`=true | 真实键盘输入（`Input.insertText`，React/IME 兼容） |
| `press` | `key*` | 单键，如 `Enter`/`Tab`/`ArrowDown` |
| `send_keys` | `keys*` | 组合键，如 `"Ctrl+a"`/`"Shift+Enter"` |
| `hover` | `id` 或 `ref` | 鼠标移动到元素中心 |
| `drag` | `from*`, `to*` | 从元素拖到元素 |
| `scroll` | `dx`=0, `dy`=100 | 滚动（正 = 下/右） |
| `swipe` | `from_x*`, `from_y*`, `to_x*`, `to_y*` | 触摸滑动 |
| `focus` | `id` 或 `ref` | 聚焦 |
| `blur` | — | 失焦 |
| `clear_input` | `id` 或 `ref` | 清空输入框 |

### 7.3 提取（8）

| 工具 | 参数 | 返回 |
|------|------|------|
| `extract_text` | `id`? | `{"text"}`（整页或单元素可见文本） |
| `extract_html` | — | `{"html"}` |
| `extract_links` | — | `{"links":[{url,text}]}` |
| `extract_images` | — | `{"images":[{src,alt,width,height}]}` |
| `extract_table` | — | `{"table":[[...]]}`（首个数据表） |
| `extract_json` | `script*` | 执行 JS 返回 JSON 结果 |
| `search` | `query*`, `regex`=false, `limit`=20 | `{"matches":[...]}` 文本搜索 |
| `find_elements` | `selector*`, `limit`=20 | CSS 选择器查元素 |

### 7.4 等待与断言（9）

| 工具 | 参数 | 说明 |
|------|------|------|
| `wait_for_element` | `id` 或 `selector`, `timeout_ms`=5000 | 等待元素出现 |
| `wait_for_navigation` | `timeout_ms`=10000 | 等待导航完成 |
| `wait_for_load_state` | `state`=`load`/`domcontentloaded`/`networkidle`, `timeout_ms`=10000 | 等待加载态 |
| `wait_for_condition` | `script*`, `timeout_ms`=5000 | 等待 JS 谓词为真 |
| `wait_for_text` | `text*`, `timeout_ms`=5000 | 等待可见文本包含子串 |
| `assert_element_exists` | `id` 或 `selector` | 断言存在，失败报错 |
| `assert_text_contains` | `text*` | 断言文本包含子串 |
| `assert_url_contains` | `contains*` | 断言 URL 包含子串 |
| `assert_title` | `contains*` | 断言标题包含子串 |

### 7.5 表单（6）

| 工具 | 参数 | 说明 |
|------|------|------|
| `fill_form` | `values*`（快照 id → 文本） | 批量填充，返回 `{"filled":[...]}` |
| `select_option` | `value*`, `id`/`ref` | 选中 `<select>` 选项 |
| `upload_file` | `paths*`, `id`/`ref` | `<input type=file>` 设文件 |
| `checkbox` | `id`/`ref`, `checked`=true | 设置勾选态 |
| `radio` | `id`/`ref` | 选中单选钮 |
| `extract_forms` | — | 列出所有表单控件 |

### 7.6 页面信息（17）

| 工具 | 参数 | 返回 / 说明 |
|------|------|------------|
| `screenshot` | `format`=`rgba`/`png`/`jpeg` | base64 截图 |
| `screenshot_element` | `id` 或 `ref` | 元素裁剪截图（base64 RGBA） |
| `get_page_title` | — | `{"title"}` |
| `get_current_url` | — | `{"url"}` |
| `get_page_text` | — | `{"text"}` |
| `get_element_info` | `id` 或 `ref` | 元素详情（tag/role/text/rect/attrs/value） |
| `get_element_text` | `id` 或 `ref` | `{"text"}` |
| `get_attributes` | `id` 或 `ref` | `{"attributes":{...}}` |
| `is_visible` | `id` 或 `ref` | `{"visible":bool}` |
| `is_enabled` | `id` 或 `ref` | `{"enabled":bool}` |
| `get_focused_element` | — | 当前聚焦元素 |
| `get_selected_text` | — | 当前选中文本 |
| `get_page_meta` | — | title/description/keywords/canonical/og |
| `get_scroll_position` | — | `{"x","y","scroll_height"}` |
| `set_scroll_position` | `x`=0, `y`=0 | 绝对滚动 |
| `get_performance_metrics` | — | 性能计时指标 |
| `get_accessibility_tree` | — | AX 树（LLM 原生感知格式） |

### 7.7 Cookie 与 Storage（8）

| 工具 | 参数 | 说明 |
|------|------|------|
| `cookie_get` | `domain`? | `{"cookies":[{name,value,domain,path,...}]}` |
| `cookie_set` | `name*`, `value*`, `domain*`, `path`?/`expires`?/`secure`?/`http_only`?/`same_site`? | 设置 cookie |
| `cookie_clear` | `domain`?, `name`? | 按域/名清除 |
| `clear_cookies` | — | 清空全部 cookie |
| `storage_get` | `key*` | `{"value"}`（localStorage） |
| `storage_set` | `key*`, `value*` | 写 localStorage |
| `storage_get_all` | — | `{"items":{...}}` 全部 |
| `clear_storage` | — | 清空 localStorage |

### 7.8 高级（6）

| 工具 | 参数 | 说明 |
|------|------|------|
| `execute_js` | `script*` | 页面上下文执行 JS，返回 `{"result"}` |
| `evaluate_xpath` | `expr*` | XPath 求值（如 `//a`、`count(//button)`） |
| `inject_css` | `css*` | 注入 `<style>` |
| `block_request` | `patterns*`, `enabled`=true | 屏蔽匹配 URL 的请求 |
| `intercept_request` | `patterns*`, `enabled`=true | 拦截匹配请求（配合 7.11） |
| `set_basic_auth` | `username*`, `password*` | 设置 Basic Auth 凭据，401 挑战自动应答 |

### 7.9 标签页（8）

| 工具 | 参数 | 说明 |
|------|------|------|
| `new_tab` | `url*` | 开新标签并激活，返回 `{"tab":N}` |
| `new_window` | `url*` | 开独立浏览器窗口并激活，返回 `{"tab":N}` |
| `close_tab` | `tab`? | 关闭标签（缺省活动标签） |
| `switch_tab` | `tab*` | 切换活动标签 |
| `list_tabs` | — | `{"tabs":[{id,url,title,loading,pinned}]}` |
| `get_active_tab` | — | 当前绑定的标签（"我在哪个标签"）：`{"tab":N,"url","title","target_id"}` |
| `get_tab` | `url_contains`? 或 `title_contains`? | 按 URL/标题查找 |
| `duplicate_tab` | — | 复制活动标签 |
| `close_other_tabs` | — | 关闭除活动标签外的所有 |

### 7.10 对话框（3）

| 工具 | 参数 | 说明 |
|------|------|------|
| `pending_dialog` | — | 当前待处理对话框，无则 `null` |
| `dialog_accept` | `prompt_text`? | 接受（prompt 可带输入） |
| `dialog_dismiss` | — | 取消 |

### 7.11 网络拦截（5）

配合 `intercept_request` 使用，`request_id` 来自 `list_pending_requests`。

| 工具 | 参数 | 说明 |
|------|------|------|
| `list_pending_requests` | — | 被拦截请求列表 `{request_id,url,method,post_data?}` |
| `fulfill_request` | `request_id*`, `status`=200, `body_b64`?, `headers`? | 自定义响应放行 |
| `modify_response` | `request_id*`, `status`=200, `body_b64`?, `headers`? | 放行但修改响应（状态码/响应体/头） |
| `continue_request` | `request_id*` | 原样放行 |
| `abort_request` | `request_id*` | 中止（客户端失败） |

### 7.12 PDF（1）

| 工具 | 参数 | 说明 |
|------|------|------|
| `save_as_pdf` | `path*` | 保存 PDF，返回 `{"path","size"}` |

### 7.13 设备模拟（3）

| 工具 | 参数 | 说明 |
|------|------|------|
| `set_touch_emulation` | `enabled*` | 开启/关闭触摸模拟 |
| `set_geolocation` | `latitude*`, `longitude*`, `accuracy`=100 | 覆盖地理位置 |
| `set_timezone` | `timezone_id*` | 覆盖时区（IANA ID） |

### 7.14 Agent 辅助（2）

| 工具 | 参数 | 说明 |
|------|------|------|
| `done` | `answer*` | 任务结束，返回最终答案给用户 |
| `export_replay` | — | 导出审计轨迹为可回放 Python 脚本 |

---

## 8. 错误模型

统一错误 `EngineError { kind, message }`，可 JSON 序列化，便于 C ABI / LLM 消费。

```rust
pub enum ErrorKind {
    TabNotFound, Navigation, Snapshot, View, Input, Evaluate, Dom,
    Io, Timeout, Unsupported, Plugin, InvalidArgument, NotInitialized, Internal,
}
// to_json() → {"error": {"kind": "tab_not_found", "message": "..."}}
// Display  → "fastbrowser[tab_not_found]: ..."
// Result<T> = std::result::Result<T, EngineError>
```

工具调用失败时，CLI / C ABI / Python 层返回统一的 JSON 错误结构 `{"error":{...}}`；
Rust 层直接返回 `Err(EngineError)`。

---

## 9. C ABI

导出符号为全局单例 + JSON 字符串出入 + `fastbrowser_free_string` 释放，详见 [`c-api.md`](c-api.md)。

| 符号 | 说明 |
|------|------|
| `fastbrowser_version()` | 版本号 |
| `fastbrowser_init(config_json)` / `fastbrowser_shutdown()` | 初始化 / 关闭 |
| `fastbrowser_open(url)` / `fastbrowser_navigate(url)` | 导航 |
| `fastbrowser_tool_call(name, params_json)` | 工具调用 |
| `fastbrowser_tool_list()` | 工具清单 |
| `fastbrowser_snapshot()` / `fastbrowser_screenshot()` | 快照 / 截图 |
| `fastbrowser_get_view()` / `fastbrowser_set_viewport(w,h)` | 视图 / 视口 |
| `fastbrowser_start_frame_stream(tab,...)` / `fastbrowser_stop_frame_stream(tab)` | 帧流 |
| `fastbrowser_get_info()` / `fastbrowser_status()` | 信息 / 状态 |
| `fastbrowser_audit()` / `fastbrowser_clear_audit()` | 审计 |
| `fastbrowser_set_permission(resource, allowed)` | 移动端权限 |
| `fastbrowser_register_event_callback(cb)` / `fastbrowser_register_viewframe_callback(cb)` | 回调 |
| `fastbrowser_register_webview_ops(ops)` | WebView 宿主桥 |
| `fastbrowser_free_string(ptr)` | 释放返回字符串 |
