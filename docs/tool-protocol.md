# 工具协议（LLM 说明书）

## 约定

- 每个工具 `ToolSpec`：`name` / `description` / `params`(简化定义) / `schema`(标准 JSON Schema) / `example`；
- `tool_list` 返回每个工具含 `schema` 的标准 JSON Schema（draft-07 风格），
  可直接用于 OpenAI/Anthropic/Gemini 等 function-calling 接口；
- 入参、出参全部 JSON 可序列化；
- 出参统一形态：
  - 成功：工具自身的 JSON 结构；
  - 失败：`{"error":{"kind":"...","message":"..."}}`。

### 简化参数 → 标准 JSON Schema

`ToolSpec.params` 用简化定义（如 `{"id":{"type":"string","required":false}}`），
`schema()` 自动转换为标准 JSON Schema：

```json
{
  "type": "object",
  "properties": {
    "id": { "type": "string" },
    "url": { "type": "string", "description": "Absolute URL to visit" }
  },
  "required": ["url"]
}
```

## 工具清单（88 个，13 功能域）

| 功能域 | 工具 |
|--------|------|
| 导航 | navigate back forward reload stop get_history |
| 交互 | click dblclick right_click type press hover drag scroll swipe focus blur clear_input send_keys |
| 内容提取 | extract_text extract_html extract_links extract_images extract_table extract_json search find_elements |
| 等待/断言 | wait_for_element wait_for_navigation wait_for_load_state wait_for_condition wait_for_text assert_element_exists assert_text_contains assert_url_contains assert_title |
| 表单 | fill_form select_option upload_file checkbox radio extract_forms |
| 页面分析 | screenshot screenshot_element get_page_title get_current_url get_page_text get_element_info get_element_text get_attributes is_visible is_enabled get_focused_element get_selected_text get_page_meta get_scroll_position set_scroll_position get_performance_metrics get_accessibility_tree |
| 会话 | cookie_get cookie_set cookie_clear clear_cookies storage_get storage_set storage_get_all clear_storage |
| 高级 | execute_js evaluate_xpath inject_css block_request intercept_request |
| 网络拦截 | list_pending_requests fulfill_request continue_request abort_request |
| 对话框 | pending_dialog dialog_accept dialog_dismiss |
| 多标签页 | new_tab close_tab switch_tab list_tabs get_active_tab get_tab duplicate_tab close_other_tabs |
| Agent 辅助 | done |
| PDF | save_as_pdf |
| SDK | session_save session_load clear_state set_viewport set_rendering_mode audit clear_audit |

> 会话/生命周期类（session_save / session_load / clear_state）为 SDK 级方法
> （`Fastbrowser::*`），对齐 browser-use 的 `storage_state` 与任务后清理。
> `audit` 返回动作审计日志（新→旧），供回放/信任审计。

## 调用方式

- **Rust**：`sdk.tool_call(name, params)`
- **CLI**：`fastbrowser click a` / `fastbrowser extract_links` / `fastbrowser audit` / …
- **C ABI**：`fastbrowser_tool_call(name, params_json)` / `fastbrowser_audit()`
- **Python/Node/Go/Swift**：见 `bindings/` 各绑定

## 典型 Agent 循环

```
1. open <url>                     # 或 navigate
2. snapshot                       # 感知 → 拿到 a/b/c 编号 + meta（滚动/截断提示）
3. click/type/fill_form ...       # 按编号行动（真实浏览器为坐标级真实点击）
4. extract_text / get_current_url # 验证结果
5. screenshot                     # 视觉确认（真实 PNG）
6. done <answer>                  # 任务收尾
```

> The loop itself is not part of the kernel (the host provides it); the kernel only
> guarantees that tools can be registered and called by any loop.

