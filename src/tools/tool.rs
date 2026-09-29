// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 工具抽象：`ToolSpec`（给 LLM 的说明书）+ `ToolContext`（执行环境）+ `Tool`。

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::bridge::Runtime;
use crate::engine::{
    Capability, ElementRef, EngineCapabilities, EngineError, ErrorKind, RefKind, Result, TabId,
};

/// 工具规格。description / params / example 是"给 LLM 的说明书"。
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// 简化的 JSON Schema（参数定义）。见 `schema()` 转标准 JSON Schema。
    pub params: Value,
    pub example: &'static str,
}

impl ToolSpec {
    /// 把简化参数定义转成标准 JSON Schema（draft-07 风格，兼容各 LLM function-calling）。
    ///
    /// 输入形如：
    /// ```json
    /// { "url": {"type":"string","required":true,"description":"..."},
    ///   "timeout_ms": {"type":"number","default":5000} }
    /// ```
    /// 输出：
    /// ```json
    /// { "type":"object", "properties":{ "url":{"type":"string","description":"..."},
    ///   "timeout_ms":{"type":"number","default":5000} },
    ///   "required":["url"] }
    /// ```
    pub fn schema(&self) -> Value {
        fn convert(def: &Value) -> Value {
            let out = def.clone();
            if let Some(obj) = out.as_object() {
                let mut m = obj.clone();
                m.remove("required");
                // items 递归转换（数组元素）
                if let Some(items) = m.get("items") {
                    m.insert("items".into(), convert(items));
                }
                return Value::Object(m);
            }
            out
        }
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), json!("object"));
        if let Some(obj) = self.params.as_object() {
            let mut required = Vec::new();
            let mut properties = serde_json::Map::new();
            for (k, def) in obj {
                if def
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    required.push(json!(k));
                }
                properties.insert(k.clone(), convert(def));
            }
            if !required.is_empty() {
                schema.insert("required".into(), Value::Array(required));
            }
            if !properties.is_empty() {
                schema.insert("properties".into(), Value::Object(properties));
            }
        }
        Value::Object(schema)
    }

    /// 人类/模型可读的参数清单 + 示例（用于参数错误时给出指引）。
    pub fn param_hint(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(obj) = self.params.as_object() {
            for (k, def) in obj {
                let ty = def.get("type").and_then(Value::as_str).unwrap_or("any");
                let req = def
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if req {
                    parts.push(format!("{k}:{ty} (required)"));
                } else {
                    parts.push(format!("{k}:{ty}"));
                }
            }
        }
        let params = if parts.is_empty() {
            "(no params)".to_string()
        } else {
            parts.join(", ")
        };
        let ex = self.example.trim();
        if ex.is_empty() {
            format!("valid params: {params}")
        } else {
            format!("valid params: {params}; example: {ex}")
        }
    }
}

/// Common cross-tool confusions: when a required param is missing, point the
/// model at the tool that actually takes the params it supplied (the LLM often
/// mixes up `wait_for_text` / `wait_for_element`, `find_elements`, etc.).
fn cross_tool_hint(tool: &str, missing: &str) -> Option<&'static str> {
    match (tool, missing) {
        ("wait_for_text", "text") => {
            Some("to wait for an element by selector use `wait_for_element` (selector=...)")
        }
        ("wait_for_element", "selector") => {
            Some("to wait for visible text use `wait_for_text` (text=...)")
        }
        ("find_elements", "selector") => {
            Some("`selector` is required, e.g. {\"selector\": \"a.product-link\"}")
        }
        ("execute_js", "script") => {
            Some("pass the JS source as `script` (aliases: expression / expr / code / js)")
        }
        ("assert_title", "contains") => {
            Some("pass the expected title substring as `contains` (aliases: query / q / title)")
        }
        _ => None,
    }
}

/// 参数类错误（缺必填/类型错）→ 附上该工具的合法参数清单与示例，便于模型自纠。
pub fn enrich_tool_error(err: EngineError, spec: &ToolSpec) -> EngineError {
    if err.kind != ErrorKind::InvalidArgument {
        return err;
    }
    let m = &err.message;
    if m.contains("missing required param") {
        // "missing required param 'text'" → cross-tool guidance.
        let key = m.split('\'').nth(1).unwrap_or("");
        let mut msg = format!("{}; {}", m, spec.param_hint());
        if let Some(h) = cross_tool_hint(spec.name, key) {
            msg.push_str("; hint: ");
            msg.push_str(h);
        }
        return EngineError::new(err.kind, msg);
    }
    if m.starts_with("param '") {
        return EngineError::new(err.kind, format!("{}; {}", m, spec.param_hint()));
    }
    err
}

/// Engine `evaluate` / `execute_xpath` report a JS exception as the *value*
/// `{"error": "<msg>"}` (CDP `exceptionDetails`; the mobile WebView host does
/// the same). Surface it as a real error so `execute_js` & friends are not
/// reported as `success:true`. Only a single-key object whose `error` is a
/// string is treated this way (a script legitimately returning more keys, or a
/// non-string `error`, is passed through unchanged).
pub fn surface_js_error(value: Value) -> Result<Value> {
    if let Value::Object(o) = &value {
        if o.len() == 1 {
            if let Some(Value::String(msg)) = o.get("error") {
                return Err(EngineError::new(
                    ErrorKind::Evaluate,
                    format!("js error: {msg}"),
                ));
            }
        }
    }
    Ok(value)
}

/// 工具执行上下文。持有 `&Runtime`（`&self` 方法 + 内部 per-tab 锁），
/// 因此不同标签页的工具调用可并行；同一标签页内串行。
pub struct ToolContext<'a> {
    pub runtime: &'a Runtime,
    pub params: Value,
}

impl<'a> ToolContext<'a> {
    /// 当前活动标签页；不存在则自动创建一个空白标签页。
    pub fn active_tab(&self) -> Result<TabId> {
        if let Some(t) = self.runtime.engine().active_tab() {
            return Ok(t);
        }
        self.runtime.ensure_tab()
    }

    /// 目标标签页：显式 `tab` 参数优先，否则活动标签页（不存在则自动创建）。
    /// 让所有工具支持多标签操作（`{"tab": N}`）。
    pub fn target_tab(&self) -> Result<TabId> {
        if let Some(t) = self.tab_param()? {
            return Ok(t);
        }
        self.active_tab()
    }

    /// 活动标签页或显式指定的标签页。
    pub fn tab(&self, explicit: Option<TabId>) -> Result<TabId> {
        match explicit {
            Some(t) => Ok(t),
            None => self.active_tab(),
        }
    }

    /// 读取必填参数（反序列化）。
    pub fn param<T: DeserializeOwned>(&self, key: &str) -> Result<T> {
        let v = self
            .params
            .get(key)
            .cloned()
            .ok_or_else(|| EngineError::invalid(format!("missing required param '{key}'")))?;
        serde_json::from_value(v).map_err(|e| EngineError::invalid(format!("param '{key}': {e}")))
    }

    /// 读取可选参数。
    pub fn param_opt<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value(v.clone())
                .map(Some)
                .map_err(|e| EngineError::invalid(format!("param '{key}': {e}"))),
        }
    }

    /// 读取必填字符串参数。
    pub fn param_str(&self, key: &str) -> Result<String> {
        self.param(key)
    }

    /// 解析元素目标，兼容多种方言（见 FASTBROWSER_CONTRACT_AND_COMPAT.md §5.4）：
    ///
    /// - `ref`（对象 `{kind,value}` 或字符串）
    /// - `selector`（CSS / `text=` / `role=` / `xpath=` / `//...`）
    /// - `id`（快照字母 / `#css` / 元素 id）
    /// - `element`（Playwright MCP 的人类可读描述，退化为文本匹配）
    ///
    /// 这是所有交互工具的统一入口。
    pub fn resolve_target(&self) -> Result<ElementRef> {
        resolve_target_from(&self.params)
    }

    /// 向后兼容别名。
    pub fn element_ref(&self) -> Result<ElementRef> {
        self.resolve_target()
    }

    /// 返回参数中的 `tab`（可选）。
    pub fn tab_param(&self) -> Result<Option<TabId>> {
        self.param_opt::<u32>("tab").map(|t| t.map(TabId))
    }

    /// 由 id/ref 解析元素并返回其快照信息（用于 hover/拖拽坐标）。
    pub fn element_snapshot(&self, tab: TabId) -> Result<crate::engine::InteractiveElement> {
        let r = self.element_ref()?;
        self.element_snapshot_for(tab, r)
    }

    /// Like [`Self::element_snapshot`] but for an explicit target (used by tools
    /// with two targets, e.g. `drag`).
    pub fn element_snapshot_for(
        &self,
        tab: TabId,
        r: crate::engine::ElementRef,
    ) -> Result<crate::engine::InteractiveElement> {
        let snap = self.runtime.engine().snapshot(tab)?;
        let found = match r.kind {
            RefKind::Snapshot => {
                let c = r
                    .value
                    .chars()
                    .next()
                    .ok_or_else(|| EngineError::invalid("bad snapshot id"))?;
                snap.element_by_id(c).cloned()
            }
            _ => snap
                .interactive
                .iter()
                .find(|el| el.matches_ref(&r))
                .cloned(),
        };
        if let Some(el) = found {
            return Ok(el);
        }
        // Live fallback for non-snapshot targets (CSS / XPath / text / role):
        // tag the element, then read back a synthetic element.
        let _ = self.eval(tab, &crate::engine::inject::live_resolve_js(&r))?;
        let info = self.eval(
            tab,
            &crate::engine::inject::element_info_js(crate::engine::inject::LIVE_ID),
        )?;
        if info.is_null() {
            return Err(EngineError::new(
                ErrorKind::Dom,
                format!("element {r} not found"),
            ));
        }
        let rect = &info["rect"];
        let mut attrs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Some(obj) = info["attrs"].as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    attrs.insert(k.clone(), s.to_string());
                }
            }
        }
        if info["disabled"].as_bool() == Some(true) {
            attrs.insert("disabled".to_string(), "true".to_string());
        }
        // The live resolver tags the element with `data-fb=<sentinel>`; it is an
        // internal marker, not a real attribute.
        attrs.remove("data-fb");
        Ok(crate::engine::InteractiveElement {
            id: crate::engine::inject::LIVE_ID,
            tag: info["tag"].as_str().unwrap_or("").to_string(),
            role: info["role"].as_str().map(String::from),
            text: info["text"].as_str().map(String::from),
            href: info["href"].as_str().map(String::from),
            rect: crate::engine::Rect::new(
                rect["x"].as_f64().unwrap_or(0.0),
                rect["y"].as_f64().unwrap_or(0.0),
                rect["width"].as_f64().unwrap_or(0.0),
                rect["height"].as_f64().unwrap_or(0.0),
            ),
            refs: vec![r.clone()],
            attrs,
            value: info["value"].as_str().map(String::from),
            input_type: info["input_type"].as_str().map(String::from),
            checked: info["checked"].as_bool(),
            selectable_options: None,
            selected_option: None,
            visible: info["visible"].as_bool().unwrap_or(true),
        })
    }

    /// 在标签页中执行 JS。
    pub fn eval(&self, tab: TabId, script: &str) -> Result<Value> {
        self.runtime.engine().evaluate(tab, script)
    }

    /// 执行 JS，失败时返回 None（用于可降级的页面内省工具）。
    pub fn eval_opt(&self, tab: TabId, script: &str) -> Option<Value> {
        self.runtime.engine().evaluate(tab, script).ok()
    }

    /// 读取页面文本。
    pub fn page_text(&self, tab: TabId) -> Result<String> {
        self.runtime.engine().get_page_text(tab)
    }
}

/// 从参数字典解析元素目标（纯函数，便于测试）。优先级：
/// `ref`（对象/字符串）→ `selector` → `id` → `element`。
pub fn resolve_target_from(params: &Value) -> Result<ElementRef> {
    // 1) 显式 ref：对象沿用旧语义；字符串按方言解析。
    if let Some(r) = params.get("ref") {
        match r {
            Value::String(s) if !s.is_empty() => return Ok(parse_ref_string(s)),
            Value::Object(_) => return deserialize_ref(r),
            _ => {}
        }
    }
    // 2) selector：CSS / Playwright 方言前缀。
    if let Some(s) = params.get("selector").and_then(Value::as_str) {
        if !s.is_empty() {
            return Ok(parse_selector(s));
        }
    }
    // 3) id：`#css`/`.class` 等直接当 CSS；快照字母/`eN` 当快照；数字当快照下标；其余当元素 id。
    if let Some(idv) = params.get("id") {
        if let Some(id) = idv.as_str() {
            if !id.is_empty() {
                if let Some(c) = crate::engine::snapshot::parse_snapshot_ref(id) {
                    return Ok(ElementRef::snapshot(c));
                }
                let looks_css = id.starts_with('#')
                    || id.starts_with('.')
                    || id.starts_with('[')
                    || id.contains(' ')
                    || id.contains('>');
                if looks_css {
                    return Ok(ElementRef::css(id));
                }
                // 多字符、非选择器 → 视为元素 id
                return Ok(ElementRef::css(format!("#{id}")));
            }
        } else if let Some(n) = idv.as_u64() {
            // browser-use `index`（经别名映射为 id）等数字下标 → 快照字母。
            let idx = (n as u8).min(25);
            return Ok(ElementRef::snapshot((b'a' + idx) as char));
        }
    }
    // 4) element：MCP 的可读描述 → 文本匹配；但若明显是选择器/方言前缀，
    //    按选择器解析（此前 `element:"#res"` 会被当成文本 `text:#res` 而找不到）。
    if let Some(e) = params.get("element").and_then(Value::as_str) {
        if !e.is_empty() {
            let looks_selector = e.starts_with('#')
                || e.starts_with('.')
                || e.starts_with('[')
                || e.starts_with("//")
                || e.starts_with("css=")
                || e.starts_with("xpath=")
                || e.starts_with("text=")
                || e.starts_with("role=")
                || e.starts_with("label=")
                || e.starts_with("placeholder=")
                || e.starts_with("alt=")
                || e.starts_with("title=")
                || e.starts_with("value=")
                || e.starts_with("href=")
                || e.starts_with("testid=")
                || e.starts_with("data-testid=")
                || e.starts_with('@')
                || e.contains('>');
            if looks_selector {
                return Ok(parse_selector(e));
            }
            return Ok(ElementRef::text(e));
        }
    }
    // 5) 独立方言参数：`role`（可配合 `name`）与 `name`（字段名/可访问名）。
    if let Some(r) = params.get("role").and_then(Value::as_str) {
        if !r.is_empty() {
            if let Some(n) = params.get("name").and_then(Value::as_str) {
                if !n.is_empty() {
                    return Ok(ElementRef::role(format!("{r}[name={n:?}]")));
                }
            }
            return Ok(ElementRef::role(r));
        }
    }
    if let Some(n) = params.get("name").and_then(Value::as_str) {
        if !n.is_empty() {
            return Ok(ElementRef::selector(format!("#{n}, [name=\"{n}\"]")));
        }
    }
    Err(EngineError::invalid(
        "need an element target: 'selector' (CSS), 'ref' (string or {kind,value}), or 'id' (snapshot letter)",
    ))
}

/// Parse a locator string (snapshot letter / `eN` / CSS / Playwright dialect)
/// into an element target — for tools that take element strings (e.g. `drag`).
pub fn target_from_str(s: &str) -> ElementRef {
    parse_ref_string(s)
}

/// 解析 `ref` 字符串：单字母 / `eN` → 快照；否则按选择器方言解析。
fn parse_ref_string(s: &str) -> ElementRef {
    if let Some(c) = crate::engine::snapshot::parse_snapshot_ref(s) {
        return ElementRef::snapshot(c);
    }
    parse_selector(s)
}

/// 解析选择器字符串，兼容 Playwright 方言前缀：
/// `css=` / `text=` / `role=` / `xpath=` / `//...` / `:has-text("...")`；否则当 CSS。
///
/// 实现下沉到 `engine::snapshot::parse_selector_dialect`，与原生 surface 共用同一套方言规则。
fn parse_selector(s: &str) -> ElementRef {
    crate::engine::snapshot::parse_selector_dialect(s)
}

/// 解析 `{"kind": "css"|"xpath"|"text", "value": "..."}`。
fn deserialize_ref(v: &Value) -> Result<ElementRef> {
    let kind = v
        .get("kind")
        .and_then(|k| k.as_str())
        .ok_or_else(|| EngineError::invalid("ref.kind missing"))?;
    let value = v
        .get("value")
        .and_then(|s| s.as_str())
        .ok_or_else(|| EngineError::invalid("ref.value missing"))?
        .to_string();
    match kind {
        "css" => Ok(ElementRef::css(value)),
        "xpath" => Ok(ElementRef::xpath(value)),
        "text" => Ok(ElementRef::text(value)),
        "role" => Ok(ElementRef::role(value)),
        "selector" | "playwright" => Ok(ElementRef::selector(value)),
        "snapshot" => crate::engine::snapshot::parse_snapshot_ref(&value)
            .map(ElementRef::snapshot)
            .ok_or_else(|| EngineError::invalid("snapshot ref empty/invalid")),
        other => Err(EngineError::invalid(format!(
            "unknown ref.kind '{other}' (css|xpath|text|role|snapshot|selector)"
        ))),
    }
}

/// 工具执行函数。
pub type ToolFn = fn(&ToolContext) -> Result<Value>;

/// 一个工具。
pub struct Tool {
    pub spec: ToolSpec,
    pub run: ToolFn,
    /// 该工具对引擎的能力要求（空 = 任何引擎可用）。
    pub requires: &'static [Capability],
}

impl Tool {
    pub fn new(
        name: &'static str,
        description: &'static str,
        params: Value,
        example: &'static str,
        run: ToolFn,
    ) -> Self {
        Tool {
            spec: ToolSpec {
                name,
                description,
                params,
                example,
            },
            run,
            requires: &[],
        }
    }

    /// 声明该工具依赖的能力（能力不满足时从清单中隐藏、调用时明确报错）。
    pub fn requires(mut self, caps: &'static [Capability]) -> Self {
        self.requires = caps;
        self
    }

    /// 该工具是否被给定引擎能力集支持。
    pub fn supported_by(&self, caps: &EngineCapabilities) -> bool {
        self.requires.iter().all(|&c| caps.supports(c))
    }

    /// 执行工具（字段 `run` 为底层函数指针，此方法便于 `tool.run(ctx)` 调用）。
    pub fn run(&self, ctx: &ToolContext<'_>) -> Result<Value> {
        (self.run)(ctx)
    }

    pub fn params_json(&self) -> Value {
        json!({
            "name": self.spec.name,
            "description": self.spec.description,
            "params": self.spec.params,
            "schema": self.spec.schema(),
            "example": self.spec.example,
        })
    }
}

/// Build `{ <field>: <text capped to max_chars>, "truncated": bool, "length": n }`.
/// `max_chars == 0` means no cap. Used by the text-returning tools so callers can
/// bound the observation (the raw length is always reported).
pub fn capped_text(field: &str, text: &str, max_chars: usize) -> Value {
    let total = text.chars().count();
    let truncated = max_chars > 0 && total > max_chars;
    let shown: String = if truncated {
        text.chars().take(max_chars).collect()
    } else {
        text.to_string()
    };
    json!({ field: shown, "truncated": truncated, "length": total })
}

/// Build `{ <field>: <array capped to limit>, "count": <total>, "truncated": bool }`.
/// `limit == 0` means no cap.
pub fn capped_array(field: &str, items: Value, limit: usize) -> Value {
    match items {
        Value::Array(a) => {
            let total = a.len();
            let truncated = limit > 0 && total > limit;
            let shown: Vec<Value> = if truncated {
                a.into_iter().take(limit).collect()
            } else {
                a
            };
            json!({ field: shown, "count": total, "truncated": truncated })
        }
        other => json!({ field: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn target_from_str_kinds() {
        use crate::engine::RefKind;
        assert_eq!(target_from_str("a").kind, RefKind::Snapshot);
        // A CSS/selector string resolves to Css or Selector depending on dialect.
        let k = target_from_str("#x").kind;
        assert!(matches!(k, RefKind::Css | RefKind::Selector), "got {k:?}");
        let k = target_from_str("button").kind;
        assert!(matches!(k, RefKind::Css | RefKind::Selector), "got {k:?}");
    }

    #[test]
    fn resolve_target_accepts_role_and_name() {
        let r = resolve_target_from(&json!({"role": "link"})).unwrap();
        assert_eq!(r.kind, crate::engine::RefKind::Role);
        assert_eq!(r.value, "link");
        // role + name → `role=...[name="..."]`
        let r = resolve_target_from(&json!({"role": "button", "name": "Go"})).unwrap();
        assert_eq!(r.kind, crate::engine::RefKind::Role);
        assert_eq!(r.value, "button[name=\"Go\"]");
        // bare name → id/name selector
        let r = resolve_target_from(&json!({"name": "username"})).unwrap();
        assert_eq!(r.kind, crate::engine::RefKind::Selector);
        assert!(r.value.contains("username"));
    }

    #[test]
    fn cross_tool_hint_points_at_the_right_tool() {
        assert!(cross_tool_hint("wait_for_text", "text")
            .unwrap()
            .contains("wait_for_element"));
        assert!(cross_tool_hint("wait_for_element", "selector")
            .unwrap()
            .contains("wait_for_text"));
        assert!(cross_tool_hint("find_elements", "selector")
            .unwrap()
            .contains("selector"));
        assert!(cross_tool_hint("assert_title", "contains")
            .unwrap()
            .contains("contains"));
        assert!(cross_tool_hint("navigate", "url").is_none());
    }

    #[test]
    fn enrich_missing_required_appends_cross_tool_hint() {
        let spec = ToolSpec {
            name: "wait_for_text",
            description: "wait",
            params: json!({"text": {"type": "string", "required": true}}),
            example: r#"{"text":"Login"}"#,
        };
        let e = enrich_tool_error(EngineError::invalid("missing required param 'text'"), &spec);
        assert!(e.message.contains("valid params"), "{}", e.message);
        assert!(
            e.message.contains("hint:") && e.message.contains("wait_for_element"),
            "{}",
            e.message
        );
        // Unrelated missing param → no cross-tool hint.
        let spec2 = ToolSpec {
            name: "navigate",
            description: "nav",
            params: json!({"url": {"type": "string", "required": true}}),
            example: "{}",
        };
        let e2 = enrich_tool_error(EngineError::invalid("missing required param 'url'"), &spec2);
        assert!(!e2.message.contains("hint:"), "{}", e2.message);
    }

    #[test]
    fn surface_js_error_detects_host_exception_shape() {
        // CDP / mobile host report a JS exception as a single-key {"error": "..."}.
        let e = surface_js_error(json!({"error": "TypeError: x is not a function"})).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Evaluate);
        assert!(e.message.contains("TypeError"), "{}", e.message);
        // A legitimate object with more keys passes through unchanged.
        assert_eq!(
            surface_js_error(json!({"error": "x", "other": 1})).unwrap(),
            json!({"error": "x", "other": 1})
        );
        // Non-string `error` passes through.
        assert_eq!(
            surface_js_error(json!({"error": 42})).unwrap(),
            json!({"error": 42})
        );
        // Non-objects pass through.
        assert_eq!(surface_js_error(json!("hello")).unwrap(), json!("hello"));
        assert_eq!(surface_js_error(Value::Null).unwrap(), Value::Null);
    }

    #[test]
    fn resolve_target_supports_dialects() {
        // ref: 单字母 / eN / 方言
        assert_eq!(
            resolve_target_from(&json!({"ref": "a"})).unwrap(),
            ElementRef::snapshot('a')
        );
        assert_eq!(
            resolve_target_from(&json!({"ref": "e5"})).unwrap(),
            ElementRef::snapshot('e')
        );
        // id: eN / 数字（browser-use index 别名）/ 元素 id / CSS
        assert_eq!(
            resolve_target_from(&json!({"id": "e2"})).unwrap(),
            ElementRef::snapshot('b')
        );
        assert_eq!(
            resolve_target_from(&json!({"id": 3})).unwrap(),
            ElementRef::snapshot('d')
        );
        assert_eq!(
            resolve_target_from(&json!({"id": "submit"})).unwrap(),
            ElementRef::css("#submit")
        );
        assert_eq!(
            resolve_target_from(&json!({"id": "#go"})).unwrap(),
            ElementRef::css("#go")
        );
        // selector: 方言
        assert_eq!(
            resolve_target_from(&json!({"selector": "id=go"})).unwrap(),
            ElementRef::css("#go")
        );
        assert_eq!(
            resolve_target_from(&json!({"selector": "role=button"})).unwrap(),
            ElementRef::role("button")
        );
        // ref 对象 {kind,value}，含 eN
        assert_eq!(
            resolve_target_from(&json!({"ref": {"kind": "snapshot", "value": "e3"}})).unwrap(),
            ElementRef::snapshot('c')
        );
        assert_eq!(
            resolve_target_from(&json!({"ref": {"kind": "css", "value": "#x"}})).unwrap(),
            ElementRef::css("#x")
        );
        // 无目标 → 报错
        assert!(resolve_target_from(&json!({})).is_err());
    }

    #[test]
    fn capped_text_and_array() {
        let v = capped_text("text", "hello", 0);
        assert_eq!(v["text"], "hello");
        assert_eq!(v["truncated"], false);
        let v = capped_text("text", "hello", 3);
        assert_eq!(v["text"], "hel");
        assert_eq!(v["truncated"], true);
        assert_eq!(v["length"], 5);
        let v = capped_array("items", serde_json::json!([1, 2, 3, 4]), 2);
        assert_eq!(v["items"], serde_json::json!([1, 2]));
        assert_eq!(v["count"], 4);
        assert_eq!(v["truncated"], true);
        let v = capped_array("items", serde_json::json!([1, 2]), 0);
        assert_eq!(v["items"], serde_json::json!([1, 2]));
        assert_eq!(v["truncated"], false);
    }
}
