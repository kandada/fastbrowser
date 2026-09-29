// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 工具注册表：聚合全部内置工具，供 SDK / CLI / LLM 使用。

use serde_json::Value;

use crate::engine::{EngineCapabilities, EngineError, ErrorKind, Result};
use crate::tools::advanced;
use crate::tools::agent;
use crate::tools::cookies;
use crate::tools::device;
use crate::tools::dialog;
use crate::tools::download;
use crate::tools::extract;
use crate::tools::form;
use crate::tools::interact;
use crate::tools::nav;
use crate::tools::network;
use crate::tools::page;
use crate::tools::pdf;
use crate::tools::tabs;
use crate::tools::tool::Tool;
use crate::tools::wait;

/// 全部内置工具（按功能域聚合）。
pub fn builtin_tools() -> Vec<Tool> {
    let mut tools = Vec::new();
    tools.extend(nav::tools());
    tools.extend(interact::tools());
    tools.extend(extract::tools());
    tools.extend(wait::tools());
    tools.extend(form::tools());
    tools.extend(page::tools());
    tools.extend(cookies::tools());
    tools.extend(advanced::tools());
    tools.extend(tabs::tools());
    tools.extend(agent::tools());
    tools.extend(dialog::tools());
    tools.extend(network::tools());
    tools.extend(pdf::tools());
    tools.extend(device::tools());
    tools.extend(download::tools());
    #[cfg(feature = "surface")]
    tools.extend(crate::tools::surface::tools());
    tools
}

/// 工具注册表（只读查找）。
pub struct ToolRegistry {
    tools: Vec<Tool>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        ToolRegistry {
            tools: builtin_tools(),
        }
    }

    pub fn find(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.spec.name == name)
    }

    /// 近似工具名建议（用于 `unknown tool` 时给出 “did you mean …?”）。
    pub fn suggest(&self, name: &str) -> Option<String> {
        closest_name(name, &self.names()).map(String::from)
    }

    /// 按名称执行工具。工具声明的能力不被当前引擎支持时，返回明确的 Unsupported 错误。
    pub fn call(
        &self,
        runtime: &crate::bridge::Runtime,
        name: &str,
        params: Value,
    ) -> Result<Value> {
        // Process-wide serialization: browser automation shares mutable tab /
        // engine state, so concurrent calls (an LLM issuing parallel tool calls)
        // must not interleave — otherwise you get races like
        // "element not found" / "js error". No tool re-enters `call`, so this
        // cannot deadlock.
        static TOOL_CALL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _serial = TOOL_CALL_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Accept Playwright MCP / browser-use tool + param names.
        let name = crate::tools::aliases::resolve_tool_alias(name);
        let tool = self.find(&name).ok_or_else(|| {
            let mut msg = format!("unknown tool '{name}'");
            if let Some(s) = self.suggest(&name) {
                msg.push_str(&format!("; did you mean '{s}'?"));
            }
            msg.push_str(" (call tool 'tools' to list available tools)");
            EngineError::new(ErrorKind::InvalidArgument, msg)
        })?;
        let (params, warnings) = crate::tools::aliases::prepare_params(params, &tool.spec.params);
        let caps = runtime.engine().capabilities();
        if !tool.supported_by(&caps) {
            return Err(EngineError::unsupported(format!(
                "tool '{name}' requires a capability the '{}' engine does not support",
                runtime.engine().name()
            )));
        }
        let mut ctx = crate::tools::tool::ToolContext { runtime, params };
        (tool.run)(&mut ctx)
            .map(|v| crate::tools::aliases::attach_warnings(v, warnings))
            .map_err(|e| crate::tools::tool::enrich_tool_error(e, &tool.spec))
    }

    /// 工具清单（给 LLM 的 JSON）。
    pub fn list(&self) -> Value {
        Value::Array(self.tools.iter().map(|t| t.params_json()).collect())
    }

    /// 按引擎能力过滤后的工具清单（能力位驱动工具收敛：LLM 只看得到可用的子集）。
    pub fn list_for(&self, caps: &EngineCapabilities) -> Value {
        Value::Array(
            self.tools
                .iter()
                .filter(|t| t.supported_by(caps))
                .map(|t| t.params_json())
                .collect(),
        )
    }

    /// 按引擎能力过滤后的工具数量。
    pub fn count_for(&self, caps: &EngineCapabilities) -> usize {
        self.tools.iter().filter(|t| t.supported_by(caps)).count()
    }

    /// 工具名列表。
    pub fn names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.spec.name.to_string()).collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// 全部工具数（静态统计）。
    pub fn count() -> usize {
        builtin_tools().len()
    }
}

/// Levenshtein 距离（小写化后比较）。
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 在候选名里找与 `name` 最接近的一个（距离阈值内）；无则 `None`。
pub fn closest_name<'a>(name: &str, names: &'a [String]) -> Option<&'a str> {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    // Prefer a prefix/extension match (e.g. "wait" → "wait_for_load_state")
    // over an unrelated short Levenshtein neighbour ("back").
    let mut prefix: Option<&str> = None;
    for n in names {
        let nl = n.to_ascii_lowercase();
        if nl.starts_with(&lower) || lower.starts_with(&nl) {
            if prefix.map(|p| n.len() < p.len()).unwrap_or(true) {
                prefix = Some(n.as_str());
            }
        }
    }
    if prefix.is_some() {
        return prefix;
    }
    let mut best: Option<(usize, &str)> = None;
    for n in names {
        let d = levenshtein(&lower, &n.to_ascii_lowercase());
        if best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, n.as_str()));
        }
    }
    let (d, n) = best?;
    let threshold = 3.max(name.chars().count() / 3);
    (d <= threshold).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_all_domains() {
        let reg = ToolRegistry::new();
        let names = reg.names();
        for n in [
            "navigate",
            "click",
            "type",
            "extract_text",
            "wait_for_element",
            "fill_form",
            "screenshot",
            "cookie_get",
            "execute_js",
            "new_tab",
        ] {
            assert!(names.iter().any(|x| x == n), "missing {n}");
        }
        // 超过 30 个工具
        assert!(reg.len() >= 30, "len={}", reg.len());
    }

    #[test]
    fn list_is_json_array_of_specs() {
        let reg = ToolRegistry::new();
        let list = reg.list();
        let arr = list.as_array().unwrap();
        assert!(arr[0]["name"].is_string());
        assert!(arr[0]["description"].is_string());
        assert!(arr[0]["params"].is_object() || arr[0]["params"].is_string());
    }

    #[test]
    fn capability_filtering_hides_unsupported_tools() {
        let reg = ToolRegistry::new();
        let full = EngineCapabilities::full();
        let js = EngineCapabilities::js_injection();
        // surface_* 工具要求 Capability::Surface（运行时装配），full() 不声明该位。
        #[cfg(feature = "surface")]
        let surface_tools = crate::tools::surface::tools().len();
        #[cfg(not(feature = "surface"))]
        let surface_tools = 0usize;

        // full 引擎暴露除 surface 外的全部工具
        assert_eq!(reg.count_for(&full), reg.len() - surface_tools);
        assert_eq!(
            reg.list_for(&full).as_array().unwrap().len(),
            reg.len() - surface_tools
        );

        // js_injection（webview）缺少 network_control、cdp、history、dialogs、
        // file_input → 额外隐藏 17 个（screenshot 现由 Capability::Screenshot
        // 提供，webview 已实现，故不再隐藏）
        let filtered = reg.list_for(&js);
        let names: Vec<&str> = filtered
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for hidden in [
            "block_request",
            "intercept_request",
            "list_pending_requests",
            "fulfill_request",
            "modify_response",
            "continue_request",
            "abort_request",
            "save_as_pdf",
            "set_touch_emulation",
            "set_geolocation",
            "set_timezone",
            "set_basic_auth",
            "get_history",
            "dialog_accept",
            "dialog_dismiss",
            "upload_file",
            "get_network_log",
        ] {
            assert!(!names.contains(&hidden), "{hidden} should be hidden");
        }
        assert_eq!(reg.count_for(&js), reg.len() - 17 - surface_tools);
    }

    #[test]
    fn tool_supported_by_caps() {
        let reg = ToolRegistry::new();
        let full = EngineCapabilities::full();
        let js = EngineCapabilities::js_injection();

        assert!(reg.find("block_request").unwrap().supported_by(&full));
        assert!(!reg.find("block_request").unwrap().supported_by(&js));
        assert!(reg.find("save_as_pdf").unwrap().supported_by(&full));
        assert!(!reg.find("save_as_pdf").unwrap().supported_by(&js));

        // 无能力要求的工具在任何引擎上都可用
        assert!(reg.find("click").unwrap().supported_by(&js));
        assert!(reg.find("type").unwrap().supported_by(&js));
        assert!(reg.find("navigate").unwrap().supported_by(&js));
    }

    #[test]
    fn element_and_tab_tools_declare_their_aliases() {
        // Every param a tool accepts must be declared, otherwise the caller gets
        // a confusing "unknown param 'x'" warning and the value is ignored.
        let reg = ToolRegistry::new();
        for (tool, keys) in [
            ("extract_text", ["selector", "ref", "element"].as_slice()),
            ("extract_links", ["selector", "ref", "element"].as_slice()),
            (
                "get_element_text",
                ["selector", "ref", "element"].as_slice(),
            ),
            (
                "assert_text_contains",
                ["selector", "ref", "element"].as_slice(),
            ),
            ("switch_tab", ["id"].as_slice()),
            ("get_tab", ["tab", "id"].as_slice()),
            ("scroll", ["x", "y"].as_slice()),
            ("cookie_set", ["url"].as_slice()),
            ("find_elements", ["role", "name"].as_slice()),
            ("select_option", ["label"].as_slice()),
            ("blur", ["selector", "ref", "id"].as_slice()),
            ("cookie_get", ["domain", "name"].as_slice()),
            ("extract_table", ["selector", "index"].as_slice()),
            ("fill_form", ["values"].as_slice()),
        ] {
            let t = reg
                .find(tool)
                .unwrap_or_else(|| panic!("missing tool {tool}"));
            let params = t.spec.params.as_object().unwrap();
            for k in keys {
                assert!(params.contains_key(*k), "{tool} must declare param '{k}'");
            }
        }
    }

    #[test]
    fn suggest_prefers_prefix_match() {
        let names: Vec<String> = ["wait_for_element", "wait_for_load_state", "back", "click"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = closest_name("wait", &names).unwrap();
        assert!(s.starts_with("wait_"), "got {s}");
        assert_eq!(closest_name("clik", &names), Some("click"));
        assert_eq!(closest_name("", &names), None);
    }
}
