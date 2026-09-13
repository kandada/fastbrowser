# 交互元素快照规范（第 1 号工具）

`PageSnapshot` 是内核给 LLM 的"感知画面"：只包含**可交互元素**，
每个元素分配 `a/b/c…` 编号。LLM 依据编号操作（`click {"id":"a"}`），
元素失效时用 `refs`（css/xpath/text）兜底。

## JSON 形态（V2）

```json
{
  "title": "Login",
  "url": "https://example.com/login",
  "viewport": { "width": 1280, "height": 800, "device_scale_factor": 2.0 },
  "interactive": [
    {
      "id": "a",
      "tag": "input",
      "role": null,
      "text": null,
      "href": null,
      "rect": { "x": 10, "y": 50, "width": 200, "height": 30 },
      "refs": [
        { "kind": "snapshot", "value": "a" },
        { "kind": "css", "value": "#username" },
        { "kind": "xpath", "value": "//input" },
        { "kind": "text", "value": "Username" }
      ],
      "attrs": { "name": "username", "placeholder": "Username", "disabled": "true" },
      "value": "",
      "input_type": "text",
      "checked": null,
      "options": null,
      "selected": null,
      "visible": true
    }
  ],
  "frames": [],
  "timestamp_ms": 1720000000000,
  "meta": { "total": 12, "truncated": false, "viewport_h": 800, "scroll_h": 2400, "scroll_y": 0 }
}
```

## 编号规则

- 只给**可见且可交互**元素编号（a、b、c…，页面内唯一）；
- 上限 **26 个（a..z）**；超过时 `meta.truncated=true`，LLM 应滚动后用 `scroll`
  工具继续（`meta.scroll_h`/`scroll_y` 提供滚动上下文）；
- 每次快照重新分配，Agent 操作前应重新快照；
- LLM 应优先用 `id`；命中失败时回退 `refs`。

## 交互判定（注入 JS，`runtime_extract_js`）

候选元素：`a,button,input,select,textarea,summary,label` + 常见 ARIA role
（`button/link/checkbox/radio/menuitem/combobox/slider/switch/tab/textbox/…`）+
`[tabindex]` + `[contenteditable]` + 显式 `onclick`/`onmousedown`。

可见性：`display/visibility/pointer-events` + 非零尺寸。

## iframe 与 shadow DOM

- **同源 iframe**：注入 JS 递归进入 `contentDocument`，元素并入 `frames[]`
  （`url`/`name`/`interactive`/`meta`），坐标为**顶层视口坐标**（iframe 偏移累加）；
- **跨域 iframe**：无法读取 DOM，记录为 `frames[]` 中 `cross_origin:true` 的条目
  （仅 url/name，无元素）；
- **open shadow DOM**：递归进入 `shadowRoot`，元素并入所在文档的 `interactive`，
  与文档同坐标系；
- **动作定位**：`fbFind` 跨文档/跨 shadow 定位 `[data-fb]`，坐标点击沿 iframe 链
  换算顶层坐标，遮挡判定沿 shadow host 链向上验证（`elementFromPoint` 不穿透
  shadow root 时正确识别）。

## meta 字段

| 字段 | 含义 |
|------|------|
| `total` | 实际扫描到的可交互元素总数（含截断部分） |
| `truncated` | 是否因超过 26 个而截断（提示 LLM 需要滚动） |
| `viewport_h` | 视口高度（CSS px） |
| `scroll_h` | 文档总高度 |
| `scroll_y` | 当前垂直滚动位置 |

## 实现要点（引擎层）

- `mock`：内存元素表直接映射；
- `webview`/`cdp`：注入 JS（`engine::inject::runtime_extract_js`）标注 `data-fb`
  属性并返回 `{elements, meta, frames}`，共享 `parse_snapshot_value` 解析保证两引擎语义一致；
  动作脚本按 `[data-fb="X"]` 定位。
- **真实点击**（CDP）：滚动进视口 → 元素中心（跨 iframe/shadow 换算顶层坐标）→
  `elementFromPoint` hit-test → 被遮挡则报错点名遮挡者 → `Input.dispatchMouseEvent`。

