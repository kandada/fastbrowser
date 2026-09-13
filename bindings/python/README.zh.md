# fastbrowser

[fastbrowser](../fastbrowser) 的 Python 绑定 —— 一个用 Rust 编写的跨平台浏览器自动化内核，面向 AI Agent。本包通过 PyO3 暴露内核。

- **引擎无关**：mock（零依赖）/ `chromium`（CDP）/ `bundled`（Chrome for Testing）/ `webview` / `cef`，统一在一个 `BrowserEngine` trait 之后。
- **AI 原生**：95 个 LLM 友好工具（`navigate`、`click`、`type`、`extract_text`、`wait_for_element`、`fill_form`、`screenshot`……），全部 JSON 出入。
- **多标签并发**：每标签独立锁 —— 不同标签并行、同一标签串行（对应真实浏览器的"每进程 / 单主线程"模型）。

## 安装

```bash
pip install fastbrowser
```

> 预编译 wheel 已发布到 CPython 3.8+ 的 macOS（arm64/x86_64）、Linux（manylinux x86_64/aarch64）与 Windows x64。

## 快速开始（mock 引擎 —— 无需浏览器）

```python
import fastbrowser as fb

b = fb.FastBrowser()
b.init()                                    # 默认：mock 引擎，零依赖

out = b.open("https://example.com")
print(out["title"])                         # "Example Page"
print(b.snapshot()["title"])                # 交互元素快照，供 LLM 使用
print(b.tool_call("get_current_url"))       # {"url": "https://example.com"}

w, h, rgba = b.screenshot()                 # 原始 RGBA（w, h, w*h*4 字节）
b.save_screenshot("page.png")               # PNG

b.shutdown()
```

## 示例

### 1. 快照驱动的交互（核心 Agent 循环）

`snapshot()` 返回 LLM 操作的可交互元素（a/b/c 编号），`click`/`type` 按编号操作：

```python
b = fb.FastBrowser(); b.init()
b.open("https://example.com")

snap = b.snapshot()
for el in snap["interactive"]:
    print(el["id"], el["tag"], el.get("text"), el.get("href"))

# 点击链接、输入文本、按回车
b.tool_call("click", {"id": "c"})
b.tool_call("type", {"id": "e", "text": "hello"})
b.tool_call("press", {"key": "Enter"})
```

### 2. 内容提取

```python
b = fb.FastBrowser(); b.init()
b.open("https://example.com")

print(b.tool_call("extract_links"))       # {"links": [{"text": "Learn more", "url": "..."}]}
print(b.tool_call("extract_text"))        # {"text": "..."}
print(b.tool_call("get_page_text"))       # {"text": "..."}
print(b.tool_call("extract_table"))       # {"table": [["Name", "Value"], ...]}
print(b.tool_call("get_page_title"))      # {"title": "Example Page"}
```

### 3. 表单填写

```python
b = fb.FastBrowser(); b.init()
b.open("https://example.com")

# 按快照编号批量填写
b.tool_call("fill_form", {"values": {"e": "alice"}})   # {"filled": ["e"], "ok": true}

# 或逐字段操作
b.tool_call("type", {"id": "e", "text": "bob", "clear": True})
b.tool_call("checkbox", {"id": "d", "checked": True})

# 真实引擎上：下拉 / 单选 / 文件上传
b.tool_call("select_option", {"id": "e", "value": "pro"})
b.tool_call("radio", {"id": "r"})
```

### 4. 多标签

所有工具支持 `{"tab": N}` —— 跨标签操作不依赖活动标签：

```python
b = fb.FastBrowser(); b.init()

t1 = b.open("https://example.com")["tab"]
t2 = b.tool_call("new_tab", {"url": "https://example.com/login"})["tab"]

print(b.tool_call("list_tabs"))                         # {"tabs": [1, 2]}
print(b.tool_call("get_page_title", {"tab": t1}))       # {"title": "Example Page"}
print(b.tool_call("get_page_title", {"tab": t2}))       # {"title": "Login"}

b.tool_call("switch_tab", {"tab": t1})
b.tool_call("close_tab", {"tab": t2})
```

### 5. 等待

```python
b.tool_call("wait_for_element", {"selector": "button", "timeout_ms": 5000})
b.tool_call("wait_for_navigation", {"timeout_ms": 10000})
```

### 6. 执行 JavaScript

```python
print(b.tool_call("execute_js", {"script": "document.title"}))            # {"result": "Example Page"}
# 真实引擎上可对真实页面执行任意脚本：
print(b.tool_call("execute_js", {"script": "document.links.length"}))     # {"result": <数量>}
```

### 7. 截图

```python
w, h, rgba = b.screenshot()          # 原始 RGBA
png = b.screenshot_png()             # PNG 字节（纯 stdlib 编码，无需 Pillow）
b.save_screenshot("page.png")        # 保存 PNG 到磁盘
b.set_viewport(390, 844)             # 手机视口
```

### 8. 事件回调

```python
import json

events = []
b.register_event_callback(lambda tab, ev: events.append((tab, json.loads(ev))))
b.open("https://example.com")
b.navigate("https://example.com/login")
print(events)                        # 导航 / console / DOM 事件
```

### 9. 离屏帧流

```python
import time, json

frames = []
b.register_frame_callback(lambda tab, f: frames.append((tab, json.loads(f))))
b.open("https://example.com")
b.start_frame_stream(1, fps=10)
time.sleep(0.5)
b.stop_frame_stream(1)
print(frames[0][1]["width"], frames[0][1]["height"])   # 帧尺寸
```

### 10. 会话、cookie 与 storage

```python
b = fb.FastBrowser(); b.init()
b.open("https://example.com")

b.tool_call("cookie_set", {"name": "sid", "value": "abc", "domain": "example.com"})
b.tool_call("storage_set", {"key": "token", "value": "t1"})

b.session_save("/tmp/state.json")     # {"tabs": 1}

b.shutdown()
b2 = fb.FastBrowser(); b2.init()
b2.session_load("/tmp/state.json")    # {"tabs": 1}
b2.open("https://example.com")
print(b2.tool_call("cookie_get", {"domain": "example.com"}))  # {"cookies": [{"value": "abc", ...}]}
print(b2.tool_call("storage_get", {"key": "token"}))          # {"value": "t1"}
```

### 11. 审计日志

```python
b.tool_call("get_current_url")
print(b.audit())                      # [{"tool":..., "ok":..., ...}, ...]
b.clear_audit()
```

## 异步（asyncio）

```python
import asyncio
import fastbrowser as fb

async def main():
    b = fb.AsyncFastBrowser()
    await b.init()

    t1 = (await b.open("https://example.com"))["tab"]
    t2 = (await b.open("https://example.com/login"))["tab"]

    # 多标签并发 —— 两个调用真正并行
    r1, r2 = await asyncio.gather(
        b.tool_call("get_page_title", {"tab": t1}),
        b.tool_call("get_page_title", {"tab": t2}),
    )
    print(r1["title"], r2["title"])

asyncio.run(main())
```

阻塞操作（`open`/`tool_call`/`snapshot`/`screenshot`…）在 async 接口里经线程池 offload，
内核调用会释放 GIL，因此 `gather` 的多个标签操作真正并发。

## 真实浏览器（Chromium）

通过 CDP 连接你自己的 Chrome：

```python
b = fb.FastBrowser()
b.init({
    "engine": "chromium",
    "cdp_url": "ws://127.0.0.1:9222/devtools/browser/<id>",  # 用 --remote-debugging-port=9222 启动 Chrome
})
b.open("https://example.com")
print(b.snapshot()["title"])
```

或使用打包的 Chrome for Testing（需单独下载，不含在 wheel 内）：

```python
b.init({"engine": "bundled"})   # 自动启动 vendor/chromium（或 CHROME_PATH）
```

真实引擎下，`click`/`type` 走坐标级真实输入（Playwright 风格 actionability），`execute_js`
在真实页面运行，`screenshot` 捕获真实像素，cookie/storage/下载都落在真实浏览器状态上。

## API

| 类别 | 方法 |
|------|---------|
| 生命周期 | `init(config?)`、`is_initialized`、`shutdown` |
| 导航 | `open(url)`、`navigate(url)` |
| 工具 | `tool_call(name, params?)`、`tool_list()`、`tool_count()` |
| 内容 | `snapshot()`、`screenshot()`、`screenshot_png()`、`save_screenshot(path)`、`get_page_*`（经工具） |
| 渲染 | `set_viewport(w,h)`、`get_view()`、`set_rendering_mode(mode)`、`start_frame_stream(tab, fps)`、`stop_frame_stream(tab)` |
| 回调 | `register_event_callback(cb)`、`register_frame_callback(cb)` |
| 状态 | `status()`、`get_info()`、`audit()`、`clear_audit()` |
| 会话 | `session_save(path)`、`session_load(path)`、`clear_state()` |

### 注意事项

- **单实例语义**：每个进程一个 `FastBrowser`（内核是共享 runtime 之上的进程级单例）。用一个实例驱动多标签并发即可。
- **回调**（`register_event_callback`/`register_frame_callback`）在专用派发线程、引擎锁之外执行 —— 可在回调内安全重入浏览器。
- **许可证**：Apache-2.0（与内核一致）。

## 测试

```bash
python -m pytest tests/ -q          # mock 引擎（无需浏览器）
TMPDIR=/tmp python -m pytest tests/test_chromium.py -q   # 真实 Chromium（需 vendor/chromium）
```
