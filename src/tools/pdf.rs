// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! PDF export tool: save_as_pdf.

use serde_json::json;

use crate::engine::Capability;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![save_as_pdf()]
}

fn save_as_pdf() -> Tool {
    Tool::new(
        "save_as_pdf",
        "Save the current page as a PDF file at 'path'. Returns the file path and byte size.",
        json!({"path": {"type": "string", "description": "Output file path", "required": true}}),
        r#"{"path": "/tmp/page.pdf"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let path = ctx.param_str("path")?;
            let bytes = ctx.runtime.engine().print_to_pdf(tab)?;
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &bytes)?;
            Ok(json!({"path": path, "bytes": bytes.len(), "ok": true}))
        },
    )
    .requires(&[Capability::Cdp])
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

    #[test]
    fn save_pdf_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.pdf");
        let path_str = path.to_str().unwrap().to_string();
        let r = runtime();
        let v = save_as_pdf()
            .run(&ToolContext {
                runtime: &r,
                params: json!({"path": path_str}),
            })
            .unwrap();
        assert_eq!(v["ok"], true);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        // mock returns a minimal PDF header
        assert_eq!(&bytes[..8], b"%PDF-1.4");
        let _ = Value::Null;
    }
}
