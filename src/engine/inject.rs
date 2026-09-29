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
use crate::engine::{ElementRef, EngineError, ErrorKind, Rect, RefKind, Result};

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

/// Sentinel `data-fb` id for live-resolved (non-snapshot) targets — a control
/// char that cannot collide with snapshot ids (a..z).
pub const LIVE_ID: char = '\u{1}';

/// Playwright 选择器引擎（注入 JS）。
///
/// 支持子集（覆盖常见用法）：
/// - 链式 `A >> B`（B 在 A 的后代中查找）
/// - 段引擎前缀：`css=` / `text=` / `xpath=` / `id=` / `data-testid=`(`testid=`) / `role=`
/// - `role=button[name="X"]`（按可访问名过滤）
/// - CSS 段伪类：`:visible` / `:has-text("...")` / `:text("...")`
/// - `nth=N` 段（取当前结果第 N 个，0-based）
/// - `text=` 支持 `"exact"` 与 `/regex/`
///
/// 定义 `window.__fbQuery(sel)`，返回首个匹配元素或 `null`。
pub fn fb_query_js() -> &'static str {
    r#"window.__fbQueryAll=function(sel){
  function vis(el){if(!el||!el.getClientRects||el.getClientRects().length===0)return false;var cs=window.getComputedStyle(el);return !!cs&&cs.visibility!=='hidden'&&cs.display!=='none';}
  function txt(el){return ((el.innerText!==undefined?el.innerText:el.textContent)||'').trim();}
  function norm(s){return (s||'').replace(/\s+/g,' ').trim();}
  function unquote(s){s=(s||'').trim();if(s.length>=2&&((s[0]==='"'&&s[s.length-1]==='"')||(s[0]==="'"&&s[s.length-1]==="'")))return {v:s.slice(1,-1),exact:true};if(s.length>=2&&s[0]==='/'&&s[s.length-1]==='/'){try{return {re:new RegExp(s.slice(1,-1)),exact:false};}catch(e){}}return {v:s,exact:false};}
  function matchText(el,q){var t=norm(txt(el));if(q.re)return q.re.test(t);return q.exact?t===q.v:t.toLowerCase().indexOf((q.v||'').toLowerCase())>=0;}
  function dedupe(a){var s=[];for(var i=0;i<a.length;i++)if(a[i]&&s.indexOf(a[i])<0)s.push(a[i]);return s;}
  function stripPseudos(css){
    var f=[];
    css=css.replace(/:visible\b/g,function(){f.push({t:'visible'});return '';});
    css=css.replace(/:has-text\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\s*\)/g,function(m,a,b,c){f.push({t:'text',q:unquote(a!==undefined?a:(b!==undefined?b:c))});return '';});
    css=css.replace(/:text\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\s*\)/g,function(m,a,b,c){f.push({t:'text',q:unquote(a!==undefined?a:(b!==undefined?b:c))});return '';});
    return {css:css.trim(),f:f};
  }
  function applyFilters(list,f){for(var i=0;i<f.length;i++){var x=f[i];list=list.filter(function(el){if(x.t==='visible')return vis(el);if(x.t==='text')return matchText(el,x.q);return true;});}return list;}
  function desc(root){return Array.prototype.slice.call(root.querySelectorAll('*'));}
  var IMP={a:'link',button:'button',select:'combobox',textarea:'textbox',img:'img',
    h1:'heading',h2:'heading',h3:'heading',h4:'heading',h5:'heading',h6:'heading',
    nav:'navigation',main:'main',header:'banner',footer:'contentinfo',aside:'complementary',
    form:'form',table:'table',tr:'row',td:'cell',th:'columnheader',caption:'caption',
    thead:'rowgroup',tbody:'rowgroup',tfoot:'rowgroup',ul:'list',ol:'list',menu:'list',
    li:'listitem',dl:'list',dt:'term',dd:'definition',p:'paragraph',dialog:'dialog',
    summary:'button',details:'group',fieldset:'group',legend:'legend',option:'option',
    optgroup:'group',progress:'progressbar',meter:'meter',output:'status',figure:'figure',
    figcaption:'caption',blockquote:'blockquote',code:'code',pre:'code',strong:'strong',
    em:'emphasis',mark:'mark',time:'time',hr:'separator',article:'article',section:'region',
    address:'group',video:'video',audio:'audio',canvas:'img',svg:'img',iframe:'iframe'};
  function implicitRole(el){
    var r=el.getAttribute&&el.getAttribute('role'); if(r)return r.trim().split(/\s+/)[0];
    var tag=el.tagName?el.tagName.toLowerCase():'';
    if(tag==='input'){var t=(el.getAttribute('type')||'text').toLowerCase();
      if(t==='hidden')return ''; if(t==='checkbox')return 'checkbox'; if(t==='radio')return 'radio';
      if(t==='submit'||t==='button'||t==='reset'||t==='image')return 'button';
      if(t==='range')return 'slider'; if(t==='search')return 'searchbox'; if(t==='number')return 'spinbutton';
      return 'textbox';}
    if(tag==='a'&&!el.hasAttribute('href'))return '';
    return IMP[tag]||'';
  }
  function roleSyn(r){r=(r||'').toLowerCase().trim();
    var syn={image:'img','search box':'searchbox',input:'textbox','text box':'textbox',
      select:'combobox','list item':'listitem','radio button':'radio',nav:'navigation',
      header:'banner',footer:'contentinfo',aside:'complementary',a:'link'};
    return syn[r]||r;
  }
  function accName(el){
    var a=el.getAttribute&&el.getAttribute('aria-label'); if(a&&a.trim())return a.trim();
    var lb=el.getAttribute&&el.getAttribute('aria-labelledby');
    if(lb){var parts=lb.split(/\s+/).map(function(id){var e=document.getElementById(id);return e?(e.innerText||e.textContent||''):'';}).join(' ').replace(/\s+/g,' ').trim(); if(parts)return parts;}
    var tag=el.tagName?el.tagName.toLowerCase():'';
    if(tag==='img'){var alt=el.getAttribute('alt'); if(alt)return alt;}
    if(tag==='input'||tag==='textarea'||tag==='select'){
      if(el.labels&&el.labels.length){var s='';for(var i=0;i<el.labels.length;i++)s+=' '+(el.labels[i].innerText||el.labels[i].textContent||'');s=s.replace(/\s+/g,' ').trim();if(s)return s;}
      var ph=el.getAttribute('placeholder'); if(ph)return ph;
    }
    var t=el.getAttribute&&el.getAttribute('title'); if(t)return t;
    return norm(txt(el));
  }
  function boolEq(v,opt){return String(v)===String(norm(String(opt))==='true');}
  function roleMatches(el,want,opts){
    if(roleSyn(implicitRole(el))!==want)return false;
    if(opts.name!==undefined){var an=norm(accName(el)).toLowerCase(),q=norm(String(opts.name)).toLowerCase();if(opts.exact?an!==q:an.indexOf(q)<0)return false;}
    if(opts.checked!==undefined){var ck=(el.checked!==undefined)?!!el.checked:el.getAttribute('aria-checked')==='true';if(!boolEq(ck,opts.checked))return false;}
    if(opts.disabled!==undefined){var di=(el.disabled!==undefined)?!!el.disabled:el.getAttribute('aria-disabled')==='true';if(!boolEq(di,opts.disabled))return false;}
    if(opts.expanded!==undefined){var ex=el.getAttribute('aria-expanded')==='true';if(!boolEq(ex,opts.expanded))return false;}
    if(opts.selected!==undefined){var se=(el.selected!==undefined)?!!el.selected:el.getAttribute('aria-selected')==='true';if(!boolEq(se,opts.selected))return false;}
    if(opts.level!==undefined){var m=/^h([1-6])$/.exec(el.tagName.toLowerCase());var lv=parseInt(el.getAttribute('aria-level')||(m?m[1]:'0'),10);if(String(lv)!==String(parseInt(String(opts.level),10)))return false;}
    return true;
  }
  function seg(part,root){
    var engine='css',val=part;
    var m=part.match(/^(css|text|xpath|id|data-testid|testid|role|label|placeholder|alt|title|value|href|ref)=([\s\S]*)$/);
    if(m){engine=m[1];val=m[2];}
    var out=[];
    if(engine==='xpath'){
      var res=document.evaluate(val,root===document?document:root,null,XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,null);
      for(var j=0;j<res.snapshotLength;j++)out.push(res.snapshotItem(j));
    } else if(engine==='id'){
      var e=document.getElementById(val);if(e)out.push(e);
    } else if(engine==='data-testid'||engine==='testid'){
      var a=root.querySelectorAll('[data-testid="'+val.replace(/"/g,'\\"')+'"]');for(var j=0;j<a.length;j++)out.push(a[j]);
    } else if(engine==='role'){
      var opts={},optRe=/\[\s*([a-zA-Z_-]+)\s*(?:=\s*(?:"([^"]*)"|'([^']*)'|([^\]]*)))?\s*\]/g,mm;
      while((mm=optRe.exec(val))){opts[mm[1].toLowerCase()]=(mm[2]!==undefined?mm[2]:(mm[3]!==undefined?mm[3]:(mm[4]!==undefined?mm[4].trim():'true')));}
      var want=roleSyn(val.replace(/\[[^\]]*\]/g,'').trim());
      var cand=desc(root);if(root!==document)cand.unshift(root);
      for(var j=0;j<cand.length;j++){var el=cand[j];if(!el.tagName)continue;if(roleMatches(el,want,opts))out.push(el);}
    } else if(engine==='label'||engine==='placeholder'||engine==='alt'||engine==='title'||engine==='value'||engine==='href'){
      var q=unquote(val),cand=desc(root);if(root!==document)cand.unshift(root);
      for(var j=0;j<cand.length;j++){var el=cand[j];if(!el.tagName)continue;var field=null,tag=el.tagName.toLowerCase();
        if(engine==='label')field=accName(el);
        else if(engine==='placeholder')field=el.getAttribute('placeholder');
        else if(engine==='alt')field=el.getAttribute('alt');
        else if(engine==='title')field=el.getAttribute('title');
        else if(engine==='value')field=(el.value!==undefined?String(el.value):null);
        else if(engine==='href')field=el.getAttribute('href');
        if(field==null)continue;field=norm(field);
        if(q.re?q.re.test(field):(q.exact?field===q.v:field.toLowerCase().indexOf((q.v||'').toLowerCase())>=0))out.push(el);
      }
    } else if(engine==='ref'){
      var id=String(val).replace(/^@/,'').split(':').pop();
      var e=document.querySelector('[data-fb="'+id.replace(/"/g,'\\"')+'"]');if(e)out.push(e);
    } else if(engine==='text'){
      var q=unquote(val),cand=desc(root);
      for(var j=0;j<cand.length;j++)if(matchText(cand[j],q))out.push(cand[j]);
      out.sort(function(a,b){return a.querySelectorAll('*').length-b.querySelectorAll('*').length;});
      if(out.length)out=[out[0]];
    } else {
      var sp=stripPseudos(val),base=sp.css||'*';
      try{out=applyFilters(Array.prototype.slice.call(root.querySelectorAll(base)),sp.f);}catch(e){out=[];}
    }
    return out;
  }
  var parts=sel.split('>>').map(function(s){return s.trim();}).filter(Boolean);
  if(!parts.length)return [];
  var current=null;
  for(var i=0;i<parts.length;i++){
    var part=parts[i],nthM=part.match(/^nth=(\d+)$/);
    if(nthM){if(!current)return [];var _n=current[parseInt(nthM[1],10)];current=_n?[_n]:[];continue;}
    var roots=(current===null)?[document]:current,next=[];
    for(var r=0;r<roots.length;r++)next=next.concat(seg(part,roots[r]));
    current=dedupe(next);
    if(!current.length)return [];
  }
  return current||[];
};
window.__fbQuery=function(sel){var a=window.__fbQueryAll(sel);return (a&&a.length)?a[0]:null;};"#
}

/// JS that returns `true` when `sel` matches an element, else `false`
/// (`null` only when the engine cannot evaluate). Playwright selectors
/// (`role=`/`text=`/`xpath=`/`css=`) are supported via the shared `__fbQuery`.
pub fn element_exists_js(sel: &str) -> String {
    let engine = fb_query_js();
    let sel_json = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{{engine}try{{return !!(window.__fbQuery?window.__fbQuery({sel_json}):null);}}catch(e){{return null;}}}})()"
    )
}

/// JS that resolves a non-snapshot [`ElementRef`] with a **live DOM query**,
/// tags the element `data-fb=<LIVE_ID>`, and returns that char (or `''` when not
/// found). This lets `click`/`hover`/`get_element_info` accept CSS / XPath /
/// text / role / Playwright-selector on engines whose cached snapshot only
/// carries snapshot refs.
pub fn live_resolve_js(r: &ElementRef) -> String {
    let kind = match r.kind {
        RefKind::Css => "css",
        RefKind::Xpath => "xpath",
        RefKind::Text => "text",
        RefKind::Role => "role",
        RefKind::Snapshot => "snapshot",
        RefKind::Selector => "selector",
    };
    let kind = serde_json::to_string(kind).unwrap_or_else(|_| "\"\"".into());
    let val = serde_json::to_string(&r.value).unwrap_or_else(|_| "\"\"".into());
    // Always define `__fbQuery` so `text=` / `role=` / `selector` share ONE
    // resolution engine (consistent substring/regex + `role=...[name=...]`).
    let engine = fb_query_js();
    let id = LIVE_ID;
    format!(
        r#"(function(){{{engine}var kind={kind},value={val},el=null;
if(kind==='css'){{el=document.querySelector(value);}}
else if(kind==='xpath'){{var x=document.evaluate(value,document,null,7,null);el=x.snapshotItem(0);}}
else if(kind==='id'){{el=document.getElementById(value);}}
else if(kind==='snapshot'){{el=document.querySelector('[data-fb="'+String(value).replace(/"/g,'\\"')+'"]');}}
else if(kind==='data-testid'||kind==='testid'){{el=document.querySelector('[data-testid="'+String(value).replace(/"/g,'\\"')+'"]');}}
else if(kind==='text'){{el=window.__fbQuery?window.__fbQuery('text='+value):null;}}
else if(kind==='role'){{el=window.__fbQuery?window.__fbQuery('role='+value):null;}}
else if(kind==='selector'){{el=window.__fbQuery?window.__fbQuery(value):null;}}
if(!el){{var o=document.querySelector('[data-fb="{id}"]');if(o)o.removeAttribute('data-fb');return '';}}
var old=document.querySelector('[data-fb="{id}"]');if(old&&old!==el)old.removeAttribute('data-fb');
el.setAttribute('data-fb','{id}');return '{id}';}})()"#
    )
}

/// JS returning a JSON object with an element's key fields (located by
/// `data-fb` id), or `null` when not found. Used to synthesize an
/// `InteractiveElement` for a live-resolved (non-snapshot) target. Mirrors the
/// fields `parse_element` consumes so `get_attributes` / `get_element_info` /
/// `is_visible` / `is_enabled` behave the same for CSS/XPath/text/role targets
/// as for snapshot refs (previously `attrs` was always empty and `visible`
/// always `true`).
pub fn element_info_js(id: char) -> String {
    format!(
        r#"(function(){{var el=document.querySelector('[data-fb="{id}"]');if(!el)return null;var r=el.getBoundingClientRect();var cs=window.getComputedStyle(el);var attrs={{}};for(var i=0;i<el.attributes.length;i++){{var a=el.attributes[i];attrs[a.name]=a.value;}}var vis=!!(el.getClientRects().length)&&cs.visibility!=='hidden'&&cs.display!=='none';var tag=el.tagName.toLowerCase();return {{id:el.getAttribute('id'),tag:tag,role:el.getAttribute('role'),text:(el.innerText||el.textContent||'').trim().slice(0,200),href:el.getAttribute('href'),name:el.getAttribute('name'),placeholder:el.getAttribute('placeholder'),aria_label:el.getAttribute('aria-label'),rect:{{x:r.x,y:r.y,width:r.width,height:r.height}},value:(el.value!==undefined?el.value:null),input_type:(tag==='input'||tag==='button'||tag==='select'||tag==='textarea'?(el.getAttribute('type')||(tag==='input'?'text':null)):el.getAttribute('type')),checked:(el.checked!==undefined?el.checked:null),disabled:(el.disabled!==undefined?el.disabled:null),visible:vis,attrs:attrs}};}})()"#
    )
}

/// JS returning `[{text, url}]` for links (`a[href]`) inside the live-resolved
/// element, or `[]` when it isn't found. Used by `extract_links` element
/// scoping.
pub fn element_links_js() -> String {
    format!(
        r#"(function(){{var el=document.querySelector('[data-fb="{id}"]');if(!el)return [];var out=[],as=el.querySelectorAll('a[href]');for(var i=0;i<as.length;i++){{out.push({{text:(as[i].innerText||as[i].textContent||'').trim(),url:as[i].href}});}}return out;}})()"#,
        id = LIVE_ID
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

/// Injected JS that builds an accessibility tree from the DOM/ARIA: role +
/// accessible name + state + geometry, recursively from `<body>`. Returns a
/// JSON string `{root, count, truncated}` (the same shape the CDP engine's
/// `accessibility_tree` produces, so mobile WebView hosts get AX too).
pub fn accessibility_js() -> &'static str {
    r#"(() => {
  const MAX = 3000;
  const MAX_DEPTH = 60;
  let count = 0, truncated = false;
  const IMPLICIT = {
    a:'link',button:'button',select:'combobox',textarea:'textbox',img:'img',
    h1:'heading',h2:'heading',h3:'heading',h4:'heading',h5:'heading',h6:'heading',
    nav:'navigation',main:'main',header:'banner',footer:'contentinfo',aside:'complementary',
    form:'form',table:'table',tr:'row',td:'cell',th:'columnheader',caption:'caption',
    thead:'rowgroup',tbody:'rowgroup',tfoot:'rowgroup',
    ul:'list',ol:'list',menu:'list',li:'listitem',dl:'list',dt:'term',dd:'definition',
    p:'paragraph',dialog:'dialog',summary:'button',details:'group',fieldset:'group',
    legend:'legend',option:'option',optgroup:'group',progress:'progressbar',meter:'meter',
    output:'status',figure:'figure',figcaption:'caption',blockquote:'blockquote',
    code:'code',pre:'code',strong:'strong',em:'emphasis',mark:'mark',time:'time',
    hr:'separator',article:'article',section:'region',address:'group',
    video:'video',audio:'audio',canvas:'img',svg:'img',iframe:'iframe'
  };
  const INTERACTIVE = {link:1,button:1,textbox:1,checkbox:1,radio:1,combobox:1,
    searchbox:1,slider:1,menuitem:1,tab:1,switch:1,option:1,spinbutton:1};
  // Non-rendered elements whose textContent is not accessible content (CSS/JS
  // leaks as `name`, e.g. an inline <style> inside an <svg>).
  const SKIP = {script:1,style:1,template:1,noscript:1,head:1,meta:1,link:1,title:1,base:1};
  const tagOf = (el) => (el.tagName ? el.tagName.toLowerCase() : '');
  const roleOf = (el) => {
    const r = el.getAttribute && el.getAttribute('role');
    if (r) return r.trim().split(/\s+/)[0];
    const tag = tagOf(el);
    if (tag === 'input') {
      const t = (el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'hidden') return '';           // never part of the AX tree
      if (t === 'checkbox') return 'checkbox';
      if (t === 'radio') return 'radio';
      if (t === 'submit' || t === 'button' || t === 'reset' || t === 'image') return 'button';
      if (t === 'range') return 'slider';
      if (t === 'search') return 'searchbox';
      if (t === 'number') return 'spinbutton';
      return 'textbox';
    }
    if (tag === 'a' && !el.hasAttribute('href')) return '';
    return IMPLICIT[tag] || '';
  };
  // Only fall back to text for leaf-ish nodes; containers would otherwise
  // report their entire descendant text as the accessible name.
  const nameOf = (el, hasKids) => {
    const a = el.getAttribute && el.getAttribute('aria-label');
    if (a && a.trim()) return a.trim();
    const lb = el.getAttribute && el.getAttribute('aria-labelledby');
    if (lb) {
      const parts = lb.split(/\s+/).map((id) => {
        const e = document.getElementById(id);
        return e ? (e.innerText || e.textContent || '') : '';
      }).join(' ').replace(/\s+/g, ' ').trim();
      if (parts) return parts;
    }
    const tag = tagOf(el);
    if (tag === 'img') { const alt = el.getAttribute('alt'); if (alt) return alt; }
    if (tag === 'input' || tag === 'textarea') {
      const ph = el.getAttribute('placeholder'); if (ph) return ph;
    }
    const t = el.getAttribute && el.getAttribute('title');
    if (t) return t;
    if (hasKids) return '';
    const txt = (el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim();
    return txt.length > 120 ? txt.slice(0, 120) : txt;
  };
  const isHidden = (el) => {
    if (el.getAttribute && el.getAttribute('aria-hidden') === 'true') return true;
    if (el.hidden) return true;
    try {
      const s = getComputedStyle(el);
      if (s && (s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse')) return true;
      if (s && s.opacity !== '' && parseFloat(s.opacity) === 0) return true;
    } catch (e) {}
    return false;
  };
  const propsOf = (el) => {
    const p = {};
    if (typeof el.checked === 'boolean') p.checked = el.checked;
    if (typeof el.disabled === 'boolean' && el.disabled) p.disabled = true;
    if (typeof el.required === 'boolean' && el.required) p.required = true;
    if (typeof el.readOnly === 'boolean' && el.readOnly) p.editable = false;
    if (el.isContentEditable) p.editable = true;
    const tag = tagOf(el);
    const lv = el.getAttribute && el.getAttribute('aria-level');
    if (lv) { const n = parseInt(lv, 10); if (!isNaN(n)) p.level = n; }
    else { const m = /^h([1-6])$/.exec(tag); if (m) p.level = parseInt(m[1], 10); }
    const attr = (k) => (el.getAttribute ? el.getAttribute(k) : null);
    const ex = attr('aria-expanded'); if (ex !== null) p.expanded = ex === 'true';
    const se = attr('aria-selected'); if (se !== null) p.selected = se === 'true';
    const bu = attr('aria-busy'); if (bu !== null) p.busy = bu === 'true';
    const inv = attr('aria-invalid'); if (inv !== null && inv !== 'false') p.invalid = true;
    try { if (document.activeElement === el) p.focused = true; } catch (e) {}
    return p;
  };
  const geoOf = (el) => {
    try { const r = el.getBoundingClientRect(); return {x: r.x, y: r.y, width: r.width, height: r.height}; }
    catch (e) { return null; }
  };
  // Same-document children + open shadow roots + same-origin iframe bodies.
  const childrenOf = (el) => {
    const out = [];
    if (el.children) for (let i = 0; i < el.children.length; i++) out.push(el.children[i]);
    if (el.shadowRoot && el.shadowRoot.children) {
      for (let i = 0; i < el.shadowRoot.children.length; i++) out.push(el.shadowRoot.children[i]);
    }
    if (tagOf(el) === 'iframe') {
      try { const d = el.contentDocument; if (d && d.body) out.push(d.body); } catch (e) {}
    }
    return out;
  };
  const walk = (el, depth) => {
    if (!el || el.nodeType !== 1) return null;
    if (SKIP[tagOf(el)]) return null;
    if (depth > MAX_DEPTH) return null;
    if (isHidden(el)) return null;
    if (count >= MAX) { truncated = true; return null; }
    const role = roleOf(el);
    const kids = [];
    const cs = childrenOf(el);
    for (let i = 0; i < cs.length; i++) {
      if (count >= MAX) { truncated = true; break; }
      const c = walk(cs[i], depth + 1);
      if (c) kids.push(c);
    }
    const name = nameOf(el, kids.length > 0);
    // Drop pure-noise nodes (no role, no name, nothing interesting inside).
    if (!role && !name && kids.length === 0) return null;
    count++;
    const node = {role: role, name: name};
    const v = (el.value !== undefined && el.value !== null) ? String(el.value) : '';
    if (v) node.value = v;
    const pr = propsOf(el); if (Object.keys(pr).length) node.props = pr;
    const g = geoOf(el); if (g) node.geometry = g;
    if (kids.length) node.children = kids;
    return node;
  };
  const root = walk(document.body || document.documentElement, 0);
  return JSON.stringify({root: root, count: count, truncated: truncated});
})()"#
}

/// Fallback text insertion into the **currently focused** element via JS.
///
/// Used when an engine has no real keyboard channel (`type_text_focused` is
/// unsupported, e.g. the mobile WebView bridge): after a click focuses the
/// target, this sets the value through the native prototype setter (so
/// framework/controlled inputs see the change) and dispatches `input`/`change`.
/// `clear` replaces the value; otherwise the text is appended. Returns a JS
/// expression that yields `'ok'` or `'noinput'`.
pub fn focused_text_js(text: &str, clear: bool) -> String {
    let val = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    let clear_lit = if clear { "true" } else { "false" };
    format!(
        r#"(function(){{
  var el = document.activeElement;
  if (!el) return 'noinput';
  function setNative(node, v) {{
    try {{
      var proto = (node.tagName === 'TEXTAREA') ? window.HTMLTextAreaElement.prototype
                                                : window.HTMLInputElement.prototype;
      var d = Object.getOwnPropertyDescriptor(proto, 'value');
      if (d && d.set) {{ d.set.call(node, v); return; }}
    }} catch (e) {{}}
    node.value = v;
  }}
  var clear = {clear};
  if (el.isContentEditable) {{
    var cur = clear ? '' : (el.textContent || '');
    el.textContent = cur + {val};
    el.dispatchEvent(new Event('input', {{bubbles: true}}));
    return 'ok';
  }}
  if ('value' in el) {{
    var cur2 = clear ? '' : (el.value || '');
    setNative(el, cur2 + {val});
    el.dispatchEvent(new Event('input', {{bubbles: true}}));
    el.dispatchEvent(new Event('change', {{bubbles: true}}));
    return 'ok';
  }}
  return 'noinput';
}})()"#,
        clear = clear_lit,
        val = val,
    )
}

/// Focus the element at viewport point `(x,y)` (or its nearest
/// focusable/editable ancestor/label). Synthetic mouse clicks do **not** move
/// focus, so AX `focus` (and text actions) must use this instead of clicking.
pub fn focus_at_point_js(x: f64, y: f64) -> String {
    r#"(function(){
  var el=document.elementFromPoint(__X__,__Y__); if(!el) return 'notfound';
  var t=el.closest('input,textarea,select,[contenteditable=""],[contenteditable="true"],[tabindex],button,a')||el;
  try{ t.focus({preventScroll:true}); }catch(e){ try{ t.focus(); }catch(_){} }
  return (document.activeElement===t || (t.contains && t.contains(document.activeElement)))?'ok':'nofocus';
})()"#
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
}

/// Insert `text` into the editable element at viewport point `(x,y)` (falling
/// back to the focused element / a wrapping `<label>`). Uses the native value
/// setter so framework/controlled inputs react. Returns `'ok'`/`'noinput'`.
pub fn set_text_at_point_js(x: f64, y: f64, text: &str, clear: bool) -> String {
    let val = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    r#"(function(){
  var EDIT='input,textarea,[contenteditable=""],[contenteditable="true"]';
  function matches(n){ try{ return n && n.matches && n.matches(EDIT); }catch(e){ return false; } }
  var el=document.elementFromPoint(__X__,__Y__), t=null;
  function find(n){ if(!n) return null; if(matches(n)) return n; try{ return n.closest(EDIT)||null; }catch(e){ return null; } }
  t=find(el);
  if(!t){ var a=document.activeElement; if(a && a!==document.body && matches(a)) t=a; }
  if(!t && el){ try{ var lab=el.closest('label'); if(lab){ var inp=lab.querySelector('input,textarea'); if(matches(inp)) t=inp; } }catch(e){} }
  if(!t) return 'noinput';
  try{ t.focus({preventScroll:true}); }catch(e){}
  var clear=__CLEAR__, v=__VAL__;
  if(t.isContentEditable){ var cur=clear?'':(t.textContent||''); t.textContent=cur+v; t.dispatchEvent(new Event('input',{bubbles:true})); return 'ok'; }
  if('value' in t){
    function set(node,x){ try{ var proto=(node.tagName==='TEXTAREA')?window.HTMLTextAreaElement.prototype:window.HTMLInputElement.prototype; var d=Object.getOwnPropertyDescriptor(proto,'value'); if(d&&d.set){d.set.call(node,x);return;} }catch(e){} node.value=x; }
    var cur2=clear?'':(t.value||''); set(t, cur2+v);
    t.dispatchEvent(new Event('input',{bubbles:true})); t.dispatchEvent(new Event('change',{bubbles:true}));
    return 'ok';
  }
  return 'noinput';
})()"#
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__CLEAR__", if clear { "true" } else { "false" })
        .replace("__VAL__", &val)
}

/// Select the option `value` on the `<select>` at viewport point `(x,y)`,
/// matched by option value / visible text / label. Returns
/// `'ok'`/`'nosel'`/`'nomatch'`/`'notfound'`.
pub fn select_at_point_js(x: f64, y: f64, value: &str) -> String {
    let val = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    r#"(function(){
  var el=document.elementFromPoint(__X__,__Y__); if(!el) return 'notfound';
  var sel; try{ sel=(el.matches&&el.matches('select'))?el:el.closest('select'); }catch(e){ sel=null; }
  if(!sel) return 'nosel';
  var v=__VAL__, opts=sel.options||[], hit=null;
  for(var i=0;i<opts.length;i++){ var o=opts[i]; if(o.value===v || ((o.text||'').trim())===v || o.label===v){ hit=o; break; } }
  if(!hit) return 'nomatch';
  sel.value=hit.value; sel.dispatchEvent(new Event('input',{bubbles:true})); sel.dispatchEvent(new Event('change',{bubbles:true}));
  return 'ok';
})()"#
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__VAL__", &val)
}

/// Set the checked state of the checkbox/radio (or `[role=checkbox]`) at
/// viewport point `(x,y)` to `checked`. Returns `'ok'`/`'noinput'`/`'notfound'`.
pub fn check_at_point_js(x: f64, y: f64, checked: bool) -> String {
    r#"(function(){
  var el=document.elementFromPoint(__X__,__Y__); if(!el) return 'notfound';
  var want=__WANT__;
  var c=null; try{ c=(el.matches&&el.matches('input[type=checkbox],input[type=radio]'))?el:el.closest('input[type=checkbox],input[type=radio]'); }catch(e){}
  if(c){ if(!!c.checked!==want){ try{ c.click(); }catch(e){ c.checked=want; c.dispatchEvent(new Event('change',{bubbles:true})); } } return 'ok'; }
  var r=null; try{ r=el.closest('[role=checkbox],[role=switch]'); }catch(e){}
  if(r){ if((r.getAttribute('aria-checked')==='true')!==want){ try{ r.click(); }catch(e){} } return 'ok'; }
  return 'noinput';
})()"#
        .replace("__X__", &x.to_string())
        .replace("__Y__", &y.to_string())
        .replace("__WANT__", if checked { "true" } else { "false" })
}

/// Idempotent console interceptor: records `console.*` calls into
/// `window.__fbConsoleLogs` so `get_console_logs` works on engines without
/// native console events (the webview engine). Original console methods still
/// run, so page behaviour is unchanged.
pub fn console_capture_js() -> &'static str {
    r#"(function(){
  if (window.__fbConsoleInstalled) return;
  window.__fbConsoleInstalled = true;
  window.__fbConsoleLogs = window.__fbConsoleLogs || [];
  var levels = ['log','info','warn','error','debug'];
  for (var i=0;i<levels.length;i++){
    (function(level){
      var orig = (console && console[level]) ? console[level].bind(console) : function(){};
      console[level] = function(){
        try {
          var parts = [];
          for (var a=0;a<arguments.length;a++){
            var x = arguments[a];
            if (typeof x === 'string') { parts.push(x); }
            else { try { parts.push(JSON.stringify(x)); } catch(e) { parts.push(String(x)); } }
          }
          window.__fbConsoleLogs.push({level: level==='warn'?'warning':level, message: parts.join(' ')});
          if (window.__fbConsoleLogs.length > 500) window.__fbConsoleLogs.shift();
        } catch(e){}
        return orig.apply(null, arguments);
      };
    })(levels[i]);
  }
})()"#
}

/// Synthesize DOM input events for engines that cannot deliver trusted native
/// input (the mobile WebView bridges expose a no-op `dispatch_event`). Returns a
/// JS expression that dispatches the matching KeyboardEvent / MouseEvent /
/// WheelEvent / TouchEvent on the element under the coordinates (or the focused
/// element for keys). Returns `None` when there is nothing to dispatch.
pub fn input_event_js(ev: &crate::engine::InputEvent) -> Option<String> {
    use crate::engine::{InputEvent, KeyKind, MouseButton, MouseKind, TouchKind};
    let jstr = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
    match ev {
        InputEvent::Key(e) => {
            let key = jstr(&e.key);
            let code = jstr(if e.code.is_empty() { &e.key } else { &e.code });
            let text = jstr(&e.text);
            let (ctrl, shift, alt, meta) = (
                e.modifiers.ctrl,
                e.modifiers.shift,
                e.modifiers.alt,
                e.modifiers.meta,
            );
            let make = format!(
                "var __el=document.activeElement||document.body;\
var __mk=function(t){{var o={{bubbles:true,cancelable:true,key:{key},code:{code},\
ctrlKey:{ctrl},shiftKey:{shift},altKey:{alt},metaKey:{meta}}};try{{return new KeyboardEvent(t,o);}}\
catch(e){{var ev=document.createEvent('Event');ev.initEvent(t,true,true);return ev;}}}};"
            );
            let mut body = String::from(make);
            match e.kind {
                KeyKind::Down => {
                    body.push_str("try{__el.dispatchEvent(__mk('keydown'));}catch(e){}");
                }
                KeyKind::Up => {
                    body.push_str("try{__el.dispatchEvent(__mk('keyup'));}catch(e){}");
                }
                KeyKind::Press => {
                    body.push_str(
                        "try{__el.dispatchEvent(__mk('keydown'));}catch(e){}\
try{var __pe=__mk('keypress');try{var __t=",
                    );
                    body.push_str(&text);
                    body.push_str(
                        ";if(typeof __t==='string'&&__t.length===1){__pe.charCode=__t.charCodeAt(0);}}catch(e){}\
__el.dispatchEvent(__pe);}catch(e){}\
try{__el.dispatchEvent(__mk('keyup'));}catch(e){}",
                    );
                }
            }
            body.push_str("return true;");
            Some(format!("(function(){{{body}}})()"))
        }
        InputEvent::Mouse(e) => {
            let (x, y) = (e.x, e.y);
            let button = match e.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                MouseButton::None => 0,
            };
            let detail = e.click_count.max(1);
            let (ctrl, shift, alt, meta) = (
                e.modifiers.ctrl,
                e.modifiers.shift,
                e.modifiers.alt,
                e.modifiers.meta,
            );
            let mk = |t: &str| {
                format!(
                    "try{{__t.dispatchEvent(new MouseEvent('{t}',{{bubbles:true,cancelable:true,\
view:window,clientX:{x},clientY:{y},button:{button},detail:{detail},\
ctrlKey:{ctrl},shiftKey:{shift},altKey:{alt},metaKey:{meta}}}));}}catch(e){{}}"
                )
            };
            let body = match e.kind {
                MouseKind::Move => mk("mousemove"),
                MouseKind::Down => mk("mousedown"),
                MouseKind::Up => mk("mouseup"),
                MouseKind::Click => format!("{}{}{}", mk("mousedown"), mk("mouseup"), mk("click")),
                MouseKind::DoubleClick => {
                    format!("{}{}", mk("dblclick"), mk("click"))
                }
            };
            Some(format!(
                "(function(){{var __t=document.elementFromPoint({x},{y})||document.body;{body}return true;}})()"
            ))
        }
        InputEvent::Wheel(e) => {
            let (x, y, dx, dy) = (e.x, e.y, e.delta_x, e.delta_y);
            Some(format!(
                "(function(){{var __t=document.elementFromPoint({x},{y})||document.body;\
try{{__t.dispatchEvent(new WheelEvent('wheel',{{bubbles:true,cancelable:true,\
clientX:{x},clientY:{y},deltaX:{dx},deltaY:{dy}}}));}}catch(e){{}}return true;}})()"
            ))
        }
        InputEvent::Touch(e) => {
            let first = e
                .points
                .first()
                .copied()
                .unwrap_or(crate::engine::TouchPoint {
                    id: 0,
                    x: 0.0,
                    y: 0.0,
                });
            let last = e.points.last().copied().unwrap_or(first);
            let (x0, y0) = (first.x, first.y);
            let (x1, y1) = (last.x, last.y);
            let make = |t: &str, x: f64, y: f64| {
                format!(
                    "try{{var __tp={{identifier:1,target:__t,clientX:{x},clientY:{y}}};\
var __tt=window.Touch?new Touch(__tp):null;\
var __to={{bubbles:true,cancelable:true,changedTouches:__tt?[__tt]:[],touches:\\\"{t}\\\"==='touchend'?[]:(__tt?[__tt]:[])}};\
__t.dispatchEvent(new TouchEvent('{t}',__to));}}catch(e){{}}"
                )
            };
            let seq = match e.kind {
                TouchKind::Start => make("touchstart", x0, y0),
                TouchKind::End => make("touchend", x1, y1),
                TouchKind::Cancel => make("touchcancel", x1, y1),
                TouchKind::Move => make("touchmove", x1, y1),
                TouchKind::Swipe => format!(
                    "{}{}{}",
                    make("touchstart", x0, y0),
                    make("touchmove", x1, y1),
                    make("touchend", x1, y1)
                ),
            };
            // Also drive mouse drag / click so mouse-only handlers react.
            let mouse = format!(
                "try{{__t.dispatchEvent(new MouseEvent('mousedown',{{bubbles:true,cancelable:true,clientX:{x0},clientY:{y0}}}));\
__t.dispatchEvent(new MouseEvent('mousemove',{{bubbles:true,cancelable:true,clientX:{x1},clientY:{y1}}}));\
__t.dispatchEvent(new MouseEvent('mouseup',{{bubbles:true,cancelable:true,clientX:{x1},clientY:{y1}}}));}}catch(e){{}}"
            );
            Some(format!(
                "(function(){{var __t=document.elementFromPoint({x0},{y0})||document.body;{seq}{mouse}return true;}})()"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fb_query_all_returns_empty_array_on_no_match() {
        // `find_elements` iterates __fbQueryAll; it must never be null.
        let js = fb_query_js();
        assert!(
            js.contains("if(!parts.length)return [];"),
            "empty selector → []"
        );
        assert!(
            js.contains("if(!current.length)return [];"),
            "no match → []"
        );
        assert!(js.contains("return current||[];"), "always an array");
    }

    #[test]
    fn element_exists_js_uses_shared_engine() {
        let js = element_exists_js("role=button[name=\"搜索\"]");
        assert!(js.contains("window.__fbQuery"), "routes through __fbQuery");
        assert!(js.contains("role=button[name="), "selector passed through");
    }

    #[test]
    fn fb_query_exposes_all() {
        let js = fb_query_js();
        assert!(
            js.contains("window.__fbQueryAll=function"),
            "must expose __fbQueryAll"
        );
        assert!(
            js.contains("window.__fbQuery=function"),
            "must expose __fbQuery"
        );
        // role=...[name="..."] + other ARIA option filters live in the shared engine.
        assert!(js.contains("opts.name"), "role name filter");
        assert!(js.contains("roleMatches"), "role option filtering");
    }

    #[test]
    fn live_resolve_clears_stale_marker_and_routes_through_query() {
        let js = live_resolve_js(&ElementRef {
            kind: RefKind::Text,
            value: "x".into(),
        });
        // On failure the stale data-fb marker MUST be removed, otherwise a later
        // element_info_js(LIVE_ID) returns the previous (wrong) element.
        assert!(js.contains("if(!el){var o=document.querySelector"));
        assert!(js.contains("o.removeAttribute('data-fb')"));
        // text / role / selector share the one resolution engine.
        assert!(
            js.contains("__fbQuery('text='+value)"),
            "text routes via __fbQuery"
        );
        let js_role = live_resolve_js(&ElementRef {
            kind: RefKind::Role,
            value: "navigation".into(),
        });
        assert!(
            js_role.contains("__fbQuery('role='+value)"),
            "role routes via __fbQuery"
        );
    }

    #[test]
    fn element_info_js_returns_attrs_and_visibility() {
        let js = element_info_js(LIVE_ID);
        // Must mirror parse_element's fields so get_attributes / get_element_info
        // / is_visible / is_enabled behave the same for live-resolved targets.
        assert!(js.contains("el.attributes"), "collects all attributes");
        assert!(js.contains("attrs:attrs"), "returns the attrs map");
        assert!(js.contains("visible:vis"), "computes visibility");
        assert!(js.contains("getAttribute('id')"), "returns the DOM id");
        assert!(js.contains("disabled:"), "returns disabled state");
    }

    #[test]
    fn element_links_js_scopes_to_the_element() {
        let js = element_links_js();
        assert!(js.contains("[data-fb="), "locates the tagged element");
        assert!(
            js.contains("querySelectorAll('a[href]')"),
            "collects links inside the element"
        );
    }

    #[test]
    fn live_resolve_handles_snapshot_kind() {
        let js = live_resolve_js(&ElementRef {
            kind: RefKind::Snapshot,
            value: "a".into(),
        });
        assert!(js.contains("kind==='snapshot'"));
        assert!(js.contains("data-fb=\"'+String(value)"));
    }

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
    fn accessibility_js_skips_non_rendered_tags() {
        let js = accessibility_js();
        assert!(js.contains("SKIP"));
        assert!(js.contains("SKIP[tagOf(el)]"));
        for t in ["script", "style", "template", "noscript", "head"] {
            assert!(js.contains(&format!("{t}:1")), "SKIP should include {t}");
        }
    }

    #[test]
    fn accessibility_js_has_robustness_features() {
        let js = accessibility_js();
        // Shadow DOM + same-origin iframe traversal.
        assert!(js.contains("shadowRoot"));
        assert!(js.contains("contentDocument"));
        // Depth cap + semantic-noise filter + hidden/opacity detection.
        assert!(js.contains("MAX_DEPTH"));
        assert!(js.contains("INTERACTIVE"));
        assert!(js.contains("opacity"));
        assert!(js.contains("aria-level"));
        // Hidden inputs must not be exposed as textboxes.
        assert!(js.contains("hidden"));
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

    #[test]
    fn input_event_js_synthesizes_dom_events() {
        use crate::engine::{InputEvent, KeyEvent, MouseButton, MouseEvent, WheelEvent};
        let k = input_event_js(&InputEvent::Key(KeyEvent::press("Enter"))).unwrap();
        assert!(k.contains("KeyboardEvent"), "k={k}");
        assert!(k.contains("keydown") && k.contains("keypress") && k.contains("keyup"));
        assert!(k.contains("\"Enter\""), "k={k}");

        let m = input_event_js(&InputEvent::Mouse(MouseEvent::click(
            10.0,
            20.0,
            MouseButton::Left,
        )))
        .unwrap();
        assert!(m.contains("elementFromPoint(10,20)"), "m={m}");
        assert!(m.contains("mousedown") && m.contains("mouseup") && m.contains("click"));

        let w = input_event_js(&InputEvent::Wheel(WheelEvent {
            x: 1.0,
            y: 2.0,
            delta_x: 0.0,
            delta_y: 120.0,
        }))
        .unwrap();
        assert!(
            w.contains("WheelEvent") && w.contains("deltaY:120"),
            "w={w}"
        );
    }

    #[test]
    fn focused_text_js_sets_value_and_handles_clear() {
        let js = focused_text_js("Alice", true);
        // Uses the native prototype setter so framework/controlled inputs react.
        assert!(js.contains("document.activeElement"), "js={js}");
        assert!(js.contains("getOwnPropertyDescriptor"), "js={js}");
        assert!(js.contains("'input'") && js.contains("'change'"), "js={js}");
        // `clear=true` must not append the current value.
        assert!(js.contains("clear = true"), "js={js}");
        // Text is JSON-escaped (embedded quotes are safe).
        let js2 = focused_text_js("a\"b", false);
        assert!(js2.contains(r#""a\"b""#), "js2={js2}");
        assert!(js2.contains("clear = false"), "js2={js2}");
    }

    #[test]
    fn point_action_js_uses_element_from_point() {
        let f = focus_at_point_js(10.0, 20.0);
        assert!(f.contains("elementFromPoint(10,20)"), "{f}");
        assert!(f.contains(".focus("), "{f}");

        let s = set_text_at_point_js(1.0, 2.0, "hi", true);
        assert!(s.contains("elementFromPoint(1,2)"), "{s}");
        assert!(s.contains("getOwnPropertyDescriptor"), "{s}");
        assert!(s.contains("clear=true"), "{s}");
        let s2 = set_text_at_point_js(-1.0, -1.0, "a\"b", false);
        assert!(s2.contains("elementFromPoint(-1,-1)"), "{s2}");
        assert!(s2.contains("clear=false"), "{s2}");
        assert!(s2.contains(r#""a\"b""#), "{s2}");

        let sel = select_at_point_js(3.0, 4.0, "Beta");
        assert!(sel.contains("closest('select')"), "{sel}");
        assert!(sel.contains("\"Beta\""), "{sel}");

        let c = check_at_point_js(5.0, 6.0, false);
        assert!(c.contains("input[type=checkbox]"), "{c}");
        assert!(c.contains("want=false"), "{c}");
    }

    #[test]
    fn fb_query_js_has_aria_dialects_and_helpers() {
        let js = fb_query_js();
        for k in [
            "implicitRole",
            "roleSyn",
            "accName",
            "roleMatches",
            "searchbox",
            "spinbutton",
            "columnheader",
            "progressbar",
        ] {
            assert!(js.contains(k), "missing helper/token: {k}");
        }
        // Extended dialect prefixes are recognized by the segment parser.
        for p in [
            "label",
            "placeholder",
            "alt",
            "title",
            "value",
            "href",
            "ref",
        ] {
            assert!(js.contains(p), "missing prefix: {p}");
        }
    }
}
