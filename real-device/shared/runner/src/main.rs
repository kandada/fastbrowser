//! fastbrowser 真机测试参考 runner。
//!
//! 加载 `cases/*.json`，按引擎执行同一套用例（mock / chromium / webview）。
//! 桌面端先跑通证明用例定义正确；Android/iOS 宿主 runner 消费同一份 JSON。
//!
//! 用法见 `../README.md`。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use fastbrowser::engine::{PageEvent, Result as FbResult, TabId};
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde::Deserialize;
use serde_json::{json, Value};

/// chromium 引擎的 CDP 端点（reset 后重建内核时保留）。
static CDP_URL: OnceLock<String> = OnceLock::new();

// ── 用例模型 ──────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct Case {
    id: String,
    category: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_engines")]
    engines: Vec<String>,
    steps: Vec<Value>,
}

fn default_engines() -> Vec<String> {
    vec!["mock".into(), "chromium".into(), "webview".into()]
}

// ── 运行上下文 ─────────────────────────────────────────────────────

#[derive(Clone)]
struct Vars(HashMap<String, Value>);
impl Vars {
    fn new() -> Self {
        Vars(HashMap::new())
    }
    fn set(&mut self, k: &str, v: Value) {
        self.0.insert(k.to_string(), v);
    }
    fn get(&self, k: &str) -> Option<&Value> {
        self.0.get(k)
    }
    fn track_tab(&mut self, tab: u32) {
        self.set("current_tab", json!(tab));
        let idx = (1..=99).find(|i| !self.0.contains_key(&format!("tab{i}"))).unwrap_or(1);
        self.set(&format!("tab{idx}"), json!(tab));
    }
}

#[derive(serde::Serialize)]
struct StepReport {
    action: String,
    ok: bool,
    detail: String,
    duration_ms: u64,
}

#[derive(serde::Serialize)]
struct CaseReport {
    id: String,
    category: String,
    status: &'static str, // pass / fail / skip
    reason: String,
    steps: Vec<StepReport>,
    duration_ms: u64,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut cases_dir = "cases".to_string();
    let mut engine = "mock".to_string();
    let mut category: Option<String> = None;
    let mut cdp_url: Option<String> = None;
    let mut base_override: Option<String> = None;
    let mut report_path: Option<String> = None;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--cases" => cases_dir = args.next().expect("--cases <dir>"),
            "--engine" => engine = args.next().expect("--engine <mock|chromium|webview>"),
            "--category" => category = Some(args.next().expect("--category <name>")),
            "--cdp-url" => cdp_url = Some(args.next().expect("--cdp-url <ws>")),
            "--base" => base_override = Some(args.next().expect("--base <url>")),
            "--report" => report_path = Some(args.next().expect("--report <path>")),
            "--help" | "-h" => {
                println!(
                    "fb-realdevice-runner\n  --cases <dir>  --engine <mock|chromium|webview>  --category <name>\n  --cdp-url <ws>  --base <url>  --report <path>"
                );
                return;
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }

    if !matches!(engine.as_str(), "mock" | "chromium" | "webview") {
        eprintln!("unsupported engine '{engine}' (mock|chromium|webview)");
        std::process::exit(2);
    }
    if engine == "webview" {
        // 桌面参考 runner 无法提供 WebViewOps；webview 由真机宿主 runner 执行
        eprintln!("the webview engine needs a real-device host runner (Android/iOS); this reference runner does not support it — use mock/chromium.");
        std::process::exit(2);
    }

    // 收集用例
    let mut all: Vec<Case> = Vec::new();
    for entry in std::fs::read_dir(&cases_dir).expect("cases dir") {
        let p = entry.unwrap().path();
        if p.extension().map(|e| e == "json").unwrap_or(false) {
            let text = std::fs::read_to_string(&p).unwrap();
            let file: Value = serde_json::from_str(&text).expect(&format!("bad json {}", p.display()));
            let cases: Vec<Case> = serde_json::from_value(file["cases"].clone())
                .expect(&format!("bad case shape in {}", p.display()));
            all.extend(cases);
        }
    }
    all.sort_by(|a, b| a.id.cmp(&b.id));
    println!("loaded {} case(s) from {cases_dir}", all.len());

    // 引擎与基准 URL
    let mut chrome_guard: Option<ChromeGuard> = None;
    let base = if let Some(b) = base_override {
        b
    } else if engine == "mock" {
        "https://example.com".to_string()
    } else {
        // chromium：起 fixture 服务器（或复用 --cdp-url）
        let (listener, base) = serve_fixtures();
        let _keep = listener;
        base
    };
    let base_host = base.replace("https://", "").replace("http://", "").split(':').next().unwrap_or("localhost").to_string();

    // 初始化内核
    let sdk = Fastbrowser::new();
    let mut cfg = Config {
        engine: engine.clone(),
        ..Config::default()
    };
    if engine == "chromium" {
        let ws = match cdp_url {
            Some(u) => u,
            None => {
                let (g, u) = launch_chrome_pair();
                chrome_guard = Some(g);
                u
            }
        };
        let _ = CDP_URL.set(ws.clone());
        cfg.cdp_url = Some(ws);
    }
    let init = sdk.init(cfg);
    if let Err(e) = init {
        eprintln!("init failed ({engine}): {e}");
        std::process::exit(1);
    }

    // 会话临时文件
    let tmp = std::env::temp_dir().join(format!("fb-rd-state-{}.json", std::process::id()));
    let session_path = tmp.to_str().unwrap().to_string();

    let mut seed_vars = Vars::new();
    seed_vars.set("base_host", json!(base_host));
    seed_vars.set("session_path", json!(session_path));

    // 执行
    let mut reports: Vec<CaseReport> = Vec::new();
    for case in &all {
        if let Some(cat) = &category {
            if case.category != *cat {
                continue;
            }
        }
        if !case.engines.iter().any(|e| e == &engine) {
            reports.push(CaseReport {
                id: case.id.clone(),
                category: case.category.clone(),
                status: "skip",
                reason: format!("not for engine {engine}"),
                steps: vec![],
                duration_ms: 0,
            });
            continue;
        }
        // 每个用例独立的 vars（tab1/current_tab 等用例内有效，避免跨用例污染）
        let mut vars = seed_vars.clone();
        let report = run_case(&sdk, case, &base, &mut vars);
        let status = if report.steps.iter().all(|s| s.ok) { "pass" } else { "fail" };
        reports.push(CaseReport {
            id: case.id.clone(),
            category: case.category.clone(),
            status,
            reason: "".into(),
            steps: report.steps,
            duration_ms: report.duration_ms,
        });
    }

    // 汇总
    let mut pass = 0;
    let mut fail = 0;
    let mut skip = 0;
    for r in &reports {
        match r.status {
            "pass" => pass += 1,
            "fail" => fail += 1,
            _ => skip += 1,
        }
        println!(
            "  [{}] {:<38} {}",
            r.status.to_uppercase(),
            r.id,
            if r.status == "fail" {
                r.steps.iter().find(|s| !s.ok).map(|s| s.detail.clone()).unwrap_or_default()
            } else {
                String::new()
            }
        );
    }
    println!("== {engine}: pass={pass} fail={fail} skip={skip} ==");

    // 报告
    let out = report_path.unwrap_or_else(|| {
        let dir = std::env::var("REAL_DEVICE_REPORTS").unwrap_or_else(|_| ".".into());
        format!("{dir}/runner-{engine}.json")
    });
    if let Some(parent) = Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let report_json = json!({
        "engine": engine,
        "base": base,
        "timestamp_ms": now_ms(),
        "cases": reports,
    });
    std::fs::write(&out, serde_json::to_string_pretty(&report_json).unwrap()).unwrap();
    println!("report → {out}");

    let _ = std::fs::remove_file(&session_path);
    if fail > 0 {
        std::process::exit(1);
    }
}

// ── 单用例执行 ────────────────────────────────────────────────────

struct RunOutcome {
    steps: Vec<StepReport>,
    duration_ms: u64,
}

fn run_case(sdk: &Fastbrowser, case: &Case, base: &str, vars: &mut Vars) -> RunOutcome {
    let start = Instant::now();
    let mut steps = Vec::new();
    for step in &case.steps {
        let action = step.get("action").and_then(Value::as_str).unwrap_or("").to_string();
        let s = Instant::now();
        let outcome = run_step(sdk, case, step, base, vars);
        let duration_ms = s.elapsed().as_millis() as u64;
        let (ok, detail) = match outcome {
            Ok(v) => (true, format!("ok: {v}")),
            Err(e) => (false, e),
        };
        steps.push(StepReport { action, ok, detail, duration_ms });
        // 失败即中断该用例
        if !ok {
            break;
        }
    }
    RunOutcome { steps, duration_ms: start.elapsed().as_millis() as u64 }
}

type StepResult = std::result::Result<Value, String>;

fn run_step(sdk: &Fastbrowser, case: &Case, step: &Value, base: &str, vars: &mut Vars) -> StepResult {
    let action = step.get("action").and_then(Value::as_str).unwrap_or("");
    match action {
        "open" => {
            let url = param_url(step, "url", base, vars);
            let out = fb(sdk.open(&url))?;
            if let Some(t) = out.get("tab").and_then(Value::as_u64) {
                vars.track_tab(t as u32);
            }
            Ok(out)
        }
        "navigate" => {
            let url = param_url(step, "url", base, vars);
            Ok(fb(sdk.navigate(&url))?)
        }
        "tool" => {
            let name = step.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let params = templated(step.get("params").cloned().unwrap_or(json!({})), base, vars);
            let expect_error = step.get("expect_error");
            match sdk.tool_call(&name, params.clone()) {
                Ok(result) => {
                    // 跟踪工具返回的 tab（new_tab / open 等）
                    if let Some(t) = result.get("tab").and_then(Value::as_u64) {
                        vars.track_tab(t as u32);
                    }
                    if let Some(ee) = expect_error {
                        return Err(format!("expected error but got ok: {result} (expect_error={ee})"));
                    }
                    if let Some(expect) = step.get("expect") {
                        check_expect(&result, expect)?;
                    }
                    Ok(result)
                }
                Err(e) => {
                    if let Some(ee) = expect_error {
                        let msg = e.to_string();
                        if let Some(c) = ee.get("contains").and_then(Value::as_str) {
                            if msg.contains(c) {
                                return Ok(json!({"error": msg}));
                            }
                            return Err(format!("expected error contains '{c}' but got: {msg}"));
                        }
                        return Ok(json!({"error": msg}));
                    }
                    Err(e.to_string())
                }
            }
        }
        "find" => {
            let var = step.get("var").and_then(Value::as_str).unwrap_or("").to_string();
            let m = step.get("match").cloned().unwrap_or(json!({}));
            // 支持显式 tab（否则用 current_tab）
            let tab = if let Some(t) = step.get("tab") {
                let v = templated(t.clone(), base, vars);
                TabId(v.as_u64().unwrap_or(1) as u32)
            } else {
                current_tab(vars)
            };
            let snap = fb(sdk.runtime())?.as_ref().ok_or("not initialized")?.engine().snapshot(tab).map_err(|e| e.to_string())?;
            let el = snap.interactive.iter().find(|e| match_el(e, &m)).ok_or_else(|| {
                format!("find {}: no element matching {m}", snap.url)
            })?;
            vars.set(&var, json!(el.id.to_string()));
            Ok(json!({"id": el.id.to_string()}))
        }
        "wait_load" => {
            let state = step.get("state").and_then(Value::as_str).unwrap_or("load").to_string();
            let timeout = step.get("timeout_ms").and_then(Value::as_u64).unwrap_or(10000);
            let tab = current_tab(vars);
            wait_until(timeout, &format!("wait_load({state})"), || {
                let ready = sdk
                    .runtime()
                    .ok()
                    .and_then(|rt| rt.as_ref().map(|r| r.engine().evaluate(tab, "document.readyState||''").ok()))
                    .flatten()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                match state.as_str() {
                    "domcontentloaded" => ready == "interactive" || ready == "complete",
                    _ => ready == "complete",
                }
            }).map_err(|e| e.to_string())?;
            Ok(json!({"ready": true}))
        }
        "wait_navigation" => {
            let timeout = step.get("timeout_ms").and_then(Value::as_u64).unwrap_or(10000);
            let tab = current_tab(vars);
            wait_until(timeout, "wait_navigation", || {
                sdk.runtime()
                    .ok()
                    .and_then(|rt| rt.as_ref().map(|r| r.engine().drain_events(tab)))
                    .unwrap_or_default()
                    .iter()
                    .any(|e| matches!(e, PageEvent::NavigationCompleted { .. } | PageEvent::Loaded { .. }))
            }).map_err(|e| e.to_string())?;
            Ok(json!({"navigated": true}))
        }
        "sleep" => {
            let ms = step.get("ms").and_then(Value::as_u64).unwrap_or(100);
            std::thread::sleep(Duration::from_millis(ms));
            Ok(json!({"slept": ms}))
        }
        "session_save" => {
            let path = param_string(step.get("path"), vars);
            Ok(fb(sdk.session_save(&path))?)
        }
        "session_load" => {
            let path = param_string(step.get("path"), vars);
            Ok(fb(sdk.session_load(&path))?)
        }
        "clear_state" => {
            Ok(fb(sdk.clear_state())?)
        }
        "reset" => {
            let engine = sdk.status()["engine"].as_str().unwrap_or("mock").to_string();
            sdk.shutdown();
            let mut cfg = Config { engine, ..Config::default() };
            if let Some(u) = CDP_URL.get() {
                cfg.cdp_url = Some(u.clone());
            }
            fb(sdk.init(cfg))?;
            Ok(json!({"reset": true}))
        }
        "benchmark" => {
            let tool = step.get("tool").and_then(Value::as_str).unwrap_or("").to_string();
            let iterations = step.get("iterations").and_then(Value::as_u64).unwrap_or(5);
            let expect_avg_ms = step.get("expect_avg_ms").and_then(Value::as_f64).unwrap_or(5000.0);
            let params = templated(step.get("params").cloned().unwrap_or(json!({})), base, vars);
            let mut durations = Vec::new();
            for _ in 0..iterations {
                let t = Instant::now();
                // snapshot 是 SDK 方法（非工具）；其余走 tool_call
                if tool == "snapshot" {
                    fb(sdk.snapshot())?;
                } else {
                    fb(sdk.tool_call(&tool, params.clone()))?;
                }
                durations.push(t.elapsed().as_millis() as f64);
            }
            let avg = durations.iter().sum::<f64>() / (durations.len() as f64).max(1.0);
            if avg > expect_avg_ms {
                return Err(format!("benchmark {tool}: avg {avg:.0}ms > limit {expect_avg_ms:.0}ms"));
            }
            Ok(json!({"tool": tool, "iterations": iterations, "avg_ms": avg, "durations_ms": durations}))
        }
        other => Err(format!("unknown action '{other}' (case {})", case.id)),
    }
}

fn current_tab(vars: &Vars) -> TabId {
    TabId(vars.get("current_tab").and_then(Value::as_u64).unwrap_or(1) as u32)
}

fn param_url(step: &Value, key: &str, base: &str, vars: &mut Vars) -> String {
    let raw = step.get("params").and_then(|p| p.get(key)).cloned().unwrap_or(json!(""));
    let s = match &raw {
        Value::String(s) => s.clone(),
        Value::Object(o) if o.contains_key("$var") => vars
            .get(o["$var"].as_str().unwrap_or(""))
            .cloned()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        _ => String::new(),
    };
    s.replace("{{BASE}}", base)
}

fn param_string(v: Option<&Value>, vars: &Vars) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(o)) if o.contains_key("$var") => vars
            .get(o["$var"].as_str().unwrap_or(""))
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// 递归替换 {{BASE}} 与 {"$var": ...}。
fn templated(v: Value, base: &str, vars: &Vars) -> Value {
    match v {
        Value::String(s) => {
            if let Some(rest) = s.strip_prefix("{{BASE}}") {
                return Value::String(format!("{base}{rest}"));
            }
            Value::String(s)
        }
        Value::Object(mut o) => {
            if let Some(var) = o.get("$var").and_then(Value::as_str) {
                if let Some(val) = vars.get(var) {
                    return val.clone();
                }
            }
            for (_k, val) in o.iter_mut() {
                *val = templated(val.clone(), base, vars);
            }
            Value::Object(o)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(|x| templated(x, base, vars)).collect()),
        other => other,
    }
}

fn match_el(e: &fastbrowser::engine::InteractiveElement, m: &Value) -> bool {
    let tag_ok = m.get("tag").map(|t| e.tag == t.as_str().unwrap_or("")).unwrap_or(true);
    let name_ok = m
        .get("name")
        .map(|n| e.attrs.get("name").map(|a| a == n.as_str().unwrap_or("")).unwrap_or(false))
        .unwrap_or(true);
    let text_ok = m
        .get("text")
        .map(|t| e.text.as_deref().map(|x| x.contains(t.as_str().unwrap_or(""))).unwrap_or(false))
        .unwrap_or(true);
    let ph_ok = m
        .get("placeholder")
        .map(|p| e.attrs.get("placeholder").map(|a| a == p.as_str().unwrap_or("")).unwrap_or(false))
        .unwrap_or(true);
    tag_ok && name_ok && text_ok && ph_ok
}

fn check_expect(result: &Value, expect: &Value) -> StepResult {
    let obj = expect.as_object().ok_or("expect must be object")?;
    for (path, check) in obj {
        let val = resolve_path(result, path);
        if let Some(check_obj) = check.as_object() {
            if let Some(c) = check_obj.get("contains").and_then(Value::as_str) {
                let s = val.and_then(Value::as_str).ok_or_else(|| format!("{path} not a string: {result}"))?;
                if !s.contains(c) {
                    return Err(format!("expect {path}.contains('{c}') failed, got '{s}'"));
                }
            } else if check_obj.get("is_number").and_then(Value::as_bool).unwrap_or(false) {
                if !val.map(Value::is_number).unwrap_or(false) {
                    return Err(format!("expect {path} is_number failed: {result}"));
                }
            } else if check_obj.get("is_array").and_then(Value::as_bool).unwrap_or(false) {
                if !val.map(Value::is_array).unwrap_or(false) {
                    return Err(format!("expect {path} is_array failed: {result}"));
                }
            } else if check_obj.get("is_object").and_then(Value::as_bool).unwrap_or(false) {
                if !val.map(Value::is_object).unwrap_or(false) {
                    return Err(format!("expect {path} is_object failed: {result}"));
                }
            } else if check_obj.get("is_null").and_then(Value::as_bool).unwrap_or(false) {
                if val.map(|v| !v.is_null()).unwrap_or(false) {
                    return Err(format!("expect {path} is_null failed: {result}"));
                }
            } else if let Some(g) = check_obj.get("gte").and_then(Value::as_f64) {
                let got = val.and_then(Value::as_f64).ok_or_else(|| format!("{path} not number: {result}"))?;
                if got < g {
                    return Err(format!("expect {path} >= {g} failed, got {got}"));
                }
            } else if let Some(l) = check_obj.get("len_gte").and_then(Value::as_f64) {
                let len = val.and_then(Value::as_array).map(|a| a.len() as f64).ok_or_else(|| format!("{path} not array: {result}"))?;
                if len < l {
                    return Err(format!("expect {path}.len >= {l} failed, got {len}"));
                }
            } else {
                let want = Value::Object(check_obj.clone());
                let got = val.cloned().unwrap_or(Value::Null);
                if got != want {
                    return Err(format!("expect {path} == {want} failed, got {got}"));
                }
            }
        } else {
            let got = val.cloned().unwrap_or(Value::Null);
            if got != *check {
                return Err(format!("expect {path} == {check} failed, got {got}"));
            }
        }
    }
    Ok(Value::Null)
}

fn resolve_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

fn wait_until<F: FnMut() -> bool>(timeout_ms: u64, what: &str, mut f: F) -> FbResult<()> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if f() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(fastbrowser::engine::EngineError::new(
        fastbrowser::engine::ErrorKind::Timeout,
        format!("{what} timed out after {timeout_ms}ms"),
    ))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把 `Result<T, EngineError>` 转成 `Result<T, String>`（步骤错误统一为字符串）。
fn fb<T>(r: FbResult<T>) -> std::result::Result<T, String> {
    r.map_err(|e| e.to_string())
}

// ── fixture 服务器（chromium/webview 用）──────────────────────────

fn serve_fixtures() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let req = String::from_utf8_lossy(&buf);
            let raw_path = req.split_whitespace().nth(1).unwrap_or("/");
            let path = raw_path.split('?').next().unwrap_or(raw_path);
            let (status, body) = match path {
                "/" => ("200 OK", HOME),
                "/login" => ("200 OK", LOGIN),
                "/search" => ("200 OK", SEARCH),
                "/two" => ("200 OK", "<html><head><title>Two</title></head><body>two</body></html>"),
                "/three" => ("200 OK", "<html><head><title>Three</title></head><body>three</body></html>"),
                "/dialog" => ("200 OK", DIALOG),
                "/register" => ("200 OK", "<html><head><title>Register</title></head><body>register</body></html>"),
                _ => ("404 Not Found", "<html><body>not found</body></html>"),
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (listener, format!("http://localhost:{port}"))
}

const HOME: &str = r##"<!doctype html><html><head><title>FastBrowser Home</title></head><body>
  <h1>Welcome to FastBrowser</h1>
  <p>This is the fixture home page.</p>
  <a href="/register">Register</a>
  <button>Click me</button>
  <input name="q" placeholder="Search">
  <table><tr><th>Name</th><th>Value</th></tr><tr><td>alpha</td><td>1</td></tr></table>
  <script>document.body.dataset.home='1';</script>
</body></html>"##;

const LOGIN: &str = r##"<!doctype html><html><head><title>Login</title></head><body>
  <h1>Sign in</h1>
  <form action="/login" method="post">
    <input name="username" placeholder="Username">
    <input name="password" type="password" placeholder="Password">
    <button type="submit">Submit</button>
  </form>
  <a href="/register">Register</a>
</body></html>"##;

const SEARCH: &str = r##"<!doctype html><html><head><title>Search</title></head><body>
  <input name="q" placeholder="Query">
  <button>Search</button>
  <a href="/r1">Result 1</a><a href="/r2">Result 2</a>
</body></html>"##;

const DIALOG: &str = r##"<!doctype html><html><head><title>Dialog</title></head><body>
  <p>dialog fixture</p>
  <script>setTimeout(function(){ alert('hello dialog'); }, 300);</script>
</body></html>"##;

// ── Chrome 启动（chromium 引擎自动拉起无头 Chrome）─────────────────

struct ChromeGuard {
    child: Child,
    _dir: tempfile::TempDir,
}

impl Drop for ChromeGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn find_chrome() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("CHROME_PATH") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    for p in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
    ] {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    None
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

fn launch_chrome_pair() -> (ChromeGuard, String) {
    let chrome = find_chrome().expect("no chrome; set CHROME_PATH or pass --cdp-url");
    let port = free_port();
    let dir = tempfile::tempdir().unwrap();
    let user_data = dir.path().join("fb-rd-chrome");
    let _ = std::fs::create_dir_all(&user_data);
    let child = Command::new(chrome)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", user_data.display()))
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--disable-extensions")
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn chrome");
    let ws = browser_ws_url(port);
    (ChromeGuard { child, _dir: dir }, ws)
}

fn browser_ws_url(port: u16) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(mut conn) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
            // Host 头必须带端口，否则 Chrome 按端口 80 构造 ws URL（导致无端口、连接被拒）
            let _ = conn.write_all(format!("GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes());
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = conn.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let text = String::from_utf8_lossy(&buf);
            if let Some(body) = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()) {
                if let Ok(v) = serde_json::from_str::<Value>(&body) {
                    if let Some(ws) = v["webSocketDebuggerUrl"].as_str() {
                        // 归一化 host 为 127.0.0.1，避免 macOS 上 localhost→::1 导致连接被拒
                        let fixed = ws.replace("://localhost:", "://127.0.0.1:").to_string();
                        eprintln!("[runner] discovered ws = {ws} → {fixed}");
                        return fixed;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("chrome debug endpoint not ready");
}
