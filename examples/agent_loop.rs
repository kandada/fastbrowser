// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Runnable Agent-loop example:
//!
//!   cargo run --example agent_loop
//!
//! Demonstrates driving an "open → fill form → submit → extract" flow with the
//! fastbrowser tool set.

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sdk = Fastbrowser::new();
    sdk.init(Config::default())?;

    println!("engine: {}", sdk.get_info().engine);
    println!("tool count: {}", sdk.tool_count());

    // Open the login page.
    let out = sdk.open("https://example.com/login")?;
    println!("opened: {} — {}", out["url"], out["title"]);

    // Perceive.
    let snap = sdk.snapshot()?;
    println!("interactive elements: {}", snap.interactive.len());
    for e in &snap.interactive {
        println!("  [{}] {} {}", e.id, e.tag, e.text.as_deref().unwrap_or(""));
    }

    // Act.
    sdk.tool_call(
        "fill_form",
        json!({"values": {"b": "alice", "c": "secret"}}),
    )?;
    sdk.tool_call("checkbox", json!({"id": "d", "checked": true}))?;
    sdk.tool_call("select_option", json!({"id": "e", "value": "pro"}))?;
    sdk.tool_call("click", json!({"id": "f"}))?;
    println!("form filled and submitted");

    // Extract.
    let links = sdk.tool_call("extract_links", json!({}))?;
    println!(
        "page links: {}",
        links["links"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    // Screenshot.
    let shot = sdk.screenshot()?;
    println!(
        "screenshot: {}x{} (rgba {})",
        shot.width,
        shot.height,
        shot.rgba.len()
    );

    println!("done.");
    Ok(())
}
