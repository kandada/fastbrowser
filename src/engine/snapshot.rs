// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 交互元素快照（LLM 感知的第 1 号数据模型）。
//!
//! 每次页面变化后，内核生成 `PageSnapshot`：只包含「可交互元素」，
//! 每个元素分配 a/b/c… 编号。LLM 依据快照按编号操作元素。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::engine::{Rect, Viewport};

/// 元素引用类型（快照失效后的兜底定位方式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefKind {
    /// CSS 选择器
    Css,
    /// XPath
    Xpath,
    /// 文本匹配
    Text,
    /// 快照编号（a/b/c…）
    Snapshot,
    /// ARIA role 匹配（如 "button"、"link"、"textbox"）
    Role,
    /// Playwright 选择器引擎（链式 `>>`、`:visible`、`:has-text()`、`:text()` 等）
    Selector,
}

/// 元素引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementRef {
    pub kind: RefKind,
    pub value: String,
}

impl std::fmt::Display for RefKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RefKind::Css => "css",
            RefKind::Xpath => "xpath",
            RefKind::Text => "text",
            RefKind::Snapshot => "snapshot",
            RefKind::Role => "role",
            RefKind::Selector => "selector",
        };
        f.write_str(s)
    }
}

impl std::fmt::Display for ElementRef {
    /// 人类可读的 `kind:value`，用于错误消息 —— 避免把 `ElementRef { … }` 的
    /// Debug 结构泄漏给 LLM / 日志（见 session 里的 `element ElementRef { kind: Css, … } not found`）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.value)
    }
}

impl ElementRef {
    pub fn css(sel: impl Into<String>) -> Self {
        ElementRef {
            kind: RefKind::Css,
            value: sel.into(),
        }
    }
    pub fn xpath(expr: impl Into<String>) -> Self {
        ElementRef {
            kind: RefKind::Xpath,
            value: expr.into(),
        }
    }
    pub fn text(t: impl Into<String>) -> Self {
        ElementRef {
            kind: RefKind::Text,
            value: t.into(),
        }
    }
    pub fn snapshot(id: char) -> Self {
        ElementRef {
            kind: RefKind::Snapshot,
            value: id.to_string(),
        }
    }
    pub fn role(role: impl Into<String>) -> Self {
        ElementRef {
            kind: RefKind::Role,
            value: role.into(),
        }
    }
    /// Playwright 选择器（链式 `>>`、`:visible`、`:has-text()` 等）。
    pub fn selector(sel: impl Into<String>) -> Self {
        ElementRef {
            kind: RefKind::Selector,
            value: sel.into(),
        }
    }
}

/// 可交互元素。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractiveElement {
    /// a/b/c… 快照编号（页面内唯一）。
    pub id: char,
    /// 标签名（小写）。
    pub tag: String,
    /// ARIA role。
    pub role: Option<String>,
    /// 可见文本。
    pub text: Option<String>,
    /// 链接 href。
    pub href: Option<String>,
    /// 视口坐标。
    pub rect: Rect,
    /// 兜底引用。
    pub refs: Vec<ElementRef>,
    /// 关键属性快照。
    pub attrs: HashMap<String, String>,
    /// 输入框当前值。
    pub value: Option<String>,
    /// input 的 type（text/checkbox/radio/file…）。
    pub input_type: Option<String>,
    /// checkbox/radio 选中态。
    pub checked: Option<bool>,
    /// select 的可选值。
    pub selectable_options: Option<Vec<String>>,
    /// select 当前选中值。
    pub selected_option: Option<String>,
    /// 是否可见/可交互。
    pub visible: bool,
}

impl InteractiveElement {
    /// A usable id for tool output: the DOM/snapshot id when real, otherwise the
    /// first ref value — never the internal live sentinel (`\u{1}`), which the
    /// model would otherwise echo back as an unusable CSS id.
    pub fn display_id(&self) -> String {
        if self.id != crate::engine::inject::LIVE_ID {
            return self.id.to_string();
        }
        self.refs
            .first()
            .map(|r| r.value.clone())
            .unwrap_or_default()
    }

    pub fn refs_as_value(&self) -> serde_json::Value {
        serde_json::json!(self.refs)
    }

    /// 判断元素是否匹配给定引用（含 role 匹配；其余 kind 走 refs 包含判断）。
    pub fn matches_ref(&self, r: &ElementRef) -> bool {
        match r.kind {
            RefKind::Role => self
                .role
                .as_deref()
                .map(|role| role.eq_ignore_ascii_case(r.value.as_str()))
                .unwrap_or(false),
            _ => self.refs.contains(r),
        }
    }
}

/// 子框架快照（iframe / 同源可交互，跨域仅记 url）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameSnapshot {
    pub url: String,
    pub name: String,
    pub interactive: Vec<InteractiveElement>,
    pub text: Option<String>,
    /// 是否跨域（无法读取内部 DOM，仅可知其存在）。
    pub cross_origin: bool,
    /// 该框架内可交互元素总数。
    pub total: usize,
    /// 该框架是否因总数超限而截断。
    pub truncated: bool,
}

/// 链接信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkInfo {
    pub url: String,
    pub text: String,
}

/// 图片信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub src: String,
    pub alt: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// 快照生成时捕获的页面级元信息（滚动状态 / 截断提示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SnapshotMeta {
    /// 快照扫描到的可交互元素总数（含截断部分）。
    pub total: usize,
    /// 是否因超过 a..z 上限而截断（提示 LLM 需要滚动）。
    pub truncated: bool,
    /// 视口高度（CSS px）。
    pub viewport_h: u32,
    /// 文档总高度（CSS px）。
    pub scroll_h: u32,
    /// 当前垂直滚动位置（CSS px）。
    pub scroll_y: u32,
    /// 快照是否可能是**过期的**（页面忙/导航中，扫描超时未完成，返回了上次缓存）。
    /// true 时 Agent 应等待页面就绪后重新快照，避免按旧编号操作已变化的 DOM。
    pub stale: bool,
}

/// 识别**快照引用**：
/// - 单字母 `a`..`z`（内核快照编号）；
/// - Playwright MCP 风格 `eN`（`e`+数字，1-based，`e1` → 第 1 个可交互元素）。
///
/// 返回对应的快照字母（超出 a..z 上限时钳制到 `z`）。不匹配返回 `None`。
pub fn parse_snapshot_ref(s: &str) -> Option<char> {
    // Accept Playwright-MCP style `@ref` / `ref=` and surrounding whitespace.
    let t = s.trim().trim_start_matches('@');
    let t = t.strip_prefix("ref=").unwrap_or(t).trim();
    let mut chars = t.chars();
    if let Some(first) = chars.next() {
        // 只接受小写 a..z（保持内核“快照编号为小写字母”的契约）。
        if chars.next().is_none() && first.is_ascii_lowercase() {
            return Some(first);
        }
    }
    let num = t.strip_prefix('e')?;
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u32 = num.parse().ok()?;
    let idx = n.saturating_sub(1).min(25) as u8;
    Some((b'a' + idx) as char)
}

/// 是否需要用 Playwright 选择器引擎（链式 `>>` / `:visible` / `:has-text()` /
/// `:text()` / `:nth-match()`）。这些语法标准 `querySelectorAll` 不支持。
pub fn is_playwright_selector(s: &str) -> bool {
    let t = s.trim_start();
    s.contains(">>")
        || s.contains(":visible")
        || s.contains(":has-text(")
        || s.contains(":text(")
        || s.contains(":nth-match(")
        // role=button[name="X"] 需要引擎做可访问名过滤
        || (t.starts_with("role=") && s.contains('['))
        // ARIA/Testing-Library style prefixes need the JS dialect engine.
        || ["label=", "placeholder=", "alt=", "title=", "value=", "href=", "ref="]
            .iter()
            .any(|p| t.starts_with(p))
}

/// 解析元素目标**方言**为 [`ElementRef`]（web 与原生 surface 共用同一套解析）：
///
/// - 含 Playwright 语法（`>>` / `:visible` / `:has-text()` / `:text()` / `:nth-match()`）
///   → [`RefKind::Selector`]（由 JS 选择器引擎解析）
/// - `text=...` → 文本匹配
/// - `role=...` → ARIA role 匹配
/// - `xpath=...` / `//...` / `/html...` / `(/...` → XPath
/// - `css=...` → CSS
/// - `id=...` → CSS `#...`
/// - `data-testid=...` / `testid=...` → CSS `[data-testid="..."]`
/// - `nth=N` → 快照编号（第 N 个可交互元素，0-based）
/// - 其余 → CSS
///
/// 注意：单字母快照 id（a..z）与 `eN` 的判定在调用方（见 `parse_snapshot_ref`、
/// `tools::tool` 与 `surfaces::browser`），本函数不做该转换（`nth=` 除外）。
pub fn parse_selector_dialect(s: &str) -> ElementRef {
    let s = s.trim();
    // Playwright-MCP `@ref` prefix is not part of the dialect value.
    let s = s.strip_prefix('@').unwrap_or(s);
    if is_playwright_selector(s) {
        return ElementRef::selector(s);
    }
    if let Some(rest) = s.strip_prefix("text=") {
        return ElementRef::text(rest);
    }
    if let Some(rest) = s.strip_prefix("role=") {
        return ElementRef::role(rest);
    }
    if let Some(rest) = s.strip_prefix("xpath=") {
        return ElementRef::xpath(rest);
    }
    if let Some(rest) = s.strip_prefix("css=") {
        return ElementRef::css(rest);
    }
    if let Some(rest) = s.strip_prefix("id=") {
        return ElementRef::css(format!("#{rest}"));
    }
    if let Some(rest) = s
        .strip_prefix("data-testid=")
        .or_else(|| s.strip_prefix("testid="))
    {
        return ElementRef::css(format!("[data-testid=\"{rest}\"]"));
    }
    if let Some(rest) = s.strip_prefix("nth=") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            let idx = n.min(25) as u8;
            return ElementRef::snapshot((b'a' + idx) as char);
        }
    }
    if s.starts_with("//") || s.starts_with("/html") || s.starts_with("(/") {
        return ElementRef::xpath(s);
    }
    ElementRef::css(s)
}

/// 页面快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSnapshot {
    pub title: String,
    pub url: String,
    pub viewport: Viewport,
    pub interactive: Vec<InteractiveElement>,
    pub frames: Vec<FrameSnapshot>,
    /// 快照生成时间戳（毫秒）。
    pub timestamp_ms: u64,
    /// 页面级元信息（默认空；由引擎填充）。
    pub meta: SnapshotMeta,
}

impl PageSnapshot {
    /// 按编号查元素。
    pub fn element_by_id(&self, id: char) -> Option<&InteractiveElement> {
        self.interactive.iter().find(|e| e.id == id)
    }

    /// 顶层可见文本（把所有元素的可见文本拼接）。
    pub fn visible_text(&self) -> String {
        let mut parts: Vec<String> = self
            .interactive
            .iter()
            .filter(|e| e.visible)
            .filter_map(|e| e.text.clone())
            .collect();
        parts.sort();
        parts.dedup();
        parts.join(" ")
    }

    /// 生成 LLM 友好的紧凑描述（给 prompt 用）。
    pub fn to_llm_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("title: {}\nurl: {}\n", self.title, self.url));
        for e in &self.interactive {
            let role = e.role.as_deref().unwrap_or(&e.tag);
            let text = e.text.as_deref().unwrap_or("");
            out.push_str(&format!(
                "[{}] {} <{}> \"{}\" at {:?}\n",
                e.id, role, e.tag, text, e.rect
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PageSnapshot {
        PageSnapshot {
            title: "t".into(),
            url: "https://example.com".into(),
            viewport: Viewport::new(800, 600),
            interactive: vec![InteractiveElement {
                id: 'a',
                tag: "button".into(),
                role: Some("button".into()),
                text: Some("Submit".into()),
                href: None,
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                refs: vec![ElementRef::css("#submit")],
                attrs: HashMap::new(),
                value: None,
                input_type: None,
                checked: None,
                selectable_options: None,
                selected_option: None,
                visible: true,
            }],
            frames: vec![],
            timestamp_ms: 1,
            meta: SnapshotMeta::default(),
        }
    }

    #[test]
    fn element_by_id() {
        let s = sample();
        assert!(s.element_by_id('a').is_some());
        assert!(s.element_by_id('z').is_none());
    }

    #[test]
    fn display_id_never_exposes_live_sentinel() {
        let s = sample();
        // Snapshot id is real → shown as-is.
        assert_eq!(s.interactive[0].display_id(), "a");
        // A live-resolved element carries the sentinel id; `display_id` falls
        // back to the ref value so tools never emit `\u{1}`.
        let mut live = s.interactive[0].clone();
        live.id = crate::engine::inject::LIVE_ID;
        assert_eq!(live.display_id(), "#submit");
    }

    #[test]
    fn element_ref_display_is_clean() {
        // 错误消息里不能泄漏 Debug 结构（曾出现 `element ElementRef { kind: Css, … } not found`）。
        assert_eq!(ElementRef::css("div#foo").to_string(), "css:div#foo");
        assert_eq!(ElementRef::xpath("//a").to_string(), "xpath://a");
        assert_eq!(ElementRef::text("Submit").to_string(), "text:Submit");
        assert_eq!(ElementRef::snapshot('a').to_string(), "snapshot:a");
        assert_eq!(ElementRef::role("button").to_string(), "role:button");
        assert_eq!(
            ElementRef::selector("a >> text=Hi").to_string(),
            "selector:a >> text=Hi"
        );
        assert!(!ElementRef::css("div").to_string().contains("ElementRef"));
        assert!(!ElementRef::css("div").to_string().contains('{'));
    }

    #[test]
    fn llm_text_contains_element() {
        let s = sample();
        let txt = s.to_llm_text();
        assert!(txt.contains("[a] button <button> \"Submit\""));
    }

    #[test]
    fn matches_ref_by_role() {
        let s = sample();
        let el = &s.interactive[0];
        assert_eq!(el.role.as_deref(), Some("button"));
        assert!(el.matches_ref(&ElementRef::role("button")));
        assert!(
            el.matches_ref(&ElementRef::role("BUTTON")),
            "role matching is case-insensitive"
        );
        assert!(!el.matches_ref(&ElementRef::role("link")));
        assert!(el.matches_ref(&ElementRef::css("#submit")));
        assert!(!el.matches_ref(&ElementRef::snapshot('z')));
    }

    #[test]
    fn selector_dialect_parsing() {
        assert_eq!(parse_selector_dialect("text=Go"), ElementRef::text("Go"));
        assert_eq!(
            parse_selector_dialect("role=button"),
            ElementRef::role("button")
        );
        assert_eq!(
            parse_selector_dialect("xpath=//button"),
            ElementRef::xpath("//button")
        );
        assert_eq!(parse_selector_dialect("css=#go"), ElementRef::css("#go"));
        assert_eq!(
            parse_selector_dialect("//button"),
            ElementRef::xpath("//button")
        );
        assert_eq!(
            parse_selector_dialect("/html/body"),
            ElementRef::xpath("/html/body")
        );
        assert_eq!(
            parse_selector_dialect("button:has-text(\"Go\")"),
            ElementRef::selector("button:has-text(\"Go\")")
        );
        assert_eq!(
            parse_selector_dialect("button:has-text('Go')"),
            ElementRef::selector("button:has-text('Go')")
        );
        // 无前缀 → CSS（含前后空白归一）
        assert_eq!(
            parse_selector_dialect("  #submit "),
            ElementRef::css("#submit")
        );
        // 单字母不做快照转换（由调用方处理）
        assert_eq!(parse_selector_dialect("a"), ElementRef::css("a"));
    }

    #[test]
    fn snapshot_ref_parsing() {
        assert_eq!(parse_snapshot_ref("a"), Some('a'));
        assert_eq!(parse_snapshot_ref("z"), Some('z'));
        // 大写单字母不是快照编号（保持 a..z 契约）
        assert_eq!(parse_snapshot_ref("Z"), None);
        // Playwright MCP 风格 eN（1-based）
        assert_eq!(parse_snapshot_ref("e1"), Some('a'));
        assert_eq!(parse_snapshot_ref("e5"), Some('e'));
        assert_eq!(parse_snapshot_ref("e100"), Some('z'));
        assert_eq!(parse_snapshot_ref("e0"), Some('a'));
        assert_eq!(parse_snapshot_ref("e3"), Some('c'));
        // 非快照引用
        assert_eq!(parse_snapshot_ref("e"), Some('e')); // 单字母 e 本身是合法快照引用
        assert_eq!(parse_snapshot_ref("eX"), None);
        assert_eq!(parse_snapshot_ref("E3"), None);
        assert_eq!(parse_snapshot_ref("#go"), None);
        assert_eq!(parse_snapshot_ref("submit"), None);
        assert_eq!(parse_snapshot_ref(""), None);
        // Playwright-MCP `@ref` / `ref=` prefixes are normalized away.
        assert_eq!(parse_snapshot_ref("@a"), Some('a'));
        assert_eq!(parse_snapshot_ref("@e3"), Some('c'));
        assert_eq!(parse_snapshot_ref(" ref=e2 "), Some('b'));
    }

    #[test]
    fn playwright_selector_routing() {
        for s in [
            "div >> button",
            "button:visible",
            "button:has-text(\"Go\")",
            "div:text(\"x\")",
            "li:nth-match(li, 2)",
            "role=button[name=\"Go\"]",
        ] {
            assert!(is_playwright_selector(s), "{s}");
            assert_eq!(parse_selector_dialect(s), ElementRef::selector(s), "{s}");
        }
        for s in ["div", "#go", "text=Go", "role=button", "//a"] {
            assert!(!is_playwright_selector(s), "{s}");
        }
    }

    #[test]
    fn selector_dialect_id_testid_nth() {
        assert_eq!(
            parse_selector_dialect("id=submit"),
            ElementRef::css("#submit")
        );
        assert_eq!(
            parse_selector_dialect("data-testid=go"),
            ElementRef::css("[data-testid=\"go\"]")
        );
        assert_eq!(
            parse_selector_dialect("testid=go"),
            ElementRef::css("[data-testid=\"go\"]")
        );
        assert_eq!(parse_selector_dialect("nth=2"), ElementRef::snapshot('c'));
        assert_eq!(parse_selector_dialect("nth=99"), ElementRef::snapshot('z'));
        // 非法 nth 回退为 CSS
        assert_eq!(parse_selector_dialect("nth=x"), ElementRef::css("nth=x"));
    }

    #[test]
    fn selector_dialect_extended_prefixes_route_to_js_engine() {
        // ARIA / Testing-Library style prefixes must use the JS dialect engine.
        for s in [
            "label=Username",
            "placeholder=Email",
            "alt=Logo",
            "title=Hi",
            "value=Pro",
            "href=/about",
            "ref=e3",
        ] {
            assert!(is_playwright_selector(s), "{s}");
            assert_eq!(parse_selector_dialect(s), ElementRef::selector(s), "{s}");
            assert_eq!(
                parse_selector_dialect(&format!("@{s}")),
                ElementRef::selector(s)
            );
        }
        // role filters (ARIA getByRole options) route to the JS engine too.
        assert_eq!(
            parse_selector_dialect("role=checkbox[checked]"),
            ElementRef::selector("role=checkbox[checked]")
        );
    }
}
