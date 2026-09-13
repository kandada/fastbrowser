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

    /// 从 `id`（快照编号）或 `ref`（{kind,value}）解析元素引用。
    pub fn element_ref(&self) -> Result<ElementRef> {
        if let Some(id) = self.params.get("id").and_then(|v| v.as_str()) {
            let c = id
                .chars()
                .next()
                .ok_or_else(|| EngineError::invalid("'id' must be a single letter"))?;
            return Ok(ElementRef::snapshot(c));
        }
        if let Some(r) = self.params.get("ref") {
            return deserialize_ref(r);
        }
        Err(EngineError::invalid(
            "need either 'id' (snapshot letter) or 'ref' {kind, value}",
        ))
    }

    /// 返回参数中的 `tab`（可选）。
    pub fn tab_param(&self) -> Result<Option<TabId>> {
        self.param_opt::<u32>("tab").map(|t| t.map(TabId))
    }

    /// 由 id/ref 解析元素并返回其快照信息（用于 hover/拖拽坐标）。
    pub fn element_snapshot(&self, tab: TabId) -> Result<crate::engine::InteractiveElement> {
        let r = self.element_ref()?;
        let snap = self.runtime.engine().snapshot(tab)?;
        match r.kind {
            RefKind::Snapshot => {
                snap.element_by_id(
                    r.value
                        .chars()
                        .next()
                        .ok_or_else(|| EngineError::invalid("bad snapshot id"))?,
                )
                .cloned()
                .ok_or_else(|| EngineError::new(ErrorKind::Dom, format!("element {r:?} not found")))
            }
            _ => snap
                .interactive
                .iter()
                .find(|el| el.matches_ref(&r))
                .cloned()
                .ok_or_else(|| {
                    EngineError::new(ErrorKind::Dom, format!("element {r:?} not found"))
                }),
        }
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
        "snapshot" => {
            let c = value
                .chars()
                .next()
                .ok_or_else(|| EngineError::invalid("snapshot id empty"))?;
            Ok(ElementRef::snapshot(c))
        }
        other => Err(EngineError::invalid(format!(
            "unknown ref.kind '{other}' (css|xpath|text|role|snapshot)"
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
