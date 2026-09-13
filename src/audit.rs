// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 动作审计日志：记录每次工具调用与 SDK 操作，供回放/调试/信任审计使用。
//!
//! 与 BrowserOS 的 audit 思路对齐：内核记录「谁在什么时间对哪个标签页
//! 做了什么、结果如何」，上层应用可据此渲染动作时间线或导出审计文件。

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 单条审计记录的最大 JSON 字节数。超出则截断（只留摘要），
/// 防止 screenshot 的 base64 等大结果把审计撑成 MB 级内存。
pub const MAX_AUDIT_JSON_BYTES: usize = 8192;

/// 判定为敏感的键名（命中即脱敏为 `***`）。
const SECRET_KEY_HINTS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "authorization",
    "auth",
    "session",
    "sid",
    "csrf",
    "cookie_value",
    "cookievalue",
];

/// 对动作参数做脱敏：已知敏感键整值替换；`fill_form` 的 `values` 内容整体脱敏
/// （保留字段键，隐藏填入值——登录/密码场景）；`cookie_set` 的 `value` 脱敏。
fn sanitize(action: &str, value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            if action == "fill_form" {
                if let Some(vals) = map.get_mut("values").and_then(Value::as_object_mut) {
                    for v in vals.values_mut() {
                        *v = json!("***");
                    }
                }
            }
            if action == "cookie_set" || action == "storage_set" {
                if let Some(v) = map.get_mut("value") {
                    *v = json!("***");
                }
            }
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                let sensitive = SECRET_KEY_HINTS
                    .iter()
                    .any(|h| lower == *h || lower.contains(h));
                let v = if sensitive {
                    json!("***")
                } else {
                    sanitize(action, v)
                };
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(|v| sanitize(action, v)).collect()),
        other => other,
    }
}

/// 超过大小上限的结果 → 保留摘要（含类型/尺寸），丢弃正文。
fn cap(value: Value) -> Value {
    let s = value.to_string();
    if s.len() <= MAX_AUDIT_JSON_BYTES {
        return value;
    }
    let mut summary = serde_json::Map::new();
    summary.insert("__truncated".into(), json!(true));
    summary.insert("bytes".into(), json!(s.len()));
    if let Some(o) = value.as_object() {
        for k in [
            "width", "height", "type", "format", "count", "saved", "path",
        ] {
            if let Some(v) = o.get(k) {
                summary.insert(k.into(), v.clone());
            }
        }
    }
    Value::Object(summary)
}

/// 一条审计记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// 自增序号。
    pub seq: u64,
    /// 时间戳（毫秒）。
    pub ts_ms: u64,
    /// 动作名（工具名或 SDK 操作名）。
    pub action: String,
    /// 目标标签页（若有）。
    pub tab: Option<u32>,
    /// 输入参数。
    pub params: Value,
    /// 结果（`{"ok": value}` 或 `{"error": {kind, message}}`）。
    pub result: Value,
    /// 耗时（毫秒）。
    pub duration_ms: u64,
}

impl AuditEntry {
    fn new(
        seq: u64,
        action: &str,
        tab: Option<u32>,
        params: Value,
        result: Value,
        duration_ms: u64,
    ) -> Self {
        AuditEntry {
            seq,
            ts_ms: now_ms(),
            action: action.to_string(),
            tab,
            params,
            result,
            duration_ms,
        }
    }
}

/// 审计日志（有界环形缓冲）。
#[derive(Debug, Default)]
pub struct AuditLog {
    entries: VecDeque<AuditEntry>,
    seq: u64,
    /// 最大保留条数（默认 500，超出丢最旧）。
    pub max_entries: usize,
}

impl AuditLog {
    pub fn new() -> Self {
        AuditLog {
            entries: VecDeque::new(),
            seq: 0,
            max_entries: 500,
        }
    }

    /// 记录一次成功的动作。
    pub fn record_ok(
        &mut self,
        action: &str,
        tab: Option<u32>,
        params: Value,
        value: Value,
        duration_ms: u64,
    ) {
        let seq = self.next_seq();
        let params = cap(sanitize(action, params));
        let result = json!({ "ok": cap(value) });
        self.push(AuditEntry::new(
            seq,
            action,
            tab,
            params,
            result,
            duration_ms,
        ));
    }

    /// 记录一次失败的动作。
    pub fn record_err(
        &mut self,
        action: &str,
        tab: Option<u32>,
        params: Value,
        err: &crate::engine::EngineError,
        duration_ms: u64,
    ) {
        let seq = self.next_seq();
        let params = cap(sanitize(action, params));
        let result = cap(err.to_json());
        self.push(AuditEntry::new(
            seq,
            action,
            tab,
            params,
            result,
            duration_ms,
        ));
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn push(&mut self, entry: AuditEntry) {
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// 全部记录（新→旧）。
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.iter().rev().cloned().collect()
    }

    /// 序列化为 JSON 数组。
    pub fn to_json(&self) -> Value {
        Value::Array(self.entries().iter().map(|e| json!(e)).collect())
    }

    /// 生成可回放的 Python 脚本（按时间正序，仅成功动作）。
    ///
    /// 注意：审计里的参数已做脱敏（密码/token 等敏感值替换为 `***`），
    /// 因此回放脚本中的敏感字段需要人工补回真实值。
    pub fn to_replay_python(&self) -> String {
        let mut out = String::new();
        out.push_str("import fastbrowser as fb\n");
        out.push_str("b = fb.FastBrowser()\n");
        out.push_str("b.init()\n\n");
        // entries() 返回新→旧，反转为旧→新（回放顺序）
        for e in self.entries().iter().rev() {
            if e.result.get("ok").is_none() {
                continue; // 跳过失败动作
            }
            match e.action.as_str() {
                "open" => {
                    if let Some(u) = e.params.get("url").and_then(Value::as_str) {
                        out.push_str(&format!("b.open({u:?})\n"));
                    }
                }
                _ => {
                    out.push_str(&format!("b.tool_call({:?}, {})\n", e.action, e.params));
                }
            }
        }
        out.push_str("\nb.shutdown()\n");
        out
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.entries.clear();
        self.seq = 0;
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_ok_and_err() {
        let mut log = AuditLog::new();
        log.record_ok("click", Some(1), json!({"id": "a"}), json!({"ok": true}), 1);
        log.record_err(
            "navigate",
            None,
            json!({"url": "x"}),
            &crate::engine::EngineError::new(crate::engine::ErrorKind::Navigation, "boom"),
            2,
        );
        let entries = log.entries();
        assert_eq!(entries.len(), 2);
        // 新→旧
        assert_eq!(entries[0].action, "navigate");
        assert!(entries[0].result.get("error").is_some());
        assert_eq!(entries[1].action, "click");
        assert!(entries[1].result.get("ok").is_some());
    }

    #[test]
    fn bounded_and_clear() {
        let mut log = AuditLog::new();
        log.max_entries = 3;
        for i in 0..5 {
            log.record_ok(&format!("a{i}"), None, json!({}), json!({}), 0);
        }
        assert_eq!(log.len(), 3);
        let names: Vec<String> = log.entries().iter().map(|e| e.action.clone()).collect();
        assert_eq!(
            names,
            vec!["a4".to_string(), "a3".to_string(), "a2".to_string()]
        );
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn sensitive_params_are_redacted() {
        let mut log = AuditLog::new();
        // fill_form：values 内容脱敏（保留键）
        log.record_ok(
            "fill_form",
            Some(1),
            json!({"values": {"b": "alice", "c": "hunter2"}}),
            json!({"filled": ["b", "c"]}),
            0,
        );
        // cookie_set：value 脱敏
        log.record_ok(
            "cookie_set",
            Some(1),
            json!({"name": "sid", "value": "abc123", "domain": "example.com"}),
            json!({"ok": true}),
            0,
        );
        // 嵌套敏感键：password / token
        log.record_ok(
            "execute_js",
            Some(1),
            json!({"script": "x", "token": "tok"}),
            json!({"result": "ok"}),
            0,
        );
        // entries 新→旧：fill_form 最后记录 → e[2]
        let e = log.entries();
        assert_eq!(e[2].params["values"]["b"], "***");
        assert_eq!(e[2].params["values"]["c"], "***");
        assert_eq!(e[1].params["value"], "***");
        assert_eq!(e[1].params["name"], "sid", "cookie name kept");
        assert_eq!(e[0].params["token"], "***");
        assert_eq!(e[0].params["script"], "x", "non-secret kept");
        // 非敏感字段保留
        assert_eq!(e[2].params["values"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn oversized_results_are_capped() {
        let mut log = AuditLog::new();
        let big = json!({"width": 1280, "height": 720, "base64": "A".repeat(20_000)});
        log.record_ok("screenshot", Some(1), json!({}), big, 0);
        let e = log.entries();
        assert_eq!(e[0].result["ok"]["__truncated"], true);
        assert_eq!(e[0].result["ok"]["width"], 1280);
        assert!(e[0].result["ok"]["bytes"].as_u64().unwrap() >= 20_000);
        // 存储的 JSON 有界
        let s = serde_json::to_string(&e[0]).unwrap();
        assert!(s.len() < 10_000, "audit entry too large: {}", s.len());
    }

    #[test]
    fn replay_python_script() {
        let mut log = AuditLog::new();
        log.record_ok(
            "open",
            None,
            json!({"url": "https://example.com"}),
            json!({"tab": 1}),
            0,
        );
        log.record_ok("click", Some(1), json!({"id": "a"}), json!({"ok": true}), 0);
        log.record_err(
            "navigate",
            None,
            json!({"url": "x"}),
            &crate::engine::EngineError::new(crate::engine::ErrorKind::Navigation, "boom"),
            0,
        );
        let s = log.to_replay_python();
        assert!(s.contains("import fastbrowser"));
        assert!(s.contains("b.open(\"https://example.com\")"));
        assert!(s.contains("b.tool_call(\"click\""));
        assert!(
            !s.contains("navigate"),
            "failed actions must be skipped during replay"
        );
        assert!(s.contains("b.shutdown()"));
    }
}
