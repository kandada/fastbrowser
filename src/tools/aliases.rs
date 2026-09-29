// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Tool-name and parameter aliases for cross-ecosystem compatibility.
//!
//! Models trained on Playwright MCP or browser-use use their own tool/param
//! names. Resolving those here, in the kernel, means every consumer (aacode-rs,
//! the Voya desktop/mobile shells, the CLI) gets the compatibility for free
//! instead of each wrapper re-implementing it.

use serde_json::{Map, Value};

/// Tool names from Playwright MCP / browser-use → canonical fastbrowser name.
pub const TOOL_ALIASES: &[(&str, &str)] = &[
    // Playwright MCP style
    ("browser_navigate", "navigate"),
    ("browser_navigate_back", "back"),
    ("browser_navigate_forward", "forward"),
    ("browser_click", "click"),
    ("browser_type", "type"),
    ("browser_fill_form", "fill_form"),
    ("browser_press_key", "press"),
    ("browser_hover", "hover"),
    ("browser_drag", "drag"),
    ("browser_select_option", "select_option"),
    ("browser_file_upload", "upload_file"),
    ("browser_take_screenshot", "screenshot"),
    ("browser_snapshot", "get_accessibility_tree"),
    ("browser_evaluate", "execute_js"),
    ("browser_run_code", "execute_js"),
    ("browser_wait_for", "wait_for_text"),
    ("browser_resize", "set_viewport"),
    ("browser_tabs", "list_tabs"),
    ("browser_tab_list", "list_tabs"),
    ("browser_tab_new", "new_tab"),
    ("browser_tab_select", "switch_tab"),
    ("browser_tab_close", "close_tab"),
    ("browser_close", "close_tab"),
    ("browser_handle_dialog", "dialog_accept"),
    // browser-use style
    ("go_to_url", "navigate"),
    ("open_url", "navigate"),
    ("click_element", "click"),
    ("click_element_by_index", "click"),
    ("input_text", "type"),
    ("scroll_down", "scroll"),
    ("scroll_up", "scroll"),
    ("extract_content", "extract_text"),
    ("get_ax_tree", "get_accessibility_tree"),
    ("go_back", "back"),
    ("refresh", "reload"),
    ("open_tab", "new_tab"),
    ("select_dropdown_option", "select_option"),
    // ── Playwright raw API (page.* / locator.*) ──
    ("page.goto", "navigate"),
    ("page.goBack", "back"),
    ("page.goForward", "forward"),
    ("page.reload", "reload"),
    ("page.click", "click"),
    ("page.fill", "type"),
    ("page.type", "type"),
    ("page.press", "press"),
    ("page.hover", "hover"),
    ("page.dragAndDrop", "drag"),
    ("page.selectOption", "select_option"),
    ("page.check", "checkbox"),
    ("page.uncheck", "checkbox"),
    ("page.setInputFiles", "upload_file"),
    ("page.textContent", "get_element_text"),
    ("page.innerText", "get_element_text"),
    ("page.title", "get_page_title"),
    ("page.url", "get_current_url"),
    ("page.content", "extract_html"),
    ("page.screenshot", "screenshot"),
    ("page.waitForSelector", "wait_for_element"),
    ("page.waitForLoadState", "wait_for_load_state"),
    ("page.locator", "find_elements"),
    ("page.$", "find_elements"),
    ("page.$$", "find_elements"),
    ("page.evaluate", "execute_js"),
    ("page.setViewportSize", "set_viewport"),
    ("page.accessibility.snapshot", "get_accessibility_tree"),
    // ── Puppeteer ──
    ("goto", "navigate"),
    ("browser_open", "navigate"),
    ("browser_goto", "navigate"),
    ("browser_open_url", "navigate"),
    ("browser_goto_url", "navigate"),
    ("open", "navigate"),
    ("open_page", "navigate"),
    ("waitForSelector", "wait_for_element"),
    ("waitForNavigation", "wait_for_navigation"),
    ("setViewport", "set_viewport"),
    ("$eval", "execute_js"),
    ("$$eval", "execute_js"),
    ("keyboard.press", "press"),
    // ── Selenium / Appium (WebDriver) ──
    ("get", "navigate"),
    ("getText", "get_page_text"),
    ("getTitle", "get_page_title"),
    ("getCurrentUrl", "get_current_url"),
    ("getPageSource", "extract_html"),
    ("findElement", "find_elements"),
    ("findElements", "find_elements"),
    ("sendKeys", "type"),
    ("getScreenshotAs", "screenshot"),
    ("executeScript", "execute_js"),
    ("executeAsyncScript", "execute_js"),
    ("switchTo", "switch_tab"),
    ("tap", "click"),
    ("getAccessibilityTree", "get_accessibility_tree"),
    // A bare `wait` → wait for the page (models often guess this name).
    ("wait", "wait_for_load_state"),
    // ── Accessibility dialect (various libraries) ──
    ("accessibility_snapshot", "get_accessibility_tree"),
    ("accessibility", "get_accessibility_tree"),
    ("ax_tree", "get_accessibility_tree"),
    ("a11y_tree", "get_accessibility_tree"),
    ("get_accessibility_snapshot", "get_accessibility_tree"),
    // ── Playwright locator / getBy* ──
    ("locator", "find_elements"),
    ("get_by_role", "find_elements"),
    ("getByRole", "find_elements"),
    ("get_by_text", "find_elements"),
    ("getByText", "find_elements"),
    ("get_by_label", "find_elements"),
    ("getByLabel", "find_elements"),
    ("get_by_placeholder", "find_elements"),
    ("getByPlaceholder", "find_elements"),
    ("get_by_alt_text", "find_elements"),
    ("getByAltText", "find_elements"),
    ("get_by_title", "find_elements"),
    ("getByTitle", "find_elements"),
    ("get_by_test_id", "find_elements"),
    ("getByTestId", "find_elements"),
    // ARIA snapshot dialects (Playwright `ariaSnapshot`, Testing Library a11y).
    ("aria_snapshot", "get_accessibility_tree"),
    ("ariaSnapshot", "get_accessibility_tree"),
    ("snapshot_accessibility", "get_accessibility_tree"),
    ("get_accessibility", "get_accessibility_tree"),
    // ── locator methods ──
    ("scroll_into_view", "scroll"),
    ("scrollIntoView", "scroll"),
    ("input_value", "get_attributes"),
    ("inputValue", "get_attributes"),
    ("text_content", "get_element_text"),
    ("textContent", "get_element_text"),
    ("inner_html", "get_element_info"),
    ("innerHTML", "get_element_info"),
    ("is_checked", "get_attributes"),
    ("isChecked", "get_attributes"),
    ("bounding_box", "get_element_info"),
    ("boundingBox", "get_element_info"),
    // ── misc ──
    ("take_screenshot", "screenshot"),
    ("screenshot_page", "screenshot"),
    ("full_page_screenshot", "screenshot"),
    ("fullPageScreenshot", "screenshot"),
    ("wait_for_selector", "wait_for_element"),
    ("waitForSelector", "wait_for_element"),
    ("wait_for_timeout", "wait_for_load_state"),
    ("waitForTimeout", "wait_for_load_state"),
    ("set_user_agent", "set_viewport"),
    ("setUserAgent", "set_viewport"),
    // ── page.* / browser_* long tail (Playwright / Puppeteer / Selenium) ──
    ("page.pdf", "save_as_pdf"),
    ("browser_pdf_save", "save_as_pdf"),
    ("page.setCookie", "cookie_set"),
    ("browser_set_cookie", "cookie_set"),
    ("addCookie", "cookie_set"),
    ("page.cookies", "cookie_get"),
    ("browser_get_cookies", "cookie_get"),
    ("getCookies", "cookie_get"),
    ("deleteCookies", "clear_cookies"),
    ("browser_clear_cookies", "clear_cookies"),
    ("page.authenticate", "set_basic_auth"),
    ("browser_authenticate", "set_basic_auth"),
    ("page.addStyleTag", "inject_css"),
    ("browser_add_style_tag", "inject_css"),
    ("page.addScriptTag", "execute_js"),
    ("browser_add_script_tag", "execute_js"),
    ("page.metrics", "get_performance_metrics"),
    ("browser_metrics", "get_performance_metrics"),
    ("page.waitForFunction", "wait_for_condition"),
    ("waitForFunction", "wait_for_condition"),
    ("browser_wait_for_function", "wait_for_condition"),
    ("page.waitForNetworkIdle", "wait_for_load_state"),
    ("waitForNetworkIdle", "wait_for_load_state"),
    ("browser_wait_for_network_idle", "wait_for_load_state"),
    ("browser_check", "checkbox"),
    ("browser_uncheck", "checkbox"),
    ("browser_select", "select_option"),
    ("browser_is_visible", "is_visible"),
    ("isDisplayed", "is_visible"),
    ("isVisible", "is_visible"),
    ("browser_is_enabled", "is_enabled"),
    ("isEnabled", "is_enabled"),
    ("browser_get_attribute", "get_attributes"),
    ("getAttribute", "get_attributes"),
    ("browser_get_text", "get_element_text"),
    ("browser_get_value", "get_attributes"),
    ("browser_query_selector", "find_elements"),
    ("querySelector", "find_elements"),
    ("querySelectorAll", "find_elements"),
    ("browser_click_at", "click_coords"),
    ("browser_new_page", "new_tab"),
    ("newPage", "new_tab"),
    ("browser_switch_page", "switch_tab"),
    ("switchPage", "switch_tab"),
    ("browser_close_page", "close_tab"),
    ("browser_swipe", "swipe"),
    ("browser_set_viewport", "set_viewport"),
    ("setViewportSize", "set_viewport"),
    ("browser_geolocation", "set_geolocation"),
    ("browser_timezone", "set_timezone"),
    ("browser_route", "intercept_request"),
    ("setRequestInterception", "intercept_request"),
    ("browser_abort_request", "abort_request"),
    ("abortRequest", "abort_request"),
    ("browser_fulfill_request", "fulfill_request"),
    ("fulfillRequest", "fulfill_request"),
    ("acceptDialog", "dialog_accept"),
    ("dismissDialog", "dialog_dismiss"),
    ("keyboardPress", "press"),
    ("keyPress", "press"),
    ("pressKey", "press"),
    ("browser_fill", "type"),
    ("fill", "type"),
    ("fillInput", "type"),
    ("browser_clear", "clear_input"),
    ("clearInput", "clear_input"),
    ("contextClick", "right_click"),
    ("rightClick", "right_click"),
    ("doubleClick", "dblclick"),
    ("browser_double_click", "dblclick"),
    ("browser_get_history", "get_history"),
    ("elementScreenshot", "screenshot_element"),
    ("browser_element_screenshot", "screenshot_element"),
    ("browser_get_url", "get_current_url"),
    ("getUrl", "get_current_url"),
    ("currentUrl", "get_current_url"),
    ("browser_get_title", "get_page_title"),
    ("getTitle", "get_page_title"),
    ("evaluateHandle", "execute_js"),
    ("browser_evaluate_handle", "execute_js"),
    ("browser_storage_get", "storage_get"),
    ("browser_storage_set", "storage_set"),
    ("browser_scroll", "scroll"),
    ("browser_console_messages", "get_console_logs"),
    ("get_console_messages", "get_console_logs"),
    ("console_messages", "get_console_logs"),
    ("page.console", "get_console_logs"),
    ("browser_network_requests", "get_network_log"),
    ("get_network_requests", "get_network_log"),
    ("network_requests", "get_network_log"),
    ("page.network", "get_network_log"),
    ("list_requests", "get_network_log"),
    ("get_requests", "get_network_log"),
    ("browser_focus", "focus"),
    ("browser_blur", "blur"),
    ("browser_press", "press"),
    ("browser_hover", "hover"),
    ("browser_drag", "drag"),
    ("browser_dblclick", "dblclick"),
    ("browser_right_click", "right_click"),
];

/// Computer-use / desktop-automation style tool names → surface tools.
///
/// Only meaningful when the `surface` feature is compiled (the targets are
/// registered then); kept separate from [`TOOL_ALIASES`] so the base compat
/// table never references an unregistered tool.
#[cfg(feature = "surface")]
pub const SURFACE_TOOL_ALIASES: &[(&str, &str)] = &[
    // Listing surfaces
    ("list_windows", "ax_list"),
    ("list_apps", "ax_list"),
    ("desktop_list", "ax_list"),
    ("surface_list", "ax_list"),
    ("list_surfaces", "ax_list"),
    // Unified snapshots
    ("ui_snapshot", "ax_snapshot"),
    ("desktop_snapshot", "ax_snapshot"),
    ("surface_snapshot", "ax_snapshot"),
    ("computer_snapshot", "ax_snapshot"),
    // Unified actions
    ("desktop_act", "ax_act"),
    ("ui_act", "ax_act"),
    ("computer", "ax_act"),
    ("surface_act", "ax_act"),
    ("desktop_click", "ax_click"),
    ("ui_click", "ax_click"),
    ("surface_click", "ax_click"),
    ("desktop_type", "ax_type"),
    ("ui_type", "ax_type"),
    ("surface_type", "ax_type"),
    ("desktop_scroll", "ax_scroll"),
    ("ui_scroll", "ax_scroll"),
    ("surface_scroll", "ax_scroll"),
    ("surface_events", "ax_events"),
];

/// Param names commonly sent by those ecosystems → canonical param name.
///
/// Applied only when the target param is declared by the tool and the alias is
/// not, so a genuinely-declared name (e.g. `expr` on `evaluate_xpath`) is never
/// rewritten.
pub const PARAM_ALIASES: &[(&str, &str)] = &[
    // execute_js / extract_json → script ; evaluate_xpath → expr
    ("expression", "script"),
    ("expr", "script"),
    ("code", "script"),
    ("js", "script"),
    ("javascript", "script"),
    ("src", "script"),
    ("expression", "expr"),
    ("xpath", "expr"),
    // navigate
    ("uri", "url"),
    ("link", "url"),
    ("address", "url"),
    // search / find_elements / wait_for_element
    ("q", "query"),
    ("keyword", "query"),
    ("css", "selector"),
    // type
    ("content", "text"),
    // browser-use index → snapshot id
    ("index", "id"),
    // dialog
    ("promptText", "prompt_text"),
    // viewport / limits
    ("max_content_length", "max_chars"),
    ("max_length", "max_chars"),
    ("timeout", "timeout_ms"),
    // surface tools
    ("surface", "target"),
    ("window", "target"),
    ("node", "ref"),
    // assert_title / assert_text_contains take `contains`; evaluate_xpath takes
    // `expr`. Both aliases are tried in order and only applied when the tool
    // declares the target.
    ("query", "contains"),
    ("query", "expr"),
    ("q", "contains"),
    ("title", "contains"),
    ("waitUntil", "wait_until"),
    ("viewportSize", "viewport"),
    ("maxNodes", "max_nodes"),
    ("maxDepth", "max_depth"),
    ("maxTextLen", "max_text_len"),
    ("elementId", "id"),
    ("element_id", "id"),
    ("nodeId", "id"),
    ("fileName", "path"),
    ("filename", "path"),
    // locator/option dialects
    ("delay", "timeout_ms"),
    // timeout / limit
    ("ms", "timeout_ms"),
    ("milliseconds", "timeout_ms"),
    ("max", "limit"),
    ("max", "max_nodes"),
    ("count", "limit"),
    ("top", "limit"),
    // upload
    ("file", "paths"),
    ("files", "paths"),
    ("upload", "paths"),
    // select_option
    ("option", "value"),
    ("label", "value"),
    ("visibleText", "value"),
    // url / assert
    ("href", "url"),
    ("site", "url"),
    ("target_url", "url"),
    ("targetUrl", "url"),
    ("expected", "contains"),
    ("substring", "contains"),
    // wait / intercept
    ("load_state", "state"),
    ("until", "state"),
    ("pattern", "patterns"),
    ("urls", "patterns"),
    ("url_pattern", "patterns"),
    // auth / geo / tz
    ("user", "username"),
    ("user_name", "username"),
    ("pass", "password"),
    ("lat", "latitude"),
    ("lng", "longitude"),
    ("lon", "longitude"),
    ("timezone", "timezone_id"),
    // drag
    ("source", "from"),
    ("start", "from"),
    ("end", "to"),
    ("target", "to"),
    // scroll / keys
    ("deltaX", "dx"),
    ("delta_x", "dx"),
    ("deltaY", "dy"),
    ("delta_y", "dy"),
    ("keycode", "key"),
    ("combo", "key"),
    ("shortcut", "key"),
    ("keys", "key"),
    ("key", "keys"),
    // `send_keys` takes `keys`; agents often send `text`.
    ("text", "keys"),
];

/// Resolve a tool name: alias → canonical, otherwise unchanged.
pub fn resolve_tool_alias(name: &str) -> String {
    if let Some((_, target)) = TOOL_ALIASES.iter().find(|(alias, _)| *alias == name) {
        return target.to_string();
    }
    #[cfg(feature = "surface")]
    if let Some((_, target)) = SURFACE_TOOL_ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
    {
        return target.to_string();
    }
    name.to_string()
}

/// `maxChars` → `max_chars`, `elementId` → `element_id` (idempotent for
/// already-snake_case input). Used as a fallback when the alias table misses.
fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Params an agent may send that are NOT tool arguments — they belong to the
/// `browser_call` wrapper. Drop them when the tool doesn't declare them,
/// instead of emitting confusing "unknown param" warnings.
const IGNORED_PARAMS: &[&str] = &[
    "compact",
    "clean",
    "max_chars",
    "max_length",
    "timeout",
    "timeout_ms",
    "timeout_secs",
];

/// Rename alias params to the tool's declared names. `declared` is the tool's
/// `params` object (`{name: {...}}`); unknown keys pass through unchanged.
pub fn normalize_params(params: Value, declared: &Value) -> Value {
    let Some(declared) = declared.as_object() else {
        return params;
    };
    let Some(obj) = params.as_object() else {
        return params;
    };
    let mut out = Map::new();
    for (k, v) in obj {
        if declared.contains_key(k) {
            out.insert(k.clone(), v.clone());
            continue;
        }
        // Alias table first; then a general camelCase → snake_case fallback
        // (e.g. `maxChars` → `max_chars`) so LLM casing quirks just work.
        let mapped: Option<String> = PARAM_ALIASES
            .iter()
            .find_map(|(alias, target)| {
                (*alias == k && declared.contains_key(*target)).then(|| (*target).to_string())
            })
            .or_else(|| {
                let snake = to_snake_case(k);
                (snake != *k && declared.contains_key(&snake)).then_some(snake)
            });
        match mapped {
            Some(target) if !out.contains_key(&target) => {
                out.insert(target, v.clone());
            }
            _ => {
                // Drop wrapper-only params the tool doesn't understand.
                if !IGNORED_PARAMS.contains(&k.as_str()) {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
    }
    Value::Object(out)
}

/// Coerce string params to the declared numeric/boolean types. LLMs commonly
/// send `"timeout_ms": "10000"` or `"checked": "true"`; the kernel normalizes
/// this once so every consumer (aacode-rs, voya-core, CLI) benefits.
pub fn coerce_params(params: Value, declared: &Value) -> Value {
    let Some(defs) = declared.as_object() else {
        return params;
    };
    let Some(obj) = params.as_object() else {
        return params;
    };
    let mut out = Map::new();
    for (k, v) in obj {
        let ty = defs
            .get(k)
            .and_then(|d| d.get("type"))
            .and_then(Value::as_str);
        let coerced = match (ty, v) {
            (Some("integer" | "number"), Value::String(s)) => {
                let s = s.trim();
                if let Ok(n) = s.parse::<i64>() {
                    serde_json::json!(n)
                } else if let Ok(f) = s.parse::<f64>() {
                    serde_json::json!(f)
                } else {
                    v.clone()
                }
            }
            (Some("boolean"), Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => serde_json::json!(true),
                "false" | "0" | "no" => serde_json::json!(false),
                _ => v.clone(),
            },
            // LLM commonly sends a single path/string where an array is declared
            // (e.g. `paths: "a.txt"`); wrap it into a one-element array instead of
            // failing the whole call with a type error.
            (Some("array"), Value::String(s)) => serde_json::json!([s]),
            _ => v.clone(),
        };
        out.insert(k.clone(), coerced);
    }
    Value::Object(out)
}

/// Params not declared by the tool → warnings so the model can self-correct
/// instead of silently failing. Call after `normalize_params`.
pub fn unknown_params(params: &Value, declared: &Value) -> Vec<String> {
    let Some(defs) = declared.as_object() else {
        return Vec::new();
    };
    let Some(obj) = params.as_object() else {
        return Vec::new();
    };
    obj.keys()
        .filter(|k| !defs.contains_key(k.as_str()))
        .map(|k| format!("unknown param '{k}'"))
        .collect()
}

/// Full preparation: alias-normalize → type-coerce → collect unknown-param
/// warnings. Returns the prepared params and the warnings.
pub fn prepare_params(params: Value, declared: &Value) -> (Value, Vec<String>) {
    let params = normalize_params(params, declared);
    let params = coerce_params(params, declared);
    let warnings = unknown_params(&params, declared);
    (params, warnings)
}

/// Attach `warnings` to an object result (no-op for non-objects / empty).
pub fn attach_warnings(result: Value, warnings: Vec<String>) -> Value {
    if warnings.is_empty() {
        return result;
    }
    match result {
        Value::Object(mut o) => {
            o.insert("warnings".into(), serde_json::json!(warnings));
            Value::Object(o)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerce_single_string_to_array() {
        // LLM sends `paths: "a.txt"` where an array is declared → wrap it.
        let declared = json!({"paths": {"type": "array"}});
        let out = coerce_params(json!({"paths": "verify2/up_test.txt"}), &declared);
        assert_eq!(out["paths"], json!(["verify2/up_test.txt"]));
        // A proper array is left untouched.
        let out = coerce_params(json!({"paths": ["a", "b"]}), &declared);
        assert_eq!(out["paths"], json!(["a", "b"]));
    }

    #[test]
    fn resolves_known_aliases_and_keeps_canonical() {
        assert_eq!(resolve_tool_alias("browser_navigate"), "navigate");
        assert_eq!(resolve_tool_alias("click_element"), "click");
        assert_eq!(resolve_tool_alias("go_to_url"), "navigate");
        assert_eq!(
            resolve_tool_alias("browser_snapshot"),
            "get_accessibility_tree"
        );
        assert_eq!(resolve_tool_alias("navigate"), "navigate");
        assert_eq!(resolve_tool_alias("unknown_tool_xyz"), "unknown_tool_xyz");
    }

    #[test]
    fn normalizes_params_by_declared_names() {
        let execute_js = json!({"script": {"type": "string", "required": true}});
        // `expr`/`expression`/`code` → script for execute_js
        for alias in ["expr", "expression", "code", "js"] {
            let out = normalize_params(json!({alias: "1+1"}), &execute_js);
            assert_eq!(out["script"], json!("1+1"), "alias {alias}");
            assert!(out.get(alias).is_none());
        }
        // evaluate_xpath declares `expr` → kept as-is
        let evaluate_xpath = json!({"expr": {"type": "string", "required": true}});
        let out = normalize_params(json!({"expr": "//a"}), &evaluate_xpath);
        assert_eq!(out["expr"], json!("//a"));
        // declared key wins over alias
        let out = normalize_params(json!({"script": "a", "code": "b"}), &execute_js);
        assert_eq!(out["script"], json!("a"));
        // unknown key passes through
        let out = normalize_params(json!({"foo": 1}), &execute_js);
        assert_eq!(out["foo"], json!(1));
    }

    #[test]
    fn query_aliases_to_contains_for_assert_title() {
        // assert_title / assert_text_contains declare `contains`, but models
        // often send `query`/`q`/`title`.
        let assert_title = json!({"contains": {"type": "string", "required": true}});
        for alias in ["query", "q", "title"] {
            let out = normalize_params(json!({alias: "Example"}), &assert_title);
            assert_eq!(out["contains"], json!("Example"), "alias {alias}");
            assert!(out.get(alias).is_none());
        }
        // `query` must stay `query` when the tool declares it (e.g. search_web).
        let search = json!({"query": {"type": "string", "required": true}});
        let out = normalize_params(json!({"query": "rust"}), &search);
        assert_eq!(out["query"], json!("rust"));
        // `q` still prefers `query` over `contains` when both are declared.
        let both = json!({"query": {"type": "string"}, "contains": {"type": "string"}});
        let out = normalize_params(json!({"q": "x"}), &both);
        assert_eq!(out["query"], json!("x"));
    }

    #[test]
    fn browser_open_family_aliases_to_navigate() {
        for a in [
            "browser_open",
            "browser_goto",
            "browser_open_url",
            "browser_goto_url",
            "open",
            "open_page",
        ] {
            assert_eq!(resolve_tool_alias(a), "navigate", "{a}");
        }
    }

    #[test]
    fn camel_case_param_fallback() {
        let declared = json!({"max_chars": {"type": "integer"}, "some_param": {"type": "string"}});
        let out = normalize_params(json!({"maxChars": 100, "someParam": "x"}), &declared);
        assert_eq!(out["max_chars"], json!(100));
        assert_eq!(out["some_param"], json!("x"));
        // A declared camelCase key wins (no rewrite).
        let declared2 = json!({"maxChars": {"type": "integer"}});
        assert_eq!(
            normalize_params(json!({"maxChars": 1}), &declared2)["maxChars"],
            json!(1)
        );
        // Alias table still wins over the snake fallback.
        let click = json!({"id": {"type": "string"}});
        assert_eq!(
            normalize_params(json!({"elementId": "abc"}), &click)["id"],
            json!("abc")
        );
        assert_eq!(to_snake_case("maxChars"), "max_chars");
        assert_eq!(to_snake_case("already_snake"), "already_snake");
    }

    #[test]
    fn extended_page_and_webdriver_aliases() {
        for (a, t) in [
            ("page.pdf", "save_as_pdf"),
            ("page.setCookie", "cookie_set"),
            ("page.cookies", "cookie_get"),
            ("page.authenticate", "set_basic_auth"),
            ("page.addStyleTag", "inject_css"),
            ("page.waitForFunction", "wait_for_condition"),
            ("page.waitForNetworkIdle", "wait_for_load_state"),
            ("querySelector", "find_elements"),
            ("querySelectorAll", "find_elements"),
            ("getAttribute", "get_attributes"),
            ("isDisplayed", "is_visible"),
            ("isEnabled", "is_enabled"),
            ("contextClick", "right_click"),
            ("doubleClick", "dblclick"),
            ("acceptDialog", "dialog_accept"),
            ("newPage", "new_tab"),
            ("switchPage", "switch_tab"),
            ("browser_fulfill_request", "fulfill_request"),
            ("browser_storage_get", "storage_get"),
        ] {
            assert_eq!(resolve_tool_alias(a), t, "{a}");
        }
        // Canonical names are never rewritten.
        for c in [
            "navigate",
            "fill_form",
            "clear_input",
            "select_option",
            "get_current_url",
        ] {
            assert_eq!(resolve_tool_alias(c), c, "{c}");
        }
    }

    #[test]
    fn console_and_network_aliases() {
        for (a, t) in [
            ("browser_console_messages", "get_console_logs"),
            ("get_console_messages", "get_console_logs"),
            ("console_messages", "get_console_logs"),
            ("page.console", "get_console_logs"),
            ("browser_network_requests", "get_network_log"),
            ("get_network_requests", "get_network_log"),
            ("network_requests", "get_network_log"),
            ("list_requests", "get_network_log"),
        ] {
            assert_eq!(resolve_tool_alias(a), t, "{a}");
        }
        // Canonical names unchanged.
        assert_eq!(resolve_tool_alias("get_console_logs"), "get_console_logs");
        assert_eq!(resolve_tool_alias("get_network_log"), "get_network_log");
    }

    #[test]
    fn max_alias_targets_max_nodes_when_declared() {
        // `max` → `max_nodes` for the AX tree (which declares max_nodes, not limit)
        let declared = json!({"max_nodes": {"type": "integer"}});
        let out = normalize_params(json!({"max": 30}), &declared);
        assert!(out.get("max_nodes").is_some(), "{out}");
        // …while still targeting `limit` for tools that declare it.
        let declared = json!({"limit": {"type": "integer"}});
        let out = normalize_params(json!({"max": 5}), &declared);
        assert!(out.get("limit").is_some(), "{out}");
    }

    #[test]
    fn query_and_text_aliases_resolve_by_declared_target() {
        // evaluate_xpath declares `expr` → `query` maps to expr.
        let declared = json!({"expr": {"type": "string"}});
        let out = normalize_params(json!({"query": "//a"}), &declared);
        assert_eq!(out["expr"], "//a");
        // send_keys declares `keys` → `text` maps to keys.
        let declared = json!({"keys": {"type": "string"}});
        let out = normalize_params(json!({"text": "bob"}), &declared);
        assert_eq!(out["keys"], "bob");
        // assert_text_contains declares `contains` → `query` still maps there.
        let declared = json!({"contains": {"type": "string"}});
        let out = normalize_params(json!({"query": "Login"}), &declared);
        assert_eq!(out["contains"], "Login");
        // Wrapper-only params are dropped (no "unknown param" warning).
        let declared = json!({"selector": {"type": "string"}});
        let out = normalize_params(
            json!({"selector": "a", "compact": true, "timeout_ms": 5}),
            &declared,
        );
        assert_eq!(out["selector"], "a");
        assert!(out.get("compact").is_none(), "{out}");
        assert!(out.get("timeout_ms").is_none(), "{out}");
    }

    #[test]
    fn extended_param_aliases() {
        // ms → timeout_ms
        let declared = json!({"timeout_ms": {"type": "number"}});
        assert_eq!(
            normalize_params(json!({"ms": 3000}), &declared)["timeout_ms"],
            json!(3000)
        );
        // max → limit
        let find = json!({"limit": {"type": "integer"}});
        assert_eq!(
            normalize_params(json!({"max": 5}), &find)["limit"],
            json!(5)
        );
        // file(s) → paths (upload_file)
        let up = json!({"paths": {"type": "array"}});
        assert_eq!(
            normalize_params(json!({"files": ["a"]}), &up)["paths"],
            json!(["a"])
        );
        // target → to (drag), but declared `target` wins (surface tools)
        let drag = json!({"from": {"type": "string"}, "to": {"type": "string"}});
        assert_eq!(
            normalize_params(json!({"target": "#b"}), &drag)["to"],
            json!("#b")
        );
        let surface = json!({"target": {"type": "string"}});
        assert_eq!(
            normalize_params(json!({"target": "w"}), &surface)["target"],
            json!("w")
        );
        // keys/key cross-map
        let press = json!({"key": {"type": "string"}});
        assert_eq!(
            normalize_params(json!({"keys": "Enter"}), &press)["key"],
            json!("Enter")
        );
        let send = json!({"keys": {"type": "string"}});
        assert_eq!(
            normalize_params(json!({"key": "Enter"}), &send)["keys"],
            json!("Enter")
        );
    }

    #[test]
    fn navigate_aliases() {
        let navigate = json!({"url": {"type": "string", "required": true}});
        for alias in ["uri", "link", "address"] {
            let out = normalize_params(json!({alias: "https://x"}), &navigate);
            assert_eq!(out["url"], json!("https://x"), "alias {alias}");
        }
    }

    #[test]
    fn index_alias_maps_to_id() {
        // browser-use 的 `index` → 声明了 `id` 的工具。
        let click = json!({
            "id": {"type": "string", "required": false},
            "selector": {"type": "string", "required": false}
        });
        let out = normalize_params(json!({"index": 3}), &click);
        assert_eq!(out["id"], json!(3));
        assert!(out.get("index").is_none());
    }

    #[cfg(feature = "surface")]
    #[test]
    fn computer_alias_routes_to_ax_act() {
        assert_eq!(resolve_tool_alias("computer"), "ax_act");
        assert_eq!(resolve_tool_alias("list_windows"), "ax_list");
        assert_eq!(resolve_tool_alias("ui_snapshot"), "ax_snapshot");
        assert_eq!(resolve_tool_alias("surface_snapshot"), "ax_snapshot");
    }

    #[test]
    fn well_known_library_tool_aliases() {
        for (alias, target) in [
            ("page.goto", "navigate"),
            ("page.fill", "type"),
            ("page.textContent", "get_element_text"),
            ("page.accessibility.snapshot", "get_accessibility_tree"),
            ("goto", "navigate"),
            ("getText", "get_page_text"),
            ("getTitle", "get_page_title"),
            ("findElement", "find_elements"),
            ("sendKeys", "type"),
            ("tap", "click"),
            ("getAccessibilityTree", "get_accessibility_tree"),
            ("wait", "wait_for_load_state"),
            ("accessibility_snapshot", "get_accessibility_tree"),
            ("ax_tree", "get_accessibility_tree"),
            ("a11y_tree", "get_accessibility_tree"),
        ] {
            assert_eq!(resolve_tool_alias(alias), target, "alias {alias}");
        }
        // Canonical names are never rewritten.
        assert_eq!(resolve_tool_alias("navigate"), "navigate");
        assert_eq!(
            resolve_tool_alias("get_accessibility_tree"),
            "get_accessibility_tree"
        );
    }

    #[test]
    fn library_param_aliases() {
        let declared =
            serde_json::json!({"max_nodes": {"type": "integer"}, "id": {"type": "string"}});
        let out = normalize_params(
            serde_json::json!({"maxNodes": 5, "elementId": "a"}),
            &declared,
        );
        assert_eq!(out["max_nodes"], 5);
        assert_eq!(out["id"], "a");
    }
}
