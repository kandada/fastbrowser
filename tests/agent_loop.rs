// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 模拟 ReAct Agent 循环：证明"AI 原生"工具链可被任意循环消费。
//!
//! 这个测试不引入 LLM，而是用一个确定性"策略"驱动：读取快照 → 按规则选元素 →
//! 执行 → 再快照。演示 感知(快照) → 决策 → 行动 的完整闭环。

use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;

/// 一个最小 ReAct 循环：在搜索页输入关键词并点搜索。
#[test]
fn simulated_react_loop() {
    let sdk = Fastbrowser::new();
    sdk.init(Config::default()).unwrap();
    sdk.open("https://example.com/search").unwrap();

    // ── 第 1 步：感知（snapshot）────────────────────────────
    let snap = sdk.snapshot().unwrap();
    assert_eq!(snap.title, "Search");

    // 找到输入框与搜索按钮（规则：input[name=q] / button 文本 Search）
    let input = snap
        .interactive
        .iter()
        .find(|e| e.input_type.as_deref() == Some("text"))
        .expect("input");
    let button = snap
        .interactive
        .iter()
        .find(|e| e.tag == "button" && e.text.as_deref() == Some("Search"))
        .expect("search button");

    // ── 第 2 步：决策 → 行动 ───────────────────────────────
    sdk.tool_call(
        "type",
        serde_json::json!({"id": input.id.to_string(), "text": "fastbrowser"}),
    )
    .unwrap();
    sdk.tool_call("click", serde_json::json!({"id": button.id.to_string()}))
        .unwrap();

    // ── 第 3 步：再感知，验证结果 ───────────────────────────
    let snap2 = sdk.snapshot().unwrap();
    assert_eq!(snap2.interactive.iter().filter(|e| e.tag == "a").count(), 3);

    let links = sdk
        .tool_call("extract_links", serde_json::json!({}))
        .unwrap();
    assert!(links["links"].as_array().unwrap().len() >= 3);
}

/// 循环里处理工具报错的路径。
#[test]
fn loop_error_recovery() {
    let sdk = Fastbrowser::new();
    sdk.init(Config::default()).unwrap();
    sdk.open("https://example.com").unwrap();

    // 首次调用不存在的元素 → 报错；Agent 应重新快照后再试。
    let res = sdk.tool_call("click", serde_json::json!({"id": "zz"}));
    assert!(res.is_err());

    // 重新感知后成功。
    let snap = sdk.snapshot().unwrap();
    let first = &snap.interactive[0];
    sdk.tool_call("click", serde_json::json!({"id": first.id.to_string()}))
        .unwrap();
}
