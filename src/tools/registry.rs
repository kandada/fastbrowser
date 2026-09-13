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

    /// 按名称执行工具。工具声明的能力不被当前引擎支持时，返回明确的 Unsupported 错误。
    pub fn call(
        &self,
        runtime: &crate::bridge::Runtime,
        name: &str,
        params: Value,
    ) -> Result<Value> {
        let tool = self.find(name).ok_or_else(|| {
            EngineError::new(ErrorKind::InvalidArgument, format!("unknown tool '{name}'"))
        })?;
        let caps = runtime.engine().capabilities();
        if !tool.supported_by(&caps) {
            return Err(EngineError::unsupported(format!(
                "tool '{name}' requires a capability the '{}' engine does not support",
                runtime.engine().name()
            )));
        }
        let mut ctx = crate::tools::tool::ToolContext { runtime, params };
        (tool.run)(&mut ctx)
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

        // full 引擎暴露全部工具
        assert_eq!(reg.count_for(&full), reg.len());
        assert_eq!(reg.list_for(&full).as_array().unwrap().len(), reg.len());

        // js_injection（webview）缺少 network_control 与 cdp → 隐藏 7 个工具
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
            "screenshot",
            "screenshot_element",
            "set_touch_emulation",
            "set_geolocation",
            "set_timezone",
            "set_basic_auth",
        ] {
            assert!(!names.contains(&hidden), "{hidden} should be hidden");
        }
        assert_eq!(reg.count_for(&js), reg.len() - 14);
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
}
