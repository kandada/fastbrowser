// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Device-emulation tools: touch / geolocation / timezone (CDP Emulation domain; real browsers only).

use serde_json::json;

use crate::engine::Capability;
use crate::tools::tool::Tool;

pub fn tools() -> Vec<Tool> {
    vec![set_touch_emulation(), set_geolocation(), set_timezone()]
}

fn set_touch_emulation() -> Tool {
    Tool::new(
        "set_touch_emulation",
        "Enable/disable touch emulation (mobile page semantics).",
        json!({"enabled": {"type": "boolean", "required": true}}),
        r#"{"enabled": true}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let enabled = ctx.param::<bool>("enabled")?;
            ctx.runtime.engine().set_touch_emulation(tab, enabled)?;
            Ok(json!({"enabled": enabled, "ok": true}))
        },
    )
    .requires(&[Capability::Cdp])
}

fn set_geolocation() -> Tool {
    Tool::new(
        "set_geolocation",
        "Override geolocation (latitude/longitude in degrees, accuracy in meters).",
        json!({
            "latitude": {"type": "number", "required": true},
            "longitude": {"type": "number", "required": true},
            "accuracy": {"type": "number", "default": 100.0}
        }),
        r#"{"latitude": 31.23, "longitude": 121.47}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let lat = ctx.param::<f64>("latitude")?;
            let lng = ctx.param::<f64>("longitude")?;
            let accuracy = ctx.param_opt::<f64>("accuracy")?.unwrap_or(100.0);
            ctx.runtime
                .engine()
                .set_geolocation(tab, lat, lng, accuracy)?;
            Ok(json!({"latitude": lat, "longitude": lng, "ok": true}))
        },
    )
    .requires(&[Capability::Cdp])
}

fn set_timezone() -> Tool {
    Tool::new(
        "set_timezone",
        "Override the timezone (IANA id, e.g. \"America/New_York\" or \"Asia/Shanghai\").",
        json!({"timezone_id": {"type": "string", "required": true}}),
        r#"{"timezone_id": "Asia/Shanghai"}"#,
        |ctx| {
            let tab = ctx.target_tab()?;
            let tz = ctx.param_str("timezone_id")?;
            ctx.runtime.engine().set_timezone(tab, &tz)?;
            Ok(json!({"timezone_id": tz, "ok": true}))
        },
    )
    .requires(&[Capability::Cdp])
}
