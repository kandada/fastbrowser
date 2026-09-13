// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 统一错误类型（可 JSON 序列化，便于 C ABI / LLM 消费）。

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 错误分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// 标签页不存在或已关闭
    TabNotFound,
    /// 导航失败
    Navigation,
    /// 快照生成失败
    Snapshot,
    /// 视图/渲染相关失败
    View,
    /// 事件注入失败
    Input,
    /// JS 求值失败
    Evaluate,
    /// DOM 操作失败
    Dom,
    /// 文件/IO 失败
    Io,
    /// 等待超时
    Timeout,
    /// 当前引擎不支持该能力
    Unsupported,
    /// 宿主插件缺失或调用失败
    Plugin,
    /// 参数非法
    InvalidArgument,
    /// 内核尚未 init
    NotInitialized,
    /// 其他内部错误
    Internal,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ErrorKind::TabNotFound => "tab_not_found",
            ErrorKind::Navigation => "navigation",
            ErrorKind::Snapshot => "snapshot",
            ErrorKind::View => "view",
            ErrorKind::Input => "input",
            ErrorKind::Evaluate => "evaluate",
            ErrorKind::Dom => "dom",
            ErrorKind::Io => "io",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Plugin => "plugin",
            ErrorKind::InvalidArgument => "invalid_argument",
            ErrorKind::NotInitialized => "not_initialized",
            ErrorKind::Internal => "internal",
        };
        f.write_str(s)
    }
}

/// 内核统一错误。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineError {
    pub kind: ErrorKind,
    pub message: String,
}

impl EngineError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        EngineError {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidArgument, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    pub fn tab_not_found(tab: crate::engine::TabId) -> Self {
        Self::new(ErrorKind::TabNotFound, format!("tab {:?} not found", tab))
    }

    pub fn not_initialized() -> Self {
        Self::new(ErrorKind::NotInitialized, "kernel not initialized")
    }

    /// 序列化为统一的 JSON 错误结构：
    /// `{"error": {"kind": "...", "message": "..."}}`
    pub fn to_json(&self) -> Value {
        json!({ "error": { "kind": self.kind, "message": self.message } })
    }

    /// 将 serde_json 错误转为内核错误。
    pub fn from_serde(e: serde_json::Error) -> Self {
        Self::new(ErrorKind::InvalidArgument, format!("json: {e}"))
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fastbrowser[{}]: {}", self.kind, self.message)
    }
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::new(ErrorKind::Io, e.to_string())
    }
}

impl From<serde_json::Error> for EngineError {
    fn from(e: serde_json::Error) -> Self {
        EngineError::from_serde(e)
    }
}

/// 内核便捷 Result。
pub type Result<T> = std::result::Result<T, EngineError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_json_shape() {
        let e = EngineError::new(ErrorKind::TabNotFound, "tab 1 missing");
        let v = e.to_json();
        assert_eq!(v["error"]["kind"], "tab_not_found");
        assert_eq!(v["error"]["message"], "tab 1 missing");
        assert_eq!(format!("{e}"), "fastbrowser[tab_not_found]: tab 1 missing");
    }

    #[test]
    fn error_kind_display() {
        assert_eq!(ErrorKind::InvalidArgument.to_string(), "invalid_argument");
        assert_eq!(ErrorKind::NotInitialized.to_string(), "not_initialized");
    }
}
