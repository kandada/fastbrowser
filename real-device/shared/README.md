# shared/ — 跨平台共享测试资产

**思想**：真机测试用例只写一份（JSON），各平台 runner 各自解释执行：

```text
shared/cases/*.json   ← 唯一用例源（smoke / agent_flow / multi_tab / events / persistence / perf）
        │
        ├─ shared/runner（Rust，桌面参考）   ── mock / chromium 先跑通，证明用例定义正确
        ├─ android/app（Kotlin + WebViewOps）── 真机：经 C ABI 逐条执行同一份 JSON
        └─ ios/FastbrowserDeviceTests（Swift）── 真机：经 C FFI 逐条执行同一份 JSON
```

## 用例 JSON 格式

```json
{
  "id": "smoke.basic",
  "category": "smoke",
  "description": "...",
  "engines": ["mock", "chromium", "webview"],   // 该用例支持的引擎（不在列表则 SKIP）
  "steps": [
    {"action": "open", "params": {"url": "{{BASE}}/login"}},
    {"action": "wait_load", "state": "load", "timeout_ms": 15000},
    {"action": "find", "var": "user", "match": {"tag": "input", "name": "username"}},
    {"action": "tool", "name": "type", "params": {"id": {"$var": "user"}, "text": "alice", "tab": {"$var": "current_tab"}}},
    {"action": "tool", "name": "get_page_title", "params": {}, "expect": {"title": {"contains": "Login"}}}
  ]
}
```

### 动作（action）
| action | 字段 | 说明 |
|---|---|---|
| `open` | `params.url` | 打开 URL（自动建标签页） |
| `navigate` | `params.url` | 活动标签导航 |
| `tool` | `name`、`params`、可选 `expect`/`expect_error` | 调用工具；`expect_error` 断言应报错 |
| `find` | `var`、`match{tag,name,text,placeholder}` | 快照找元素，把 a/b/c 存入 `var` |
| `wait_load` | `state`、`timeout_ms` | 等 document.readyState |
| `wait_navigation` | `timeout_ms` | 等导航完成事件 |
| `sleep` | `ms` | 固定等待（如对话框触发） |
| `session_save` / `session_load` | `path` | SDK 会话持久化（非工具） |
| `reset` | — | 关闭并重建内核（会话往返用） |
| `benchmark` | `tool`、`iterations`、`expect_avg_ms` | 耗时报告 + 宽松上限 |

### 模板与内置变量
- `{{BASE}}`：引擎基准 URL（mock=`https://example.com`，chromium/webview=本地 fixture 服务器）。
- `{"$var":"name"}`：参数引用变量。内置：`current_tab`（最近标签）、`tab1/tab2…`（打开顺序）、
  `base_host`（cookie 域名）、`session_path`（runner 分配的临时会话文件）。

### expect 匹配
| 写法 | 含义 |
|---|---|
| `{"key": "abc"}` | 结果 `result.key` 深等于 `"abc"` |
| `{"key": {"contains":"x"}}` | 字符串包含 |
| `{"key": {"is_number":true}}` / `{"is_array"}` / `{"is_object"}` / `{"is_null"}` | 类型断言 |
| `{"key": {"gte":2}}` | 数值 ≥2 |
| `{"key": {"len_gte":2}}` | 数组长度 ≥2 |

## 参考 runner 用法

```bash
cd real-device/shared/runner
cargo run -- --cases ../cases --engine mock          # 无浏览器（CI 可用）
cargo run --features engine-cdp -- --cases ../cases --engine chromium   # 真实 Chrome
cargo run --features engine-cdp -- --cases ../cases --engine chromium --category agent_flow
cargo run --features engine-cdp -- --cases ../cases --engine chromium --cdp-url ws://...  # 连已开的 Chrome
```
报告输出到 `$REAL_DEVICE_REPORTS/runner-<engine>.json`。
