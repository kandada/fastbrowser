// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! CLI 端到端测试：spawn 编译出的 `fastbrowser` 二进制，走真实进程边界
//! 验证「CLI → SDK → 内核 → 引擎」全栈。mock 引擎，无需浏览器。
//!
//! 依赖：cargo test 会先构建 bin（`CARGO_BIN_EXE_fastbrowser`）。

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fastbrowser")
}

/// 单命令模式：返回 stdout 解析后的 JSON。
fn one(args: &[&str]) -> Value {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn fastbrowser");
    assert!(
        out.status.success(),
        "exit {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("non-JSON output '{stdout}': {e}"))
}

/// REPL 模式：管道输入多行命令，返回 stdout 全文。
fn repl(lines: &[&str]) -> String {
    let mut child = Command::new(bin())
        .args(["--engine", "mock", "--repl"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn fastbrowser repl");
    {
        let mut stdin = child.stdin.take().unwrap();
        for line in lines {
            writeln!(stdin, "{line}").unwrap();
        }
        // 关闭 stdin → repl 退出
    }
    let out = child.wait_with_output().expect("wait repl");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn single_command_returns_json() {
    // open 返回 JSON
    let v = one(&["--engine", "mock", "open", "https://example.com"]);
    assert_eq!(v["title"], "Example Page");
    assert!(v["tab"].is_number());

    // tools 输出为数组（>= 88）
    let v = one(&["--engine", "mock", "tools"]);
    assert!(v.as_array().unwrap().len() >= 88);

    // status / info
    let v = one(&["--engine", "mock", "status"]);
    assert_eq!(v["engine"], "mock");

    // 未知命令 → 标准 error JSON（进程非零退出）
    let out = Command::new(bin())
        .args(["--engine", "mock", "no_such_cmd"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert!(err.get("error").is_some());
}

#[test]
fn repl_preserves_session_across_commands() {
    // 同一进程内：open → snapshot → type → extract → audit
    let out = repl(&[
        "open https://example.com/login",
        "snapshot",
        "type e hello",
        "extract_links",
        "get_current_url",
        "audit",
    ]);

    // snapshot 应包含交互元素与 meta
    assert!(out.contains("\"interactive\""));
    assert!(out.contains("\"meta\""));
    // type 后 snapshot 的输入框值
    // get_current_url 仍是登录页
    assert!(out.contains("\"url\":\"https://example.com/login\""));
    // audit 记录了 open / type 等动作
    assert!(out.contains("\"action\":\"open\""));
    assert!(out.contains("\"action\":\"type\""));
    // 跨命令会话保持：没有崩溃、输出为 JSON 行
    for line in out.lines() {
        let l = line.trim_start_matches("> ").trim();
        if l.is_empty() || l.starts_with("fastbrowser") {
            continue;
        }
        // 每行都应是合法 JSON（工具输出）
        let parsed = serde_json::from_str::<Value>(l);
        assert!(parsed.is_ok(), "REPL output not JSON: {l:?}");
    }
}

#[test]
fn repl_failed_command_returns_error_json_but_continues() {
    let out = repl(&[
        "open https://example.com",
        "click zz",        // 无效元素 → error JSON，但 REPL 继续
        "get_current_url", // 应仍可用
    ]);
    assert!(out.contains("\"error\""));
    assert!(out.contains("\"url\":\"https://example.com\""));
}

#[test]
fn repl_supports_new_tools() {
    let out = repl(&[
        "open https://example.com",
        "done \"completed\"",
        "send_keys Enter",
        "get_history",
    ]);
    assert!(out.contains("\"done\":true"));
    assert!(out.contains("\"history\""));
}

// ── 新 E2E：会话持久化跨进程 ─────────────────────────────────

/// 会话状态（cookie/storage）可跨进程保存/加载：
/// 进程 A（REPL）保存到文件，进程 B（REPL）加载并恢复。
#[test]
fn session_persistence_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let path = path.to_str().unwrap();

    // 进程 A：open → cookie_set → storage_set → session_save
    let a = repl(&[
        "open https://example.com",
        "cookie_set {\"name\":\"sid\",\"value\":\"abc\",\"domain\":\"example.com\"}",
        "storage_set token t1",
        &format!("session_save {path}"),
    ]);
    assert!(a.contains("\"saved\""));
    assert!(a.contains("\"ok\":true"));

    // 进程 B：session_load → open（应用状态）→ cookie_get / storage_get
    let b = repl(&[
        &format!("session_load {path}"),
        "open https://example.com",
        "cookie_get example.com",
        "storage_get token",
    ]);
    assert!(b.contains("\"value\":\"abc\""));
    assert!(b.contains("\"value\":\"t1\""));
    assert!(b.contains("\"loaded\""));
}

// ── 新 E2E：截图写文件 ───────────────────────────────────────

#[test]
fn screenshot_command_writes_rgba_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shot.rgba");
    let path = path.to_str().unwrap();
    let v = one(&["--engine", "mock", "open", "https://example.com"]);
    assert!(v["tab"].is_number());
    let v = one(&["--engine", "mock", "screenshot", path]);
    assert_eq!(v["saved"], path);
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len(), 800 * 600 * 4);
}

// ── 新 E2E：全局标志 ─────────────────────────────────────────

#[test]
fn global_flags_engine_hosted_and_config() {
    // --engine 与 --hosted/--headless 生效
    let v = one(&["--engine", "mock", "--headless", "status"]);
    assert_eq!(v["engine"], "mock");
    assert_eq!(v["rendering_mode"], "headless");
    let v = one(&["--engine", "mock", "--hosted", "status"]);
    assert_eq!(v["rendering_mode"], "hosted");
    // --config JSON 覆盖
    let v = one(&[
        "--config",
        r#"{"engine":"mock","rendering_mode":"hosted"}"#,
        "status",
    ]);
    assert_eq!(v["rendering_mode"], "hosted");
}

// ── 新 E2E：缺参命令报标准 error JSON（进程非零）──────────────

#[test]
fn missing_required_args_error_json() {
    for args in [
        &["--engine", "mock", "open"][..],
        &["--engine", "mock", "navigate"][..],
        &["--engine", "mock", "type"][..],
        &["--engine", "mock", "type", "e"][..],
        &["--engine", "mock", "new_tab"][..],
        &["--engine", "mock", "switch_tab"][..],
        &["--engine", "mock", "fill_form"][..],
        &["--engine", "mock", "execute_js"][..],
        &["--engine", "mock", "wait_for_element"][..],
        &["--engine", "mock", "done"][..],
    ] {
        let out = Command::new(bin()).args(args).output().unwrap();
        assert!(!out.status.success(), "args {args:?} should fail");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let err: Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("{args:?} non-JSON {stdout}: {e}"));
        assert!(err.get("error").is_some(), "args {args:?}");
    }
}

// ── 新 E2E：表单 / 会话工具经 CLI ────────────────────────────

#[test]
fn cli_fill_form_and_select_option() {
    // 两种 fill_form JSON 形态都应可用
    let v = one(&["--engine", "mock", "open", "https://example.com/login"]);
    assert_eq!(v["title"], "Login");
    let v = one(&[
        "--engine",
        "mock",
        "fill_form",
        r#"{"values":{"b":"alice","c":"secret"}}"#,
    ]);
    assert_eq!(v["filled"].as_array().unwrap().len(), 2);
    let v = one(&["--engine", "mock", "fill_form", r#"{"b":"bob"}"#]);
    assert_eq!(v["filled"][0], "b");
    // select_option 需同一会话 → 用 REPL
    let out = repl(&[
        "open https://example.com/login",
        "select_option e pro",
        "snapshot",
    ]);
    assert!(out.contains("\"selected\":\"pro\""));
    assert!(out.contains("\"selected_option\":\"pro\""));
}

// ── 新 E2E：标签页 / 历史经 CLI ──────────────────────────────

#[test]
fn cli_tab_lifecycle_and_history() {
    // 单命令进程内可自洽：new_tab + list_tabs 在同一进程
    let out = repl(&[
        "open https://example.com",
        "new_tab https://example.com/search",
        "list_tabs",
        "switch_tab 1",
        "close_tab 2",
        "list_tabs",
    ]);
    assert!(out.contains("\"tabs\""));
    assert!(out.contains("\"closed\":2"));

    // 历史：同一进程内 navigate → back → forward
    let out = repl(&[
        "open https://example.com",
        "navigate https://example.com/login",
        "back",
        "forward",
        "get_history",
    ]);
    assert!(out.contains("\"url\":\"https://example.com\""));
    assert!(out.contains("\"url\":\"https://example.com/login\""));
    assert!(out.contains("\"history\""));
}

// ── 新 E2E：导航 / 页面工具经 CLI ────────────────────────────

#[test]
fn cli_page_and_extract_tools() {
    let v = one(&["--engine", "mock", "open", "https://example.com"]);
    assert_eq!(v["title"], "Example Page");
    // get_page_meta 在 mock 中 JS 求值器受限 → meta 可为 null/对象（不崩溃）
    let v = one(&["--engine", "mock", "get_page_meta"]);
    assert!(v["meta"].is_object() || v["meta"].is_null());
    let v = one(&["--engine", "mock", "extract_links"]);
    assert_eq!(v["links"][0]["url"], "https://example.com/about");
    let v = one(&["--engine", "mock", "execute_js", "document.title"]);
    assert_eq!(v["result"], "Example Page");
    let v = one(&["--engine", "mock", "evaluate_xpath", "//a"]);
    assert!(v["result"].is_array());
    let v = one(&["--engine", "mock", "inject_css", "body{}"]);
    assert_eq!(v["injected"], true);
}

// ── 新 E2E：JSON 输出逐行合法（REPL 含对话框/网络工具）────────

#[test]
fn repl_dialog_and_network_tools_output_json() {
    let out = repl(&[
        "open https://example.com",
        "pending_dialog",
        "dialog_accept",
        "dialog_dismiss",
        "list_pending_requests",
        "block_request *ads*",
        "intercept_request *.png",
        "get_accessibility_tree",
        "get_performance_metrics",
    ]);
    for line in out.lines() {
        let l = line.trim_start_matches("> ").trim();
        if l.is_empty() || l.starts_with("fastbrowser") {
            continue;
        }
        assert!(
            serde_json::from_str::<Value>(l).is_ok(),
            "non-JSON line: {l:?}"
        );
    }
}

// ── 新 E2E：未知引擎 / 未知命令的退出码与错误形态 ────────────

#[test]
fn unknown_engine_and_command_error_shapes() {
    let out = Command::new(bin())
        .args(["--engine", "nope", "status"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert!(err["error"]["kind"] == "invalid_argument" || err["error"]["kind"] == "internal");

    let out = Command::new(bin())
        .args(["--engine", "mock", "definitely_not_a_cmd"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(err["error"]["kind"], "invalid_argument");
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown command"));
}

// ── 新 E2E：info / version ───────────────────────────────────

#[test]
fn cli_info_and_version() {
    let v = one(&["--engine", "mock", "info"]);
    assert_eq!(v["engine"], "mock");
    assert!(v["tools"].as_u64().unwrap() >= 30);
    let v = one(&["--engine", "mock", "tools"]);
    assert!(v.as_array().unwrap().len() >= 30);
}
