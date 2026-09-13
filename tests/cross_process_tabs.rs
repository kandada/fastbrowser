// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Cross-process tab continuity + full tab discovery (real Chromium).
//!
//! Regression for the "after `open`, the next command cannot see the new tab →
//! false failure → repeated `open` → duplicate pages" problem: each CLI command is
//! a separate process, so it must follow the most-recently-active tab via the
//! stable `targetId` in the manifest, and `list_tabs` must reflect real tabs.

#![cfg(feature = "engine-cdp")]

mod common;

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fastbrowser")
}

/// 单命令模式（独立进程），返回 stdout 解析后的 JSON。
fn run(cfg: &str, args: &[&str]) -> serde_json::Value {
    let out = Command::new(bin())
        .arg("--config")
        .arg(cfg)
        .args(args)
        .output()
        .expect("spawn fastbrowser");
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "non-JSON output '{stdout}': {e} (stderr: {})",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn file_url(dir: &std::path::Path, name: &str, title: &str) -> String {
    let p = dir.join(name);
    std::fs::write(
        &p,
        format!("<!doctype html><title>{title}</title><h1>{title}</h1>"),
    )
    .unwrap();
    format!("file://{}", p.display())
}

fn setup(headless: bool) -> Option<(&'static common::SharedBrowser, common::BrowserGuard)> {
    if common::find_chrome().is_none() {
        eprintln!("SKIP: cross_process_tabs (no chrome found; set CHROME_PATH)");
        return None;
    }
    let b = common::shared_browser(headless).expect("launch chrome");
    let guard = common::browser_guard(b);
    Some((b, guard))
}

/// `open` 必须返回真实 URL（不是创建瞬间的 about:blank），且下一条独立命令
/// 要跟随最近打开的标签；`list_tabs` 要发现所有真实标签并带稳定 target_id。
#[test]
fn open_sticks_across_processes_and_list_tabs_discovers_all() {
    let Some((b, _guard)) = setup(true) else {
        return;
    };
    let ws = b.ws.clone();

    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("browser.json");
    std::fs::write(
        &cfg,
        serde_json::json!({"engine": "chromium", "cdp_url": ws}).to_string(),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();

    // 干净起点：关闭除活动外的所有标签页。
    let _ = run(cfg, &["clear_state"]);

    let url_a = file_url(dir.path(), "a.html", "AAA");
    let url_b = file_url(dir.path(), "b.html", "BBB");

    // 进程 1：open A → 返回真实 URL / 标题（不是 about:blank）
    let a = run(cfg, &["open", &url_a]);
    assert_eq!(
        a["url"],
        url_a.as_str(),
        "open must return the real URL: {a}"
    );
    assert_eq!(a["title"], "AAA", "open must return the real title: {a}");

    // 进程 2：open B → 同样返回真实 URL
    let bres = run(cfg, &["open", &url_b]);
    assert_eq!(
        bres["url"],
        url_b.as_str(),
        "open must return the real URL: {bres}"
    );

    // 进程 3：新进程 get_current_url → 跟随最近 open 的标签（B），而不是旧标签
    let cur = run(cfg, &["get_current_url"]);
    assert_eq!(
        cur["url"],
        url_b.as_str(),
        "active tab must follow the last open across processes: {cur}"
    );

    // 进程 4：list_tabs → 发现所有标签页（含 A 与 B），且每个都带稳定 target_id
    let tabs = run(cfg, &["list_tabs"]);
    let arr = tabs["tabs"].as_array().expect("tabs array");
    let urls: Vec<&str> = arr.iter().filter_map(|t| t["url"].as_str()).collect();
    assert!(
        urls.contains(&url_a.as_str()),
        "list_tabs missing A: {tabs}"
    );
    assert!(
        urls.contains(&url_b.as_str()),
        "list_tabs missing B: {tabs}"
    );
    assert!(
        arr.iter().all(|t| t.get("target_id").is_some()),
        "every tab needs a stable target_id: {tabs}"
    );
}

/// 回归：页面已经加载完成后，`wait_for_navigation` 必须立即返回，不能等满
/// 默认 10s 超时（否则 Agent 在页面已就绪时仍被卡住）。真实 Chromium + 独立进程。
#[test]
fn wait_for_navigation_is_fast_when_page_already_loaded() {
    let Some((b, _guard)) = setup(true) else {
        return;
    };
    let ws = b.ws.clone();

    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("browser.json");
    std::fs::write(
        &cfg,
        serde_json::json!({"engine": "chromium", "cdp_url": ws}).to_string(),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();
    let _ = run(cfg, &["clear_state"]);

    let url = file_url(dir.path(), "wait.html", "WAIT");
    let opened = run(cfg, &["open", &url]);
    assert_eq!(
        opened["url"],
        url.as_str(),
        "open must load the page: {opened}"
    );

    // wait_for_navigation 默认超时 10s；页面已加载 → 必须很快返回。
    let started = std::time::Instant::now();
    let v = run(cfg, &["wait_for_navigation"]);
    let elapsed = started.elapsed();
    assert_eq!(v["navigated"], true, "must report navigated: {v}");
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "already-loaded page must not wait the full timeout (took {elapsed:?})"
    );
}

/// 隔离上下文跨进程复用：多次 `open` 不应每次新建一个 BrowserContext
/// （否则无状态 CLI 会不断泄漏上下文）。`<config>.ctx` 应保持稳定。
#[test]
fn isolated_context_is_reused_across_processes() {
    let Some((b, _guard)) = setup(true) else {
        return;
    };
    let ws = b.ws.clone();

    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("browser.json");
    std::fs::write(
        &cfg,
        serde_json::json!({"engine": "chromium", "cdp_url": ws, "isolated_profiles": true})
            .to_string(),
    )
    .unwrap();
    let cfg = cfg.to_str().unwrap();
    let ctx_path = dir.path().join("browser.ctx");

    let _ = run(cfg, &["clear_state"]);

    let url_a = file_url(dir.path(), "ia.html", "IsoA");
    let url_b = file_url(dir.path(), "ib.html", "IsoB");

    let a = run(cfg, &["open", &url_a]);
    assert_eq!(a["url"], url_a.as_str(), "isolated open must load: {a}");
    let ctx1 = std::fs::read_to_string(&ctx_path)
        .expect("context id persisted after first open")
        .trim()
        .to_string();
    assert!(!ctx1.is_empty(), "context id must be non-empty");

    let b2 = run(cfg, &["open", &url_b]);
    assert_eq!(b2["url"], url_b.as_str(), "isolated open must load: {b2}");
    let ctx2 = std::fs::read_to_string(&ctx_path)
        .unwrap()
        .trim()
        .to_string();

    assert_eq!(
        ctx1, ctx2,
        "subsequent opens must reuse the same browser context (no leak)"
    );
}
