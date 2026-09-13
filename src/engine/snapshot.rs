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
}

/// 元素引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementRef {
    pub kind: RefKind,
    pub value: String,
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
}
