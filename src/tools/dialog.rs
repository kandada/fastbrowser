// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! JS dialog tools: pending_dialog / dialog_accept / dialog_dismiss.

use serde_json::json;

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
            Ok(json!({"dialog": dlg}))
        },
    )
}

fn dialog_accept() -> Tool {
    Tool::new(
        "dialog_accept",
        "Accept the pending JS dialog. For prompt dialogs, optionally pass 'prompt_text'.",
        json!({"prompt_text": {"type": "string", "description": "Input for prompt dialogs", "required": false}}),
        r#"{"prompt_text": "yes"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let text = ctx.param_opt::<String>("prompt_text")?;
            ctx.runtime.engine().dialog_accept(tab, text.as_deref())?;
            // Route dialogClosed events to clear the pending dialog.
            let _ = ctx.runtime.engine().drain_events(tab);
            Ok(json!({"accepted": true, "ok": true}))
        },
    )
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
}
