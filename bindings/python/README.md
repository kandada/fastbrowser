# fastbrowser

Python bindings for [fastbrowser](../fastbrowser) — a cross-platform browser-automation
kernel for AI agents, written in Rust. This package exposes the kernel through PyO3.

- **Engine-agnostic**: mock (no deps) / `chromium` (CDP) / `bundled` (Chrome for Testing) /
  `webview` / `cef`, unified behind one `BrowserEngine` trait.
- **AI-native**: 95 LLM-friendly tools (`navigate`, `click`, `type`, `extract_text`,
  `wait_for_element`, `fill_form`, `screenshot`, …), all JSON in/out.
- **Multi-tab concurrency**: per-tab locks — different tabs run in parallel, same tab serialized
  (mirrors a real browser's per-process / single-main-thread model).

## Install

```bash
pip install fastbrowser
```

> Prebuilt wheels are published for CPython 3.8+ on macOS (arm64/x86_64),
> Linux (manylinux x86_64/aarch64) and Windows x64.

## Quickstart (mock engine — no browser required)

```python
import fastbrowser as fb

b = fb.FastBrowser()
b.init()                                    # default: mock engine, zero deps

out = b.open("https://example.com")
print(out["title"])                         # "Example Page"
print(b.snapshot()["title"])                # interactive-element snapshot for LLMs
print(b.tool_call("get_current_url"))       # {"url": "https://example.com"}

w, h, rgba = b.screenshot()                 # raw RGBA (w, h, w*h*4 bytes)
b.save_screenshot("page.png")               # PNG

b.shutdown()
```

## Examples

### 1. Snapshot-driven interaction (the core agent loop)

`snapshot()` returns the interactive elements (a/b/c ids) the LLM acts on; `click`/`type`
target those ids:

```python
b = fb.FastBrowser(); b.init()
b.open("https://example.com")

snap = b.snapshot()
for el in snap["interactive"]:
    print(el["id"], el["tag"], el.get("text"), el.get("href"))

# click a link, type into a field, press Enter
b.tool_call("click", {"id": "c"})
b.tool_call("type", {"id": "e", "text": "hello"})
b.tool_call("press", {"key": "Enter"})
```

### 2. Content extraction

```python
b = fb.FastBrowser(); b.init()
b.open("https://example.com")

print(b.tool_call("extract_links"))       # {"links": [{"text": "Learn more", "url": "..."}]}
print(b.tool_call("extract_text"))        # {"text": "..."}
print(b.tool_call("get_page_text"))       # {"text": "..."}
print(b.tool_call("extract_table"))       # {"table": [["Name", "Value"], ...]}
print(b.tool_call("get_page_title"))      # {"title": "Example Page"}
```

### 3. Form filling

```python
b = fb.FastBrowser(); b.init()
b.open("https://example.com")

# fill by snapshot id
b.tool_call("fill_form", {"values": {"e": "alice"}})   # {"filled": ["e"], "ok": true}

# or field-by-field
b.tool_call("type", {"id": "e", "text": "bob", "clear": True})
b.tool_call("checkbox", {"id": "d", "checked": True})

# on a real engine: dropdown / radio / file upload
b.tool_call("select_option", {"id": "e", "value": "pro"})
b.tool_call("radio", {"id": "r"})
```

### 4. Multi-tab

Every tool accepts `{"tab": N}` — cross-tab operations don't depend on the active tab:

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

### 5. Waiting

```python
b.tool_call("wait_for_element", {"selector": "button", "timeout_ms": 5000})
b.tool_call("wait_for_navigation", {"timeout_ms": 10000})
```

### 6. Execute JavaScript

```python
print(b.tool_call("execute_js", {"script": "document.title"}))            # {"result": "Example Page"}
# on a real engine, any script runs against the real page:
print(b.tool_call("execute_js", {"script": "document.links.length"}))     # {"result": <count>}
```

### 7. Screenshots

```python
w, h, rgba = b.screenshot()          # raw RGBA
png = b.screenshot_png()             # PNG bytes (stdlib-encoded, no Pillow)
b.save_screenshot("page.png")        # save PNG to disk
b.set_viewport(390, 844)             # mobile viewport
```

### 8. Event callbacks

```python
import json

events = []
b.register_event_callback(lambda tab, ev: events.append((tab, json.loads(ev))))
b.open("https://example.com")
b.navigate("https://example.com/login")
print(events)                        # navigation / console / dom events
```

### 9. OSR frame streaming

```python
import time, json

frames = []
b.register_frame_callback(lambda tab, f: frames.append((tab, json.loads(f))))
b.open("https://example.com")
b.start_frame_stream(1, fps=10)
time.sleep(0.5)
b.stop_frame_stream(1)
print(frames[0][1]["width"], frames[0][1]["height"])   # frame dims
```

### 10. Session state, cookies & storage

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

### 11. Audit trail

```python
b.tool_call("get_current_url")
print(b.audit())                      # [{"tool":..., "ok":..., ...}, ...]
b.clear_audit()
```

## Async (asyncio)

```python
import asyncio
import fastbrowser as fb

async def main():
    b = fb.AsyncFastBrowser()
    await b.init()

    t1 = (await b.open("https://example.com"))["tab"]
    t2 = (await b.open("https://example.com/login"))["tab"]

    # multi-tab concurrency — the two calls run truly in parallel
    r1, r2 = await asyncio.gather(
        b.tool_call("get_page_title", {"tab": t1}),
        b.tool_call("get_page_title", {"tab": t2}),
    )
    print(r1["title"], r2["title"])

asyncio.run(main())
```

Blocking operations (`open`/`tool_call`/`snapshot`/`screenshot`…) are offloaded to a thread
pool in the async interface, and the kernel releases the GIL, so `gather` runs concurrent tabs
truly in parallel.

## Real browser (Chromium)

Point at your own Chrome via CDP:

```python
b = fb.FastBrowser()
b.init({
    "engine": "chromium",
    "cdp_url": "ws://127.0.0.1:9222/devtools/browser/<id>",  # start Chrome with --remote-debugging-port=9222
})
b.open("https://example.com")
print(b.snapshot()["title"])
```

Or use the bundled Chrome for Testing (downloads separately, not shipped in the wheel):

```python
b.init({"engine": "bundled"})   # auto-launches vendor/chromium (or CHROME_PATH)
```

With a real engine, `click`/`type` use real coordinate-level input with Playwright-style
actionability, `execute_js` runs in the real page, `screenshot` captures real pixels, and
cookies/storage/downloads hit real browser state.

## API

| Area | Methods |
|------|---------|
| Lifecycle | `init(config?)`, `is_initialized`, `shutdown` |
| Navigation | `open(url)`, `navigate(url)` |
| Tools | `tool_call(name, params?)`, `tool_list()`, `tool_count()` |
| Content | `snapshot()`, `screenshot()`, `screenshot_png()`, `save_screenshot(path)`, `get_page_*` via tools |
| Rendering | `set_viewport(w,h)`, `get_view()`, `set_rendering_mode(mode)`, `start_frame_stream(tab, fps)`, `stop_frame_stream(tab)` |
| Callbacks | `register_event_callback(cb)`, `register_frame_callback(cb)` |
| State | `status()`, `get_info()`, `audit()`, `clear_audit()` |
| Session | `session_save(path)`, `session_load(path)`, `clear_state()` |

### Notes

- **Single-instance semantics**: one `FastBrowser` per process (the kernel is a process-wide
  singleton behind a shared runtime). Use one instance and drive tabs concurrently.
- **Callbacks** (`register_event_callback`/`register_frame_callback`) fire on a dedicated
  dispatcher thread outside the engine lock — safe to re-enter the browser from inside them.
- **License**: Apache-2.0 (matches the kernel).

## Test

```bash
python -m pytest tests/ -q          # mock engine (no browser)
TMPDIR=/tmp python -m pytest tests/test_chromium.py -q   # real Chromium (needs vendor/chromium)
```
