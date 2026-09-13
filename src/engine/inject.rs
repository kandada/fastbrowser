// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 注入页面的 JS 运行时（webview / cdp 引擎共用）。
//!
//! V3 返回结构：
//! ```json
//! {
//!   "elements": [ { "id","tag","role","text","href","rect","value","input_type",
//!                   "checked","disabled","placeholder","name","aria_label","visible" } ],
//!   "meta": { "total", "truncated", "viewport_h", "scroll_h", "scroll_y" },
//!   "frames": [ { "url","name","cross_origin", "elements":[...], "meta":{...} } ]
//! }
//! ```
//! - `elements`：顶层文档（含 open shadow root）的可交互元素；
//! - `frames`：同源 iframe 的可交互元素（跨域 iframe 仅记 url + cross_origin）；
//! - 坐标一律换算为**顶层视口坐标**（iframe 偏移累加）。

use std::collections::HashMap;

use serde_json::Value;

use crate::engine::snapshot::{FrameSnapshot, InteractiveElement, SnapshotMeta};
use crate::engine::{ElementRef, EngineError, ErrorKind, Rect, Result};

/// 跨文档/跨 shadow root 定位元素（供动作脚本与点击几何复用）。
pub fn fb_find_js() -> &'static str {
    r#"(function(){
  function fbFind(id){
    function walk(root){
      var el=root.querySelector('[data-fb="'+id+'"]');
      if(el) return el;
      var all=root.querySelectorAll('*');
      for(var i=0;i<all.length;i++){
        var n=all[i];
        if(n.shadowRoot){ var r=walk(n.shadowRoot); if(r) return r; }
        if(n.tagName==='IFRAME'){ var d=null; try{d=n.contentDocument;}catch(e){} if(d){ var r2=walk(d); if(r2) return r2; } }
      }
      return null;
    }
    return walk(document);
  }
  window.__fbFind=fbFind;
})()"#
}

/// 由 `data-fb` 定位元素（跨 iframe / shadow）并执行动作。
/// `body` 无需以分号结尾；此处自动补齐 `;` 再 `return 'ok'`。
pub fn action_js(id: char, body: &str) -> String {
    format!(
        r#"(function(){{var el=window.__fbFind?window.__fbFind("{id}"):document.querySelector('[data-fb="{id}"]');if(!el)return 'notfound';{body};return 'ok';}})()"#
    )
}

/// 标注可交互元素并返回 JSON 对象字符串（含 frames）。
pub fn runtime_extract_js() -> &'static str {
    r#"/*FB_EXTRACT_V3*/
(function () {
  var MAX = 26; /* a..z */
  var INTERACTIVE_ROLES = {
    button:1, link:1, checkbox:1, radio:1, menuitem:1, menuitemcheckbox:1,
    menuitemradio:1, option:1, combobox:1, slider:1, switch:1, tab:1,
    textbox:1, searchbox:1, spinbutton:1, listbox:1, treeitem:1, gridcell:1,
    row:1, menubar:1, tablist:1, tabpanel:1, dialog:1, alertdialog:1, tooltip:1
  };
  function isInteractive(el) {
    var tag = el.tagName.toLowerCase();
    if (tag === 'a' || tag === 'button' || tag === 'input' || tag === 'select' ||
        tag === 'textarea' || tag === 'summary' || tag === 'label') return true;
    var role = (el.getAttribute('role') || '').toLowerCase();
    if (role && INTERACTIVE_ROLES[role]) return true;
    if (el.getAttribute('contenteditable') === 'true') return true;
    if (typeof el.tabIndex === 'number' && el.tabIndex >= 0) return true;
    if (el.hasAttribute('onclick') || el.hasAttribute('onmousedown') || el.hasAttribute('onkeydown')) return true;
    return false;
  }
  function visible(el) {
    if (el.offsetParent === null && el.getClientRects().length === 0) return false;
    var cs = window.getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || cs.pointerEvents === 'none') return false;
    var r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  }
  var frames = [];
  var grandTotal = 0;
  var truncated = false;
  var idc = 97;

  function collect(doc, ox, oy) {
    var res = { elements: [], total: 0 };
    // 单次全量遍历：iframes / open shadow roots / 可交互元素 一次扫描完成，
    // 避免原先"交互元素选择器 + 全量 shadow 遍历"两次全扫（无 shadow 页面省一半 DOM 迭代）。
    var all = doc.querySelectorAll('*');
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      if (el.tagName === 'IFRAME') {
        var r = el.getBoundingClientRect();
        var fr = { url: el.src || '', name: el.name || '', cross_origin: false, elements: [], meta: null };
        try {
          var inner = el.contentDocument;
          if (inner) {
            var sub = collect(inner, ox + r.left, oy + r.top);
            fr.elements = sub.elements;
            fr.meta = sub.meta;
          } else {
            fr.cross_origin = true;
          }
        } catch (e) {
          fr.cross_origin = true;
        }
        frames.push(fr);
        continue;
      }
      if (el.shadowRoot) {
        var sh = collect(el.shadowRoot, ox, oy); // shadow 与所在文档同坐标系
        res.elements = res.elements.concat(sh.elements);
        res.total += sh.total;
        grandTotal += sh.total;
        if (sh.total > 0 && res.total > MAX) truncated = true;
      }
      if (!isInteractive(el)) continue;
      if (!visible(el)) continue;
      res.total++;
      grandTotal++;
      if (res.total > MAX || grandTotal > MAX) {
        truncated = true;
        continue;
      }
      var id = String.fromCharCode(idc++);
      el.setAttribute('data-fb', id);
      var rect = el.getBoundingClientRect();
      var text = '';
      if (el.innerText) text = el.innerText;
      else if (el.value !== undefined) text = el.value;
      else if (el.textContent) text = el.textContent;
      res.elements.push({
        id: id,
        tag: el.tagName.toLowerCase(),
        role: el.getAttribute('role'),
        text: (text || '').trim().slice(0, 200),
        href: el.getAttribute('href'),
        rect: { x: rect.x + ox, y: rect.y + oy, width: rect.width, height: rect.height },
        value: (el.value !== undefined) ? el.value : null,
        input_type: el.getAttribute('type'),
        checked: (el.checked !== undefined) ? el.checked : null,
        disabled: (el.disabled === true) || (el.getAttribute('aria-disabled') === 'true'),
        placeholder: el.getAttribute('placeholder'),
        name: el.getAttribute('name'),
        aria_label: el.getAttribute('aria-label'),
        visible: true
      });
    }
    res.meta = { total: res.total, truncated: truncated, viewport_h: 0, scroll_h: 0, scroll_y: 0 };
    return res;
  }

  var top = collect(document, 0, 0);
  var doc = document.documentElement || document.body;
  return JSON.stringify({
    elements: top.elements,
    meta: {
      total: grandTotal,
      truncated: truncated,
      viewport_h: window.innerHeight || (doc ? doc.clientHeight : 0) || 0,
      scroll_h: doc ? (doc.scrollHeight || 0) : 0,
      scroll_y: window.scrollY || 0
    },
    frames: frames
  });
})()"#
}

/// 解析注入 JS 的返回（`{elements, meta, frames}` 对象，兼容旧版裸数组），
/// 生成顶层元素 + 元信息 + 子框架快照。
/// 供 cdp / webview 引擎共用，保证两引擎快照语义一致。
pub fn parse_snapshot_value(
    raw: &Value,
) -> Result<(Vec<InteractiveElement>, SnapshotMeta, Vec<FrameSnapshot>)> {
    let json_str = raw
        .as_str()
        .ok_or_else(|| EngineError::new(ErrorKind::Snapshot, "extract returned non-string"))?;
    let v: Value = serde_json::from_str(json_str)
        .map_err(|e| EngineError::new(ErrorKind::Snapshot, format!("extract json: {e}")))?;

    let elements = v.get("elements").cloned().unwrap_or_else(|| v.clone()); // 兼容旧版裸数组
    let arr: Vec<Value> = elements.as_array().cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for el in arr {
        out.push(parse_element(&el));
    }

    let meta_raw = v
        .get("meta")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let num = |k: &str| meta_raw.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    let meta = SnapshotMeta {
        total: num("total"),
        truncated: meta_raw
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        viewport_h: num("viewport_h") as u32,
        scroll_h: num("scroll_h") as u32,
        scroll_y: num("scroll_y") as u32,
        stale: false,
    };

    let mut frames = Vec::new();
    if let Some(farr) = v.get("frames").and_then(Value::as_array) {
        for f in farr {
            let felems: Vec<InteractiveElement> = f
                .get("elements")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(parse_element).collect())
                .unwrap_or_default();
            let fmeta = f
                .get("meta")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            let total = fmeta.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
            let ftrunc = fmeta
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            frames.push(FrameSnapshot {
                url: f
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                name: f
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                interactive: felems,
                text: None,
                cross_origin: f
                    .get("cross_origin")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                total,
                truncated: ftrunc,
            });
        }
    }
    Ok((out, meta, frames))
}

fn parse_element(el: &Value) -> InteractiveElement {
    let id = el
        .get("id")
        .and_then(Value::as_str)
        .and_then(|s| s.chars().next())
        .unwrap_or('?');
    let rect = el.get("rect").cloned().unwrap_or(Value::Null);
    let mut attrs = HashMap::new();
    for key in ["name", "placeholder", "aria_label"] {
        if let Some(s) = el.get(key).and_then(Value::as_str) {
            attrs.insert(key.to_string(), s.to_string());
        }
    }
    if el.get("disabled").and_then(Value::as_bool) == Some(true) {
        attrs.insert("disabled".to_string(), "true".to_string());
    }
    InteractiveElement {
        id,
        tag: el
            .get("tag")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        role: el.get("role").and_then(Value::as_str).map(String::from),
        text: el.get("text").and_then(Value::as_str).map(String::from),
        href: el.get("href").and_then(Value::as_str).map(String::from),
        rect: Rect {
            x: rect.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            y: rect.get("y").and_then(Value::as_f64).unwrap_or(0.0),
            width: rect.get("width").and_then(Value::as_f64).unwrap_or(0.0),
            height: rect.get("height").and_then(Value::as_f64).unwrap_or(0.0),
        },
        refs: vec![ElementRef::snapshot(id)],
        attrs,
        value: el.get("value").and_then(Value::as_str).map(String::from),
        input_type: el
            .get("input_type")
            .and_then(Value::as_str)
            .map(String::from),
        checked: el.get("checked").and_then(Value::as_bool),
        selectable_options: None,
        selected_option: None,
        visible: el.get("visible").and_then(Value::as_bool).unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_v3_with_frames() {
        let raw = json!(
            r#"{
          "elements":[{"id":"a","tag":"a","text":"Top","rect":{"x":0,"y":0,"width":1,"height":1},"visible":true}],
          "meta":{"total":2,"truncated":false,"viewport_h":800,"scroll_h":1200,"scroll_y":0},
          "frames":[{"url":"https://x/iframe","name":"f1","cross_origin":false,"elements":[{"id":"b","tag":"button","text":"Inner","rect":{"x":10,"y":20,"width":5,"height":5},"visible":true}],"meta":{"total":1,"truncated":false}}]
        }"#
        );
        let (els, meta, frames) = parse_snapshot_value(&raw).unwrap();
        assert_eq!(els.len(), 1);
        assert_eq!(els[0].id, 'a');
        assert_eq!(meta.total, 2);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].url, "https://x/iframe");
        assert_eq!(frames[0].interactive[0].id, 'b');
        assert_eq!(frames[0].interactive[0].rect.x, 10.0);
    }

    #[test]
    fn parses_legacy_array() {
        let raw = json!(
            r#"[{"id":"b","tag":"button","text":"Go","rect":{"x":0,"y":0,"width":1,"height":1},"visible":true}]"#
        );
        let (els, meta, _frames) = parse_snapshot_value(&raw).unwrap();
        assert_eq!(els.len(), 1);
        assert_eq!(els[0].id, 'b');
        assert_eq!(meta.total, 0);
    }

    #[test]
    fn parses_cross_origin_frame() {
        let raw = json!(
            r#"{
          "elements":[],
          "meta":{"total":0,"truncated":false},
          "frames":[{"url":"https://other","name":"x","cross_origin":true,"elements":[],"meta":null}]
        }"#
        );
        let (_els, _meta, frames) = parse_snapshot_value(&raw).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].cross_origin);
        assert!(frames[0].interactive.is_empty());
    }

    #[test]
    fn rejects_non_string() {
        assert!(parse_snapshot_value(&json!([1, 2])).is_err());
    }

    #[test]
    fn extract_js_shape() {
        let js = runtime_extract_js();
        assert!(js.starts_with("/*FB_EXTRACT_V3*/"));
        assert!(js.contains("frames"));
        assert!(js.contains("contentDocument"));
        assert!(js.contains("shadowRoot"));
        assert!(js.contains("meta"));
    }

    #[test]
    fn fb_find_and_action_js() {
        let find = fb_find_js();
        assert!(find.contains("__fbFind"));
        assert!(find.contains("shadowRoot"));
        let js = action_js('a', "el.click()");
        assert!(js.contains("__fbFind"));
        assert!(js.contains("[data-fb=\"a\"]"));
        assert!(js.contains("el.click()"));
    }
}
