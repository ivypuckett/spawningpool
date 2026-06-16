//! Tests for [`super`]. Extracted from `message.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

#[test]
fn content_block_round_trips_through_json() {
    let blocks = vec![
        ContentBlock::text("hello"),
        ContentBlock::Thinking {
            thinking: "hmm".into(),
        },
        ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "get_weather".into(),
            arguments: serde_json::json!({ "city": "Paris" }),
        },
        ContentBlock::ToolResult {
            tool_call_id: "call_1".into(),
            content: "sunny".into(),
            is_error: false,
        },
    ];
    let json = serde_json::to_string(&blocks).unwrap();
    let back: Vec<ContentBlock> = serde_json::from_str(&json).unwrap();
    assert_eq!(blocks, back);
}

#[test]
fn tool_call_block_uses_camel_case_tag_fields() {
    let json = serde_json::to_value(ContentBlock::ToolResult {
        tool_call_id: "x".into(),
        content: "y".into(),
        is_error: true,
    })
    .unwrap();
    assert_eq!(json["type"], "toolResult");
    assert_eq!(json["tool_call_id"], "x");
    assert_eq!(json["is_error"], true);
}
