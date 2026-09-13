// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 边缘情况与主流程测试（SDK / 工具 / 并发 / 压力）。

use std::sync::Mutex;

use fastbrowser::engine::{EngineError, RenderingMode};
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;
use serde_json::json;

fn sdk() -> Fastbrowser {
    let s = Fastbrowser::new();
    s.init(Config::default()).unwrap();
    s
}

// ── 生命周期边缘 ─────────────────────────────────────────────

#[test]
fn init_before_any_call() {
    let s = Fastbrowser::new();
    assert!(s.tool_call("get_page_title", json!({})).is_err());
    assert!(!s.is_initialized());
}

#[test]
fn tool_call_auto_creates_tab() {
    // 无显式 open 时，工具调用会自动建空白标签页
    let s = sdk();
    let out = s.tool_call("get_current_url", json!({})).unwrap();
    assert!(out["url"].is_string());
    assert_eq!(
        s.tool_call("list_tabs", json!({})).unwrap()["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn reinit_replaces_engine() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    s.init(Config::for_engine("mock")).unwrap();
    // 重新 init 后旧标签不在了，工具调用应新建
    let out = s.tool_call("list_tabs", json!({})).unwrap();
    assert_eq!(out["tabs"].as_array().unwrap().len(), 0);
}

#[test]
fn shutdown_then_use_errors() {
    let s = sdk();
    s.shutdown();
    assert!(s.tool_call("get_page_title", json!({})).is_err());
    assert!(s.navigate("https://example.com").is_err());
    assert!(s.session_save("/tmp/x.json").is_err());
}

#[test]
fn open_with_edge_urls() {
    let s = sdk();
    // 空串 / 畸形 URL：mock 引擎不崩溃
    let _ = s.open("").unwrap();
    let _ = s.open("not a url").unwrap();
    let _ = s.navigate("https://").unwrap();
    let st = s.status();
    assert!(st["tabs"].as_u64().unwrap() >= 1);
}

#[test]
fn viewport_edge_values() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    // 0 尺寸不 panic
    s.set_viewport(0, 0).unwrap();
    let img = s.screenshot().unwrap();
    assert_eq!(img.width, 0);
    // 超大尺寸不 panic
    s.set_viewport(10_000, 10_000).unwrap();
    let _ = s.screenshot().unwrap();
}

// ── 工具参数边缘 ─────────────────────────────────────────────

#[test]
fn tool_missing_required_params_error() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    assert!(s.tool_call("navigate", json!({})).is_err());
    assert!(s.tool_call("type", json!({"id": "e"})).is_err());
    assert!(s.tool_call("execute_js", json!({})).is_err());
    assert!(s.tool_call("cookie_set", json!({})).is_err());
    assert!(s.tool_call("select_option", json!({"id": "e"})).is_err());
    assert!(s.tool_call("wait_for_condition", json!({})).is_err());
}

#[test]
fn tool_bad_element_refs_error() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    // 未知快照编号
    let r = s.tool_call("click", json!({"id": "zz"}));
    assert!(r.is_err());
    let r = s.tool_call("get_element_text", json!({"id": "zz"}));
    assert!(r.is_err());
    // 缺 id/ref
    let r = s.tool_call("click", json!({}));
    assert!(r.is_err());
    // 非法 ref.kind
    let r = s.tool_call("click", json!({"ref": {"kind": "nope", "value": "x"}}));
    assert!(r.is_err());
}

#[test]
fn select_invalid_option_and_fill_errors() {
    let s = sdk();
    s.open("https://example.com/login").unwrap();
    assert!(s
        .tool_call("select_option", json!({"id": "e", "value": "nope"}))
        .is_err());
    // fill_form 对不存在的 id 不 panic（引擎 set 时按 snapshot 引用解析，未知则报错）
    let r = s.tool_call("fill_form", json!({"values": {"zz": "x"}}));
    assert!(r.is_err());
}

#[test]
fn extract_and_js_errors() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    // JS 语法错误 → 报错（mock MiniJs）
    let r = s.tool_call("execute_js", json!({"script": "1 +"}));
    assert!(r.is_err());
    let r = s.tool_call("extract_json", json!({"script": "unknown()"}));
    assert!(r.is_err());
}

#[test]
fn wait_timeouts_error() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    let r = s.tool_call(
        "wait_for_element",
        json!({"selector": "missing-xyz", "timeout_ms": 30}),
    );
    assert!(r.is_err());
    let r = s.tool_call(
        "wait_for_condition",
        json!({"script": "false", "timeout_ms": 30}),
    );
    assert!(r.is_err());
    // mock 中 load 状态立即就绪（不报错），此处验证其快速成功
    let r = s
        .tool_call(
            "wait_for_load_state",
            json!({"state": "load", "timeout_ms": 2000}),
        )
        .unwrap();
    assert_eq!(r["ready"], "complete");
}

#[test]
fn screenshot_element_out_of_view() {
    // 元素在视口外（rect 越界）不应 panic / 下溢
    let s = sdk();
    s.set_viewport(50, 50).unwrap();
    s.open("https://example.com").unwrap();
    // mock 元素 rect 可能超出小视口，但不会 panic
    let r = s.tool_call("screenshot_element", json!({"id": "a"}));
    assert!(r.is_ok() || r.is_err());
}

#[test]
fn assert_failures_return_errors() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    assert!(s
        .tool_call("assert_text_contains", json!({"text": "zzz-not-there"}))
        .is_err());
    assert!(s
        .tool_call("assert_element_exists", json!({"selector": "missing"}))
        .is_err());
    assert!(s
        .tool_call("assert_url_contains", json!({"contains": "zzz"}))
        .is_err());
    assert!(s
        .tool_call("assert_title", json!({"contains": "zzz"}))
        .is_err());
}

// ── 主流程（跨标签页 agent 任务）────────────────────────────

#[test]
fn main_flow_multi_tab_task() {
    let s = sdk();
    // 任务：搜索页 → 提取链接 → 打开目标 → 断言 → 清理
    s.open("https://example.com/search").unwrap();
    let links = s.tool_call("extract_links", json!({})).unwrap();
    let first = links["links"][0]["url"].as_str().unwrap().to_string();
    s.tool_call("new_tab", json!({"url": first})).unwrap();
    let tabs = s.tool_call("list_tabs", json!({})).unwrap();
    assert_eq!(tabs["tabs"].as_array().unwrap().len(), 2);
    s.tool_call("switch_tab", json!({"tab": 2})).unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], first);
    s.tool_call("close_tab", json!({"tab": 1})).unwrap();
    s.clear_state().unwrap();
    assert_eq!(
        s.tool_call("list_tabs", json!({})).unwrap()["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn main_flow_login_dashboard() {
    let s = sdk();
    s.open("https://example.com/login").unwrap();
    s.tool_call(
        "fill_form",
        json!({"values": {"b": "alice", "c": "secret"}}),
    )
    .unwrap();
    s.tool_call("checkbox", json!({"id": "d", "checked": true}))
        .unwrap();
    s.tool_call("select_option", json!({"id": "e", "value": "pro"}))
        .unwrap();
    // 提交（按钮 f）→ 转 dashboard
    s.tool_call("navigate", json!({"url": "https://example.com/dashboard"}))
        .unwrap();
    let text = s.tool_call("get_page_text", json!({})).unwrap();
    assert!(text["text"].as_str().unwrap().contains("Welcome"));
    s.tool_call("assert_element_exists", json!({"selector": "a"}))
        .unwrap();
}

// ── 会话持久化边缘 ───────────────────────────────────────────

#[test]
fn session_load_missing_file_errors() {
    let s = sdk();
    let r = s.session_load("/tmp/definitely-not-exists.json");
    assert!(r.is_err());
}

#[test]
fn session_empty_state_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.json");
    let path = path.to_str().unwrap().to_string();
    let s = sdk();
    s.open("https://example.com").unwrap();
    s.session_save(&path).unwrap();
    let s2 = sdk();
    s2.session_load(&path).unwrap();
    s2.open("https://example.com").unwrap();
    let c = s2.tool_call("cookie_get", json!({})).unwrap();
    assert_eq!(c["cookies"].as_array().unwrap().len(), 0);
}

// ── 并发（C ABI 单例线程安全）────────────────────────────────

#[test]
fn ffi_concurrent_calls_are_safe() {
    use std::ffi::{c_char, CStr};

    extern "C" {
        fn fastbrowser_init(cfg: *const c_char) -> *mut c_char;
        fn fastbrowser_open(url: *const c_char) -> *mut c_char;
        fn fastbrowser_tool_call(name: *const c_char, params: *const c_char) -> *mut c_char;
        fn fastbrowser_free_string(p: *mut c_char);
        fn fastbrowser_shutdown() -> *mut c_char;
    }

    // 串行化与其它 ffi 场景的初始化
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    unsafe {
        let cfg = std::ffi::CString::new(r#"{"engine":"mock"}"#).unwrap();
        let p = fastbrowser_init(cfg.as_ptr());
        let s = CStr::from_ptr(p).to_str().unwrap().to_string();
        fastbrowser_free_string(p);
        assert!(s.contains("\"ok\"") || s.contains("error"), "{s}");

        let url = std::ffi::CString::new("https://example.com").unwrap();
        let p = fastbrowser_open(url.as_ptr());
        let s = CStr::from_ptr(p).to_str().unwrap().to_string();
        fastbrowser_free_string(p);
        assert!(s.contains("Example Page"), "{s}");

        let threads: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    let name = std::ffi::CString::new("get_page_title").unwrap();
                    let params = std::ffi::CString::new("{}").unwrap();
                    let mut ok = 0;
                    for _ in 0..20 {
                        let p = fastbrowser_tool_call(name.as_ptr(), params.as_ptr());
                        let s = CStr::from_ptr(p).to_str().unwrap().to_string();
                        fastbrowser_free_string(p);
                        if s.contains("Example Page") {
                            ok += 1;
                        }
                    }
                    (i, ok)
                })
            })
            .collect();
        for t in threads {
            let (i, ok) = t.join().unwrap();
            assert_eq!(ok, 20, "thread {i} lost results");
        }
        let p = fastbrowser_shutdown();
        fastbrowser_free_string(p);
    }
}

// ── 确定性压力走查（无 panic / 不变量保持）───────────────────

#[test]
fn stress_random_walk_invariants() {
    let s = sdk();
    // 简单确定性伪随机（LGC），避免外部依赖
    let mut rng: u64 = 42;
    let mut next = move || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng >> 33) as usize
    };

    let urls = [
        "https://example.com",
        "https://example.com/login",
        "https://example.com/search",
        "about:blank",
    ];

    for _ in 0..400 {
        match next() % 16 {
            0 => {
                let url = urls[next() % urls.len()];
                let _ = s.open(url);
            }
            1 => {
                let url = urls[next() % urls.len()];
                let _ = s.navigate(url);
            }
            2 => {
                let _ = s.snapshot();
            }
            3 => {
                let _ = s.screenshot();
            }
            4 => {
                let _ = s.tool_call("extract_links", json!({}));
            }
            5 => {
                let _ = s.tool_call("click", json!({"id": (b'a' + (next() % 26) as u8) as char}));
            }
            6 => {
                let _ = s.tool_call("type", json!({"id": "e", "text": "x"}));
            }
            7 => {
                let _ = s.tool_call("new_tab", json!({"url": urls[next() % urls.len()]}));
            }
            8 => {
                let _ = s.tool_call("list_tabs", json!({})).unwrap();
            }
            9 => {
                if let Ok(tabs) = s.tool_call("list_tabs", json!({})) {
                    let arr = tabs["tabs"].as_array().unwrap();
                    if !arr.is_empty() {
                        let id = arr[next() % arr.len()]["id"].as_u64().unwrap() as u32;
                        let _ = s.tool_call("switch_tab", json!({"tab": id}));
                    }
                }
            }
            10 => {
                if let Ok(tabs) = s.tool_call("list_tabs", json!({})) {
                    let arr = tabs["tabs"].as_array().unwrap();
                    if !arr.is_empty() {
                        let id = arr[next() % arr.len()]["id"].as_u64().unwrap() as u32;
                        let _ = s.tool_call("close_tab", json!({"tab": id}));
                    }
                }
            }
            11 => {
                let _ = s.tool_call(
                    "cookie_set",
                    json!({"name": "k", "value": "v", "domain": "example.com"}),
                );
                let _ = s.tool_call("storage_set", json!({"key": "k", "value": "v"}));
                let _ = s.tool_call("cookie_clear", json!({}));
            }
            12 => {
                // 新工具：send_keys / get_history / done / search
                let combo = ["Enter", "Ctrl+a", "Tab"][next() % 3];
                let _ = s.tool_call("send_keys", json!({"keys": combo}));
                let _ = s.tool_call("get_history", json!({}));
                let _ = s.tool_call("search", json!({"query": "Welcome", "limit": 3}));
                let _ = s.tool_call("done", json!({"answer": "ok"}));
            }
            13 => {
                // 对话框与网络拦截（mock 优雅降级）
                let _ = s.tool_call("pending_dialog", json!({}));
                let _ = s.tool_call("dialog_accept", json!({}));
                let _ = s.tool_call("list_pending_requests", json!({}));
                let _ = s.tool_call("fulfill_request", json!({"request_id": "x"}));
            }
            14 => {
                // 提取/查找/无障碍
                let _ = s.tool_call("find_elements", json!({"selector": "a"}));
                let _ = s.tool_call("get_accessibility_tree", json!({}));
                let _ = s.tool_call("extract_forms", json!({}));
                let _ = s.tool_call("get_performance_metrics", json!({}));
            }
            15 => {
                // PDF 写入临时文件
                if let Some(dir) = std::env::temp_dir().to_str().map(String::from) {
                    let p = format!("{dir}/fb_stress_{}.pdf", next() % 5);
                    let _ = s.tool_call("save_as_pdf", json!({"path": p}));
                }
            }
            _ => unreachable!(),
        }
        // 不变量：active_tab 始终在 tabs 中（若存在）
        let tabs = s.tool_call("list_tabs", json!({})).unwrap();
        let ids: Vec<u64> = tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_u64().unwrap())
            .collect();
        let st = s.status();
        if let Some(active) = st["active_tab"].as_u64() {
            assert!(ids.contains(&active), "active tab {active} not in {ids:?}");
        }
    }
    // 最终一致性：无崩溃、活动标签不变量保持（上面已逐轮校验）
    let _final_tabs = s.tool_call("list_tabs", json!({})).unwrap();
}

// ── 渲染模式 ────────────────────────────────────────────────

#[test]
fn rendering_mode_switch_and_status() {
    let s = sdk();
    assert_eq!(s.status()["rendering_mode"], "headless");
    s.set_rendering_mode(RenderingMode::Hosted);
    assert_eq!(s.status()["rendering_mode"], "hosted");
    // 切换后工具仍可用
    s.open("https://example.com").unwrap();
    assert_eq!(
        s.tool_call("get_current_url", json!({})).unwrap()["url"],
        "https://example.com"
    );
}

#[test]
fn error_kind_serialization() {
    let e = EngineError::invalid("boom");
    let v = e.to_json();
    assert_eq!(v["error"]["kind"], "invalid_argument");
    let e = EngineError::not_initialized();
    assert_eq!(e.to_json()["error"]["kind"], "not_initialized");
}

// ── 原生并发（非 FFI）：Arc<Mutex<Fastbrowser>> 多线程调用 ────

#[test]
fn native_concurrent_tool_calls() {
    use std::sync::{Arc, Mutex};

    let s = Arc::new(Mutex::new(sdk()));
    s.lock().unwrap().open("https://example.com").unwrap();

    let threads: Vec<_> = (0..8)
        .map(|i| {
            let s = s.clone();
            std::thread::spawn(move || {
                let mut ok = 0;
                for _ in 0..25 {
                    let guard = s.lock().unwrap();
                    let v = guard.tool_call("get_page_title", json!({}));
                    if let Ok(o) = v {
                        if o["title"] == "Example Page" {
                            ok += 1;
                        }
                    }
                    drop(guard);
                }
                (i, ok)
            })
        })
        .collect();

    for t in threads {
        let (i, ok) = t.join().unwrap();
        assert_eq!(ok, 25, "thread {i} lost results");
    }
}

// ── SDK 级多 Profile 隔离（历史/标签页归属，mock 引擎）────────

#[test]
fn profile_isolation_at_sdk_level() {
    use fastbrowser::engine::TabId;

    let s = sdk();
    // 默认 profile 打开工作页（记下标签页 id）
    let default_tab = s.open("https://example.com").unwrap()["tab"]
        .as_u64()
        .unwrap() as u32;

    // 新建工作/生活 profile，切换活动 profile
    {
        let rt_guard = s.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        let work = rt
            .session()
            .create_profile("work", "mock", false, None, None, None);
        rt.session().set_active_profile(work).unwrap();
    }
    s.open("https://example.com/work").unwrap();
    {
        let rt_guard = s.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        let life = rt
            .session()
            .create_profile("life", "mock", false, None, None, None);
        rt.session().set_active_profile(life).unwrap();
    }
    s.open("https://example.com/life").unwrap();

    // 三个 profile，每个绑定自己的标签页；活动 profile 为 life
    let st = s.status();
    assert_eq!(st["profiles"].as_array().unwrap().len(), 3);

    // 访问历史按 profile 隔离（life 只看到 life 的访问）
    {
        let rt_guard = s.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        let life = rt.session().profile_id_by_name("life").unwrap();
        let hist = rt.session().visit_history(life);
        assert!(hist.iter().any(|u| u.contains("/life")));
        assert!(!hist.iter().any(|u| u.contains("/work")));
    }

    // 切回 default profile，其标签页仍在且可切换
    {
        let rt_guard = s.runtime().unwrap();
        let rt = rt_guard.as_ref().unwrap();
        let def = rt.session().profile_id_by_name("default").unwrap();
        rt.session().set_active_profile(def).unwrap();
    }
    let tabs = s.tool_call("list_tabs", json!({})).unwrap();
    let ids: Vec<u32> = tabs["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_u64().unwrap() as u32)
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&default_tab));
    // 切到 default 的首个标签页，URL 应为最初打开的页面
    s.runtime()
        .unwrap()
        .as_ref()
        .unwrap()
        .engine()
        .switch_tab(TabId(default_tab))
        .unwrap();
    let url = s.tool_call("get_current_url", json!({})).unwrap();
    assert_eq!(url["url"], "https://example.com");
}

// ── 工具参数深层校验（JSON 结构而非类型宽松）──────────────────

#[test]
fn deep_param_validation() {
    let s = sdk();
    s.open("https://example.com").unwrap();
    // type 需要字符串：给数字应报错而非静默成功
    let r = s.tool_call("type", json!({"id": "e", "text": 123}));
    assert!(r.is_err(), "type with non-string text should fail");
    // scroll 坐标给字符串应报错
    let r = s.tool_call("scroll", json!({"dy": "fast"}));
    assert!(r.is_err());
    // fill_form 值必须是对象
    let r = s.tool_call("fill_form", json!({"values": [1, 2]}));
    assert!(r.is_err());
    // block_request patterns 必须是数组
    let r = s.tool_call("block_request", json!({"patterns": "*.ads*"}));
    assert!(r.is_err());
}
