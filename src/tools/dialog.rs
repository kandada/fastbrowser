// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! JS dialog tools: pending_dialog / dialog_accept / dialog_dismiss.

use serde_json::json;

use crate::engine::Capability;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![pending_dialog(), dialog_accept(), dialog_dismiss()]
}

fn pending_dialog() -> Tool {
    Tool::new(
        "pending_dialog",
        "Return the currently pending JS dialog (alert/confirm/prompt), or null if none.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            // Route CDP events first so dialogOpening/Closed state is up to date.
            let _ = ctx.runtime.engine().drain_events(tab);
            let dlg = ctx.runtime.engine().pending_dialog(tab);
            if dlg.is_some() {
                return Ok(json!({"dialog": dlg}));
            }
            // Webview fallback: read the injected dialog buffer (see
            // WebViewUIDelegate); the last entry is the most recent dialog.
            if let Ok(v) = ctx.eval(
                tab,
                "JSON.stringify((window.__fbDialogs||[]).slice(-1)[0]||null)",
            ) {
                if let Some(s) = v.as_str() {
                    if let Ok(d) = serde_json::from_str::<serde_json::Value>(s) {
                        return Ok(json!({"dialog": d}));
                    }
                }
            }
            Ok(json!({"dialog": null}))
        },
    )
}

fn dialog_accept() -> Tool {
    Tool::new(
        "dialog_accept",
        "Handle the pending JS dialog. Accepts by default; pass 'accept': false to dismiss (Playwright MCP's browser_handle_dialog). For prompt dialogs, optionally pass 'prompt_text'.",
        json!({
            "prompt_text": {"type": "string", "description": "Input for prompt dialogs", "required": false},
            "accept": {"type": "boolean", "default": true, "description": "true=accept, false=dismiss"}
        }),
        r#"{"prompt_text": "yes"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let accept = ctx.param_opt::<bool>("accept")?.unwrap_or(true);
            if !accept {
                ctx.runtime.engine().dialog_dismiss(tab)?;
                let _ = ctx.runtime.engine().drain_events(tab);
                return Ok(json!({"dismissed": true, "ok": true}));
            }
            let text = ctx.param_opt::<String>("prompt_text")?;
            ctx.runtime.engine().dialog_accept(tab, text.as_deref())?;
            // Route dialogClosed events to clear the pending dialog.
            let _ = ctx.runtime.engine().drain_events(tab);
            Ok(json!({"accepted": true, "ok": true}))
        },
    )
    // 原生 JS 对话框处理需要引擎支持（webview 未实现 → 隐藏）。
    .requires(&[Capability::Dialogs])
}

fn dialog_dismiss() -> Tool {
    Tool::new(
        "dialog_dismiss",
        "Dismiss / cancel the pending JS dialog.",
        json!({}),
        r#"{}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            ctx.runtime.engine().dialog_dismiss(tab)?;
            let _ = ctx.runtime.engine().drain_events(tab);
            Ok(json!({"dismissed": true, "ok": true}))
        },
    )
    .requires(&[Capability::Dialogs])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Runtime;
    use crate::config::Config;
    use crate::engine::TabOptions;
    use crate::tools::tool::ToolContext;
    use serde_json::Value;

    fn runtime() -> Runtime {
        let r = Runtime::new(
            Box::new(crate::engines::mock::MockEngine::new()),
            Config::default(),
        );
        r.engine()
            .create_tab("https://example.com", &TabOptions::default())
            .unwrap();
        r
    }

    fn call(tool: &Tool, r: &Runtime, params: Value) -> Value {
        tool.run(&ToolContext { runtime: r, params }).unwrap()
    }

    #[test]
    fn dialog_roundtrip() {
        let r = runtime();
        let none = call(&pending_dialog(), &r, json!({}));
        assert_eq!(none["dialog"], Value::Null);
        let _ = call(&dialog_accept(), &r, json!({}));
        let _ = call(&dialog_dismiss(), &r, json!({}));
    }

    #[test]
    fn dialog_accept_false_dismisses() {
        let r = runtime();
        // Playwright MCP's browser_handle_dialog {accept:false} → dismiss.
        let v = call(&dialog_accept(), &r, json!({"accept": false}));
        assert_eq!(v["dismissed"], json!(true), "{v}");
        let v = call(&dialog_accept(), &r, json!({"accept": true}));
        assert_eq!(v["accepted"], json!(true), "{v}");
    }
}
