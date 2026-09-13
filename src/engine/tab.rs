// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 标签页生命周期与信息。

use serde::{Deserialize, Serialize};

use crate::engine::Viewport;

/// 标签页 ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TabId(pub u32);

impl TabId {
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for TabId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 浏览器上下文 ID（隔离的 profile/cookie 域，对应 CDP BrowserContext）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextId(pub u32);

impl ContextId {
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for ContextId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 导航历史条目（对应 CDP `Page.getNavigationHistory`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    /// 过渡类型（"link" / "typed" / "reload" …）。
    pub transition: String,
}

/// 创建标签页的选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabOptions {
    /// 创建后是否立即激活。
    pub active: bool,
    /// 该标签页自定义 UA（覆盖全局）。
    pub user_agent: Option<String>,
    /// 该标签页初始视口。
    pub viewport: Option<Viewport>,
    /// 无痕模式（不落盘）。
    pub incognito: bool,
    /// Referrer。
    pub referrer: Option<String>,
}

impl Default for TabOptions {
    fn default() -> Self {
        TabOptions {
            active: true,
            user_agent: None,
            viewport: None,
            incognito: false,
            referrer: None,
        }
    }
}

/// 标签页信息（供 list_tabs / switch_tab 等使用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    /// 引擎内的临时句柄（仅本进程内有效；跨进程会重新编号）。
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub pinned: bool,
    pub created_ms: u64,
    /// 稳定标识（CDP `targetId`）：跨进程/跨连接唯一定位同一标签页，宿主可持久化
    /// 后在下一进程重新绑定。非 CDP 引擎（mock/webview）为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_id_serde_roundtrip() {
        let id = TabId(42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "42");
        let back: TabId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn tab_options_default() {
        let o = TabOptions::default();
        assert!(o.active);
        assert!(!o.incognito);
        assert!(o.user_agent.is_none());
    }
}
