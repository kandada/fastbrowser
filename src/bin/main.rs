// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! fastbrowser CLI（browserctl 风格）—— 终端命令 SDK。
//!
//! 命令与工具一一对应，输出统一为 JSON，方便 LLM 与脚本消费。
//!
//! 用法示例：
//!   fastbrowser --engine mock open https://example.com
//!   fastbrowser snapshot
//!   fastbrowser click a
//!   fastbrowser type a "hello"
//!   fastbrowser extract_links
//!   fastbrowser screenshot /tmp/shot.png
//!   fastbrowser tools

use serde_json::{json, Value};

use fastbrowser::config::Config;
use fastbrowser::engine::RenderingMode;
use fastbrowser::sdk::Fastbrowser;

const USAGE: &str = r#"fastbrowser — browser automation CLI (browserctl style)

Usage:
  fastbrowser [--engine <mock|cef|webview>] [--hosted|--headless] [--config <json>] <command> [args...]
  fastbrowser --repl                       # interactive: run line by line, keep the session across commands

Commands (output is JSON):
  open <url> | navigate <url>
  back | forward | reload | stop
  snapshot | screenshot [file] | get_page_title | get_current_url | get_page_text | get_element_info <id>
  click <id> | dblclick <id> | type <id> <text> | press <key> | hover <id> | scroll [dx] [dy] | swipe <fx> <fy> <tx> <ty>
  extract_text | extract_html | extract_links | extract_images | extract_table | extract_json <script>
  wait_for_element <selector> [timeout_ms] | wait_for_navigation [timeout_ms] | wait_for_load_state <state> [timeout_ms]
  wait_for_condition <script> [timeout_ms] | wait_for_text <text> [timeout_ms]
  assert_url_contains <text> | assert_title <text>
  get_element_text <id> | get_attributes <id> | is_visible <id> | is_enabled <id> | screenshot_element <id>
  get_focused_element | get_selected_text | get_page_meta | get_scroll_position | set_scroll_position <x> <y> | get_performance_metrics | get_accessibility_tree
  right_click <id> | focus <id> | blur | clear_input <id>
  extract_forms | storage_get_all | clear_cookies | clear_storage
  get_tab <title> | duplicate_tab | close_other_tabs
  session_save <path> | session_load <path> | clear_state
  fill_form <json> | select_option <id> <value> | checkbox <id> <bool> | radio <id>
  cookie_get [domain] | cookie_set <json> | cookie_clear [domain] | storage_get <key> | storage_set <key> <value>
  execute_js <script> | evaluate_xpath <expr> | inject_css <css>
  block_request <pattern>... [off] | intercept_request <pattern>... [off]
  new_tab <url> | close_tab [tab] | switch_tab <tab> | list_tabs | get_active_tab
  tools | status | info
"#;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("{USAGE}");
        std::process::exit(0);
    }

    let (config, config_source, rest) = parse_global_flags(&args);

    // 宿主注入的连接清单：`--config` 未显式给出时，从 `FASTBROWSER_CONFIG`
    // 读取 browser.json（连接一个常驻浏览器实例，而非每次新起）。
    let (config, config_source) = apply_env_config(config, config_source);

    // 跨调用保活：宿主未显式指定连接目标（Electron 浏览器视图）时，恢复上次
    // 会话的活跃标签页。状态文件记录的是**稳定 targetId**（而非跨连接不稳定的
    // 本地编号），因此在 `init` 前把它注入 `config.cdp_target_id`，让引擎直接
    // 附加到上一进程遗留的那个页面 —— 否则新连接只会盲选"第一个 page target"
    // （通常是浏览器启动时的 about:blank），`open → get_current_url` 会错页。
    //
    // 注：每条命令结束后会把当前活跃 targetId 回写到 manifest 的
    // `cdp_target_id`（见 `persist_active_tab`），因此下一进程即使 manifest 已
    // 带绑定也会跟随最新活跃标签；宿主重新绑定（重写 manifest）时自然覆盖。
    let mut config = config;
    if config.cdp_target_id.is_none() && config.cdp_target_url_contains.is_none() {
        if let Some(tid) = read_persisted_target(config_source.as_deref()) {
            config.cdp_target_id = Some(tid);
        }
    }
    // 复用上次的隔离浏览器上下文（宿主未显式指定时）。
    if config.browser_context_id.is_none() {
        if let Some(cid) = read_persisted_context(config_source.as_deref()) {
            config.browser_context_id = Some(cid);
        }
    }

    if rest.first().map(String::as_str) == Some("--repl")
        || rest.first().map(String::as_str) == Some("repl")
    {
        run_repl(config);
        return;
    }

    let sdk = Fastbrowser::new();
    if let Err(e) = sdk.init(config) {
        println!("{}", e.to_json());
        std::process::exit(1);
    }

    if rest.is_empty() {
        println!(
            "{}",
            json!({"error": "no command given", "hint": "run --help"})
        );
        std::process::exit(1);
    }

    let cmd = &rest[0];
    let params = &rest[1..];
    let result = dispatch(&sdk, cmd, params);
    match result {
        Ok(v) => {
            persist_active_tab(&sdk, config_source.as_deref());
            persist_context(&sdk, config_source.as_deref());
            println!("{v}");
        }
        Err(e) => {
            println!("{}", e.to_json());
            std::process::exit(1);
        }
    }
}

/// 宿主注入的连接清单：`--config` 未显式给出时，从 `FASTBROWSER_CONFIG`
/// 环境变量读取 browser.json 作为配置来源。
fn apply_env_config(mut config: Config, config_source: Option<String>) -> (Config, Option<String>) {
    if config_source.is_some() {
        return (config, config_source);
    }
    let env_source = std::env::var("FASTBROWSER_CONFIG")
        .ok()
        .filter(|p| !p.trim().is_empty());
    if let Some(path) = env_source {
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<Config>(&s) {
                config = cfg;
                return (config, Some(path));
            }
        }
    }
    (config, config_source)
}

/// `<config>.<ext>` → `<config>.tab`（活跃标签状态文件）。
fn tab_state_path(config_source: Option<&str>) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(config_source?);
    if p.exists() && p.is_file() {
        Some(p.with_extension("tab"))
    } else {
        None
    }
}

/// 读取上次会话持久化的活跃标签 **targetId**。
///
/// 早期版本写入的是跨连接不稳定的本地标签编号（数字），新连接无法据其定位
/// 目标，故遇到非 targetId 内容时视为无状态（返回 None），由引擎兜底选目标。
fn read_persisted_target(config_source: Option<&str>) -> Option<String> {
    let path = tab_state_path(config_source)?;
    let s = std::fs::read_to_string(&path).ok()?;
    let id = s.trim().to_string();
    // targetId 是 CDP 生成的十六进制串；纯数字 / 空串都是旧格式或无效。
    if id.is_empty() || id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(id)
}

/// 当前活动标签页的稳定 CDP targetId。
///
/// 引擎暴露的 page 级端点形如 `ws://host:port/devtools/page/<targetId>`；
/// 从中解析出 targetId（浏览器级连接才有；page 级直连无 target_id 时返回 None）。
fn active_target_id(sdk: &Fastbrowser) -> Option<String> {
    let ep = sdk.cdp_endpoint()?;
    let marker = "/devtools/page/";
    let idx = ep.rfind(marker)?;
    let id = ep[idx + marker.len()..].to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// 把当前活跃标签的 targetId 持久化，供下一次 CLI 调用（或 Agent 的下一条
/// `run_shell`）跟随同一个页面 —— 这是"跨命令保活"的关键：只有稳定 targetId
/// 能在不同 CDP 连接之间唯一定位标签页。
///
/// 双写：
/// 1. 回写 manifest 的 `cdp_target_id`（单一事实源，下一次 `--config` 直接读取）；
/// 2. 兼容旧读取方，同时写 `<config>.tab`。
fn persist_active_tab(sdk: &Fastbrowser, config_source: Option<&str>) {
    let Some(target_id) = active_target_id(sdk) else {
        return;
    };
    if let Some(manifest) = config_source {
        let mpath = std::path::Path::new(manifest);
        if mpath.is_file() {
            let _ = update_manifest_target(mpath, &target_id);
        }
    }
    if let Some(path) = tab_state_path(config_source) {
        let _ = std::fs::write(&path, &target_id);
    }
}

/// 把活跃 targetId 回写进 manifest（保留其它字段；临时文件 + rename 原子替换）。
fn update_manifest_target(path: &std::path::Path, target_id: &str) -> Option<()> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut v: Value = serde_json::from_str(&text).ok()?;
    let obj = v.as_object_mut()?;
    if obj.get("cdp_target_id").and_then(Value::as_str) == Some(target_id) {
        return Some(());
    }
    obj.insert(
        "cdp_target_id".to_string(),
        Value::String(target_id.to_string()),
    );
    let out = serde_json::to_string_pretty(&v).ok()?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, out).ok()?;
    std::fs::rename(&tmp, path).ok()
}

/// `<config>.ctx`（隔离浏览器上下文原生 id 的状态文件）。
fn context_state_path(config_source: Option<&str>) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(config_source?);
    if p.exists() && p.is_file() {
        Some(p.with_extension("ctx"))
    } else {
        None
    }
}

/// 读取上次会话持久化的浏览器上下文原生 id（跨进程复用，避免每次新建上下文）。
fn read_persisted_context(config_source: Option<&str>) -> Option<String> {
    let path = context_state_path(config_source)?;
    let s = std::fs::read_to_string(&path).ok()?;
    let id = s.trim().to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// 持久化当前活动 Profile 的隔离上下文原生 id（若引擎支持）。
fn persist_context(sdk: &Fastbrowser, config_source: Option<&str>) {
    let Some(path) = context_state_path(config_source) else {
        return;
    };
    if let Some(id) = sdk.active_context_id() {
        let _ = std::fs::write(&path, id);
    }
}

/// `open` 之后有界等待新标签报出真实 URL，并返回刷新后的 `{tab,url,title}`。
///
/// 导航是异步的：`sdk.open` 返回时页面往往仍是 `about:blank`。若把该瞬时值直接
/// 返回给调用方（Agent），会被误判为"打开失败"从而反复重试、开出一堆重复标签页。
fn wait_for_open_url(sdk: &Fastbrowser, initial: Value) -> Value {
    let tab = initial.get("tab").cloned();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Ok(r) = sdk.tool_call("get_current_url", json!({})) {
            if let Some(u) = r.get("url").and_then(Value::as_str) {
                if !u.is_empty() && u != "about:blank" && u != "about:blank#blocked" {
                    let title = sdk
                        .tool_call("get_page_title", json!({}))
                        .ok()
                        .and_then(|t| t.get("title").and_then(Value::as_str).map(String::from))
                        .unwrap_or_default();
                    return json!({"tab": tab, "url": u, "title": title});
                }
            }
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    initial
}

/// REPL：stdin 逐行执行命令，跨命令保持浏览器会话（browser-use 式）。
///
/// ```text
/// $ fastbrowser --repl
/// > open https://example.com
/// > snapshot
/// > click c
/// > extract_links
/// > exit
/// ```
fn run_repl(config: Config) {
    let sdk = Fastbrowser::new();
    if let Err(e) = sdk.init(config) {
        eprintln!("init failed: {e}");
        std::process::exit(1);
    }
    println!("fastbrowser repl — type a command, 'exit' to quit");
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        print!("> ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "exit" || line == "quit" {
            break;
        }
        let parts: Vec<String> = line.split_whitespace().map(String::from).collect();
        if parts.is_empty() {
            continue;
        }
        let cmd = &parts[0];
        let params = &parts[1..];
        match dispatch(&sdk, cmd, params) {
            Ok(v) => println!("{v}"),
            Err(e) => println!("{}", e.to_json()),
        }
    }
    sdk.shutdown();
}

fn parse_global_flags(args: &[String]) -> (Config, Option<String>, Vec<String>) {
    let mut config = Config::default();
    let mut rest = Vec::new();
    let mut config_source: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--engine" => {
                if let Some(e) = args.get(i + 1) {
                    config.engine = e.clone();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--hosted" => {
                config.rendering_mode = RenderingMode::Hosted;
                i += 1;
            }
            "--headless" => {
                config.rendering_mode = RenderingMode::Headless;
                i += 1;
            }
            "--config" => {
                if let Some(c) = args.get(i + 1) {
                    // 接受内联 JSON 或指向 JSON 文件的路径（如 browser.json）。
                    let parsed: Option<Config> =
                        serde_json::from_str::<Config>(c).ok().or_else(|| {
                            std::fs::read_to_string(c)
                                .ok()
                                .and_then(|s| serde_json::from_str::<Config>(&s).ok())
                        });
                    if let Some(cfg) = parsed {
                        config = cfg;
                    }
                    // 记录配置来源路径（用于活跃标签保活）。
                    if std::path::Path::new(c).exists() && std::path::Path::new(c).is_file() {
                        config_source = Some(c.clone());
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            other => {
                rest.push(other.to_string());
                i += 1;
            }
        }
    }
    (config, config_source, rest)
}

fn dispatch(sdk: &Fastbrowser, cmd: &str, args: &[String]) -> fastbrowser::engine::Result<Value> {
    let params = |map: Value| -> Value { map };

    macro_rules! simple {
        ($tool:expr) => {
            sdk.tool_call($tool, json!({}))
        };
    }

    match cmd {
        "open" => {
            let url = need(args, 0, "open <url>")?;
            let v = sdk.open(url)?;
            // 导航是异步的：有界轮询直到新标签报出真实 URL（非 about:blank），
            // 并**返回刷新后的结果**（否则调用方只拿到创建瞬间的 about:blank，
            // 会误以为打开失败而重复 open → 开出一堆重复标签页）。
            Ok(wait_for_open_url(sdk, v))
        }
        "navigate" => {
            let url = need(args, 0, "navigate <url>")?;
            sdk.navigate(url)
        }
        "back" => simple!("back"),
        "forward" => simple!("forward"),
        "reload" => simple!("reload"),
        "stop" => simple!("stop"),

        "snapshot" => sdk
            .snapshot()
            .map(|s| serde_json::to_value(s).unwrap_or(Value::Null)),
        "get_page_title" => simple!("get_page_title"),
        "get_current_url" => simple!("get_current_url"),
        "get_page_text" => simple!("get_page_text"),
        "get_element_info" => {
            let id = need(args, 0, "get_element_info <id>")?;
            sdk.tool_call("get_element_info", params(json!({"id": id})))
        }
        "screenshot" => {
            let shot = sdk.screenshot()?;
            let result = json!({"width": shot.width, "height": shot.height, "format": "rgba", "base64": shot.to_base64()});
            if let Some(path) = args.first() {
                std::fs::write(path, &shot.rgba).map_err(fastbrowser::engine::EngineError::from)?;
                Ok(json!({"saved": path, "width": shot.width, "height": shot.height}))
            } else {
                Ok(result)
            }
        }

        "click" => {
            let id = need(args, 0, "click <id>")?;
            sdk.tool_call("click", params(json!({"id": id})))
        }
        "dblclick" => {
            let id = need(args, 0, "dblclick <id>")?;
            sdk.tool_call("dblclick", params(json!({"id": id})))
        }
        "type" => {
            let id = need(args, 0, "type <id> <text>")?;
            let text = need(args, 1, "type <id> <text>")?;
            sdk.tool_call("type", params(json!({"id": id, "text": text})))
        }
        "press" => {
            let key = need(args, 0, "press <key>")?;
            sdk.tool_call("press", params(json!({"key": key})))
        }
        "hover" => {
            let id = need(args, 0, "hover <id>")?;
            sdk.tool_call("hover", params(json!({"id": id})))
        }
        "scroll" => {
            let dx = args
                .first()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let dy = args
                .get(1)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(100.0);
            sdk.tool_call("scroll", params(json!({"dx": dx, "dy": dy})))
        }
        "swipe" => {
            let fx = need(args, 0, "swipe <fx> <fy> <tx> <ty>")?
                .parse::<f64>()
                .unwrap_or(0.0);
            let fy = need(args, 1, "swipe <fx> <fy> <tx> <ty>")?
                .parse::<f64>()
                .unwrap_or(0.0);
            let tx = need(args, 2, "swipe <fx> <fy> <tx> <ty>")?
                .parse::<f64>()
                .unwrap_or(0.0);
            let ty = need(args, 3, "swipe <fx> <fy> <tx> <ty>")?
                .parse::<f64>()
                .unwrap_or(0.0);
            sdk.tool_call(
                "swipe",
                params(json!({"from_x": fx, "from_y": fy, "to_x": tx, "to_y": ty})),
            )
        }

        "extract_text" => simple!("extract_text"),
        "extract_html" => simple!("extract_html"),
        "extract_links" => simple!("extract_links"),
        "extract_images" => simple!("extract_images"),
        "extract_table" => simple!("extract_table"),
        "extract_json" => {
            let script = need(args, 0, "extract_json <script>")?;
            sdk.tool_call("extract_json", params(json!({"script": script})))
        }

        "wait_for_element" => {
            let sel = need(args, 0, "wait_for_element <selector> [timeout_ms]")?;
            let timeout = args
                .get(1)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5000);
            sdk.tool_call(
                "wait_for_element",
                params(json!({"selector": sel, "timeout_ms": timeout})),
            )
        }
        "wait_for_navigation" => {
            let timeout = args
                .first()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(10000);
            sdk.tool_call(
                "wait_for_navigation",
                params(json!({"timeout_ms": timeout})),
            )
        }
        "wait_for_load_state" => {
            let state = args.first().cloned().unwrap_or_else(|| "load".into());
            let timeout = args
                .get(1)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(10000);
            sdk.tool_call(
                "wait_for_load_state",
                params(json!({"state": state, "timeout_ms": timeout})),
            )
        }
        "wait_for_condition" => {
            let script = need(args, 0, "wait_for_condition <script> [timeout_ms]")?;
            let timeout = args
                .get(1)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5000);
            sdk.tool_call(
                "wait_for_condition",
                params(json!({"script": script, "timeout_ms": timeout})),
            )
        }
        "wait_for_text" => {
            let text = need(args, 0, "wait_for_text <text> [timeout_ms]")?;
            let timeout = args
                .get(1)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5000);
            sdk.tool_call(
                "wait_for_text",
                params(json!({"text": text, "timeout_ms": timeout})),
            )
        }
        "assert_url_contains" => {
            let s = need(args, 0, "assert_url_contains <text>")?;
            sdk.tool_call("assert_url_contains", params(json!({"contains": s})))
        }
        "assert_title" => {
            let s = need(args, 0, "assert_title <text>")?;
            sdk.tool_call("assert_title", params(json!({"contains": s})))
        }
        "get_element_text" | "get_attributes" | "is_visible" | "is_enabled"
        | "screenshot_element" => {
            let id = need(args, 0, "this-command <id>")?;
            sdk.tool_call(cmd, params(json!({"id": id})))
        }
        "get_focused_element"
        | "get_selected_text"
        | "get_page_meta"
        | "get_scroll_position"
        | "get_performance_metrics"
        | "get_accessibility_tree"
        | "clear_cookies"
        | "clear_storage"
        | "storage_get_all"
        | "extract_forms"
        | "duplicate_tab"
        | "close_other_tabs"
        | "blur" => sdk.tool_call(cmd, params(json!({}))),
        "set_scroll_position" => {
            let x = args
                .first()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            let y = args
                .get(1)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            sdk.tool_call(cmd, params(json!({"x": x, "y": y})))
        }
        "right_click" | "focus" | "clear_input" => {
            let id = need(args, 0, "this-command <id>")?;
            sdk.tool_call(cmd, params(json!({"id": id})))
        }
        "get_tab" => {
            let title = args.first().cloned();
            sdk.tool_call("get_tab", params(json!({"title_contains": title})))
        }
        "session_save" => {
            let p = need(args, 0, "session_save <path>")?;
            sdk.session_save(p)
        }
        "session_load" => {
            let p = need(args, 0, "session_load <path>")?;
            sdk.session_load(p)
        }
        "clear_state" => sdk.clear_state(),

        "fill_form" => {
            let v = need(args, 0, "fill_form <json>")?;
            let parsed: Value =
                serde_json::from_str(v).unwrap_or(Value::Object(Default::default()));
            // 兼容两种形态：`{"values": {...}}` 或直接 `{...}`（values 本体）
            let params = if parsed.get("values").is_some() {
                parsed
            } else {
                json!({"values": parsed})
            };
            sdk.tool_call("fill_form", params)
        }
        "select_option" => {
            let id = need(args, 0, "select_option <id> <value>")?;
            let value = need(args, 1, "select_option <id> <value>")?;
            sdk.tool_call("select_option", params(json!({"id": id, "value": value})))
        }
        "checkbox" => {
            let id = need(args, 0, "checkbox <id> [true|false]")?;
            let checked = args.get(1).map(|s| s == "true").unwrap_or(true);
            sdk.tool_call("checkbox", params(json!({"id": id, "checked": checked})))
        }
        "radio" => {
            let id = need(args, 0, "radio <id>")?;
            sdk.tool_call("radio", params(json!({"id": id})))
        }

        "cookie_get" => {
            let domain = args.first().cloned();
            sdk.tool_call("cookie_get", params(json!({"domain": domain})))
        }
        "cookie_set" => {
            let v = need(args, 0, "cookie_set <json>")?;
            let c: Value = serde_json::from_str(v).unwrap_or(Value::Object(Default::default()));
            sdk.tool_call("cookie_set", c)
        }
        "cookie_clear" => {
            let domain = args.first().cloned();
            sdk.tool_call("cookie_clear", params(json!({"domain": domain})))
        }
        "storage_get" => {
            let key = need(args, 0, "storage_get <key>")?;
            sdk.tool_call("storage_get", params(json!({"key": key})))
        }
        "storage_set" => {
            let key = need(args, 0, "storage_set <key> <value>")?;
            let value = need(args, 1, "storage_set <key> <value>")?;
            sdk.tool_call("storage_set", params(json!({"key": key, "value": value})))
        }

        "execute_js" => {
            let script = need(args, 0, "execute_js <script>")?;
            sdk.tool_call("execute_js", params(json!({"script": script})))
        }
        "evaluate_xpath" => {
            let expr = need(args, 0, "evaluate_xpath <expr>")?;
            sdk.tool_call("evaluate_xpath", params(json!({"expr": expr})))
        }
        "inject_css" => {
            let css = need(args, 0, "inject_css <css>")?;
            sdk.tool_call("inject_css", params(json!({"css": css})))
        }
        "block_request" => {
            let (patterns, enabled) = patterns(args);
            sdk.tool_call(
                "block_request",
                params(json!({"patterns": patterns, "enabled": enabled})),
            )
        }
        "intercept_request" => {
            let (patterns, enabled) = patterns(args);
            sdk.tool_call(
                "intercept_request",
                params(json!({"patterns": patterns, "enabled": enabled})),
            )
        }

        "new_tab" => {
            let url = need(args, 0, "new_tab <url>")?;
            sdk.tool_call("new_tab", params(json!({"url": url})))
        }
        "close_tab" => {
            let tab = args.first().and_then(|s| s.parse::<u32>().ok());
            sdk.tool_call("close_tab", params(json!({"tab": tab})))
        }
        "switch_tab" => {
            let tab = need(args, 0, "switch_tab <tab>")?;
            sdk.tool_call(
                "switch_tab",
                params(json!({"tab": tab.parse::<u32>().unwrap_or(0)})),
            )
        }
        "list_tabs" => simple!("list_tabs"),
        "get_active_tab" => simple!("get_active_tab"),

        "done" => {
            let answer = need(args, 0, "done <answer>")?;
            sdk.tool_call("done", params(json!({"answer": answer})))
        }
        "send_keys" => {
            let keys = need(args, 0, "send_keys <keys>")?;
            sdk.tool_call("send_keys", params(json!({"keys": keys})))
        }
        "get_history" => simple!("get_history"),
        "pending_dialog" => simple!("pending_dialog"),
        "dialog_accept" => {
            let text = args.first().cloned();
            sdk.tool_call("dialog_accept", params(json!({"prompt_text": text})))
        }
        "dialog_dismiss" => simple!("dialog_dismiss"),
        "list_pending_requests" => simple!("list_pending_requests"),
        "continue_request" => {
            let id = need(args, 0, "continue_request <request_id>")?;
            sdk.tool_call("continue_request", params(json!({"request_id": id})))
        }
        "abort_request" => {
            let id = need(args, 0, "abort_request <request_id>")?;
            sdk.tool_call("abort_request", params(json!({"request_id": id})))
        }
        "fulfill_request" => {
            let id = need(args, 0, "fulfill_request <request_id> [status] [body_b64]")?;
            let status = args
                .get(1)
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(200);
            let body = args.get(2).cloned();
            sdk.tool_call(
                "fulfill_request",
                params(json!({"request_id": id, "status": status, "body_b64": body})),
            )
        }
        "save_as_pdf" => {
            let path = need(args, 0, "save_as_pdf <path>")?;
            sdk.tool_call("save_as_pdf", params(json!({"path": path})))
        }
        "search" => {
            let q = need(args, 0, "search <query> [limit]")?;
            let limit = args
                .get(1)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(20);
            sdk.tool_call("search", params(json!({"query": q, "limit": limit})))
        }
        "find_elements" => {
            let sel = need(args, 0, "find_elements <selector> [limit]")?;
            let limit = args
                .get(1)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(20);
            sdk.tool_call(
                "find_elements",
                params(json!({"selector": sel, "limit": limit})),
            )
        }

        "tools" => Ok(sdk.tool_list()),
        "status" => Ok(sdk.status()),
        "info" => Ok(sdk.get_info().to_json()),
        "audit" => Ok(sdk.audit()),
        "clear_audit" => {
            sdk.clear_audit();
            Ok(json!({"cleared": true}))
        }
        _ => Err(fastbrowser::engine::EngineError::new(
            fastbrowser::engine::ErrorKind::InvalidArgument,
            format!("unknown command '{cmd}'"),
        )),
    }
}

fn patterns(args: &[String]) -> (Vec<String>, bool) {
    let mut pats = Vec::new();
    let mut enabled = true;
    for a in args {
        if a == "off" {
            enabled = false;
        } else {
            pats.push(a.clone());
        }
    }
    (pats, enabled)
}

fn need<'a>(args: &'a [String], idx: usize, usage: &str) -> fastbrowser::engine::Result<&'a str> {
    args.get(idx)
        .map(String::as_str)
        .ok_or_else(|| fastbrowser::engine::EngineError::invalid(format!("usage: {usage}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_manifest_target_preserves_other_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("browser.json");
        std::fs::write(
            &path,
            r#"{"engine":"chromium","cdp_url":"ws://x","profile_id":"p","cdp_target_id":"OLD"}"#,
        )
        .unwrap();

        update_manifest_target(&path, "NEW").unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["cdp_target_id"], "NEW");
        assert_eq!(v["engine"], "chromium");
        assert_eq!(v["cdp_url"], "ws://x");
        assert_eq!(v["profile_id"], "p");
    }

    #[test]
    fn update_manifest_target_is_noop_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("browser.json");
        let original = r#"{"cdp_target_id":"SAME","engine":"chromium"}"#;
        std::fs::write(&path, original).unwrap();

        update_manifest_target(&path, "SAME").unwrap();

        // 未变化时不重写（内容逐字一致）。
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn context_state_path_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("browser.json");
        std::fs::write(&cfg, "{}").unwrap();
        let cfg = cfg.to_str().unwrap();

        assert!(read_persisted_context(Some(cfg)).is_none());
        std::fs::write(dir.path().join("browser.ctx"), " CTX123 \n").unwrap();
        assert_eq!(read_persisted_context(Some(cfg)).as_deref(), Some("CTX123"));
        // 无配置文件时无状态路径。
        assert!(context_state_path(None).is_none());
    }
}
