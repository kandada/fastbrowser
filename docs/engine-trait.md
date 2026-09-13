# BrowserEngine trait 契约

`src/engine/trait_.rs` 定义全内核唯一引擎接口。所有引擎（mock/chromium/cef/webview）
必须完整实现；工具层只依赖它。

```rust
pub trait BrowserEngine: Send {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> EngineCapabilities;
    fn cdp_endpoint(&self, tab: TabId) -> Option<String>;   // 真实 CDP 端点（若支持）

    // 标签页
    fn create_tab(&mut self, url: &str, opts: &TabOptions) -> Result<TabId>;
    fn close_tab(&mut self, tab: TabId) -> Result<()>;
    fn list_tabs(&self) -> Vec<TabInfo>;
    fn switch_tab(&mut self, tab: TabId) -> Result<()>;
    fn active_tab(&self) -> Option<TabId>;

    // 导航
    fn navigate(&mut self, tab: TabId, url: &str) -> Result<()>;
    fn back(&mut self, tab: TabId) -> Result<()>;
    fn forward(&mut self, tab: TabId) -> Result<()>;
    fn reload(&mut self, tab: TabId) -> Result<()>;
    fn stop(&mut self, tab: TabId) -> Result<()>;

    // DOM / 内容
    fn snapshot(&mut self, tab: TabId) -> Result<PageSnapshot>;   // ★ 感知入口
    fn get_page_text(&mut self, tab: TabId) -> Result<String>;
    fn get_page_html(&mut self, tab: TabId) -> Result<String>;
    fn get_links(&mut self, tab: TabId) -> Result<Vec<LinkInfo>>;
    fn get_images(&mut self, tab: TabId) -> Result<Vec<ImageInfo>>;
    fn get_table(&mut self, tab: TabId) -> Result<Vec<Vec<String>>>;
    fn execute_xpath(&mut self, tab: TabId, expr: &str) -> Result<Value>;

    // 元素级操作（统一通过 ElementRef）
    fn click_element(&mut self, tab: TabId, element: &ElementRef) -> Result<()>;
    fn set_element_value(&mut self, tab: TabId, element: &ElementRef, value: &str) -> Result<()>;
    fn select_option(&mut self, tab: TabId, element: &ElementRef, value: &str) -> Result<()>;
    fn check_element(&mut self, tab: TabId, element: &ElementRef, checked: bool) -> Result<()>;
    fn set_file_input(&mut self, tab: TabId, element: &ElementRef, paths: &[String]) -> Result<()>;

    fn inject_css(&mut self, tab: TabId, css: &str) -> Result<()>;
    fn inject_event(&mut self, tab: TabId, ev: InputEvent) -> Result<()>;
    fn evaluate(&mut self, tab: TabId, script: &str) -> Result<Value>;

    // 视图 / 渲染
    fn set_viewport(&mut self, tab: TabId, vp: Viewport) -> Result<()>;
    fn screenshot(&mut self, tab: TabId) -> Result<Image>;
    fn view_handle(&self, tab: TabId) -> Option<ViewHandle>;
    fn view_frame(&mut self, tab: TabId) -> Result<ViewFrame>;

    // 会话状态
    fn cookie_get(&self, tab: TabId, domain: Option<&str>) -> Result<Vec<Cookie>>;
    fn cookie_set(&mut self, tab: TabId, cookie: &Cookie) -> Result<()>;
    fn cookie_clear(&mut self, tab: TabId, domain: Option<&str>, name: Option<&str>) -> Result<()>;
    fn storage_get(&self, tab: TabId, key: &str) -> Result<Option<String>>;
    fn storage_set(&mut self, tab: TabId, key: &str, value: &str) -> Result<()>;

    // 网络控制
    fn block_requests(&mut self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()>;
    fn intercept_requests(&mut self, tab: TabId, patterns: &[String], enabled: bool) -> Result<()>;

    // 事件
    fn drain_events(&mut self, tab: TabId) -> Vec<PageEvent>;

    // 宿主回调
    fn set_event_sink(&mut self, tab: TabId, sink: Option<Arc<dyn PageEventSink>>) -> Result<()>;
    fn set_frame_sink(&mut self, tab: TabId, sink: Option<Arc<dyn ViewFrameSink>>) -> Result<()>;
}
```

## 能力位

`EngineCapabilities` 标记引擎能力，工具层据此向 LLM 收敛可用工具：

- `supports_cdp` / `supports_coordinate_input` / `supports_osr`
- `supports_windowed` / `supports_touch` / `supports_dom_injection`
- `supports_storage` / `supports_cookies` / `supports_network_control`

典型值：CEF=full；WebView=`js_injection()`（降级：无 CDP、无坐标输入、无 OSR）。

## 新增引擎步骤

1. 实现 `BrowserEngine`（参考 `engines/webview.rs`，元素级操作走注入 JS）；
2. 在 `engines/mod.rs` 注册 + `create_engine` 分支 + feature；
3. 用 mock 引擎的测试模式补测试。

## 扩展方法（带默认实现的真实浏览器语义）

trait 中以下方法提供**默认实现**（`Unsupported` 或退化），真实引擎（CDP）覆盖：

- **对话框**：`pending_dialog` / `dialog_accept` / `dialog_dismiss`（`Page.javascriptDialogOpening` 驱动）
- **元素动作变体**：`double_click_element` / `right_click_element`（默认退化为单击；CDP 为坐标级真实双击/右击）
- **PDF**：`print_to_pdf`（`Page.printToPDF`）
- **导航历史**：`get_history`（`Page.getNavigationHistory`）
- **无障碍树**：`accessibility_tree`（`Accessibility.getFullAXTree`）
- **隔离上下文**：`create_context` / `dispose_context` / `create_tab_in_context`（`Target.createBrowserContext`）
- **网络拦截**：`pending_requests` / `fulfill_request` / `continue_request` / `abort_request`（`Fetch` 域）

新增引擎只需覆盖需要的能力；其余自动拿到语义正确的降级。
