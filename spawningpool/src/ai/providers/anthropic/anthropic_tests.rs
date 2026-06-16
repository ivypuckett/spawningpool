//! Tests for [`super`]. Extracted from `anthropic.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::request::to_wire_message;
use super::*;
use crate::ai::message::{ContentBlock, Message, Role};
use crate::ai::model::{Api, Context, Model};
use crate::ai::provider::{CompleteOptions, Reasoning, StreamEvent};

fn model() -> Model {
    Model {
        id: "claude-opus-4-8".to_string(),
        name: "Claude Opus 4.8".to_string(),
        api: Api::AnthropicMessages,
        provider: "anthropic".to_string(),
        base_url: "https://api.anthropic.com".to_string(),
        max_tokens: 4096,
        context_window: 200_000,
    }
}

#[test]
fn request_puts_system_top_level_and_maps_tool_result() {
    let ctx = Context {
        system: Some("be terse".into()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_call_id: "call_1".into(),
                content: "42".into(),
                is_error: false,
            }],
        }],
        tools: vec![],
    };
    let body = build_request(&model(), &ctx, &CompleteOptions::default(), false);
    assert_eq!(body["system"], "be terse");
    assert_eq!(body["messages"][0]["content"][0]["type"], "tool_result");
    assert_eq!(body["messages"][0]["content"][0]["tool_use_id"], "call_1");
    // Off reasoning sends no thinking field.
    assert!(body.get("thinking").is_none());
}

#[test]
fn forced_tool_choice_serializes_and_default_omits_it() {
    let opts = CompleteOptions {
        tool_choice: Some("get_weather".into()),
        ..Default::default()
    };
    let body = build_request(&model(), &Context::default(), &opts, false);
    assert_eq!(body["tool_choice"]["type"], "tool");
    assert_eq!(body["tool_choice"]["name"], "get_weather");

    let default = build_request(
        &model(),
        &Context::default(),
        &CompleteOptions::default(),
        false,
    );
    assert!(default.get("tool_choice").is_none());
}

#[test]
fn high_reasoning_sends_adaptive_thinking_and_effort() {
    let opts = CompleteOptions {
        reasoning: Reasoning::High,
        ..Default::default()
    };
    let body = build_request(&model(), &Context::default(), &opts, false);
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(body["output_config"]["effort"], "high");
}

#[test]
fn parses_response_with_text_and_tool_use() {
    let raw = r#"{
        "content": [
            {"type": "text", "text": "Let me check."},
            {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "Paris"}}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 10, "output_tokens": 20}
    }"#;
    let parsed: WireResponse = serde_json::from_str(raw).unwrap();
    let completion = parsed.into_completion();
    assert_eq!(completion.stop_reason, StopReason::ToolUse);
    assert_eq!(completion.message.content.len(), 2);
    assert_eq!(
        completion.message.content[1],
        ContentBlock::ToolCall {
            id: "toolu_1".into(),
            name: "get_weather".into(),
            arguments: serde_json::json!({ "city": "Paris" }),
        }
    );
    assert_eq!(completion.usage.input, 10);
    assert_eq!(completion.usage.output, 20);
}

#[test]
fn unknown_blocks_are_ignored() {
    let raw = r#"{
        "content": [{"type": "redacted_thinking", "data": "xxx"}, {"type": "text", "text": "hi"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 1, "output_tokens": 1}
    }"#;
    let parsed: WireResponse = serde_json::from_str(raw).unwrap();
    let completion = parsed.into_completion();
    assert_eq!(completion.message.content, vec![ContentBlock::text("hi")]);
    assert_eq!(completion.stop_reason, StopReason::Stop);
}

#[test]
fn tool_call_maps_to_tool_use_and_thinking_is_dropped() {
    let wire = to_wire_message(&Message {
        role: Role::Assistant,
        content: vec![
            ContentBlock::Thinking {
                thinking: "internal".into(),
            },
            ContentBlock::ToolCall {
                id: "toolu_1".into(),
                name: "get_weather".into(),
                arguments: serde_json::json!({ "city": "Paris" }),
            },
        ],
    });
    let value = serde_json::to_value(&wire).unwrap();
    assert_eq!(value["role"], "assistant");
    // The thinking block is filtered out; only the tool_use survives.
    assert_eq!(value["content"].as_array().unwrap().len(), 1);
    assert_eq!(value["content"][0]["type"], "tool_use");
    assert_eq!(value["content"][0]["id"], "toolu_1");
    assert_eq!(value["content"][0]["name"], "get_weather");
    assert_eq!(value["content"][0]["input"]["city"], "Paris");
}

#[test]
fn low_and_medium_reasoning_map_to_adaptive_thinking_and_effort() {
    for (level, effort) in [(Reasoning::Low, "low"), (Reasoning::Medium, "medium")] {
        let opts = CompleteOptions {
            reasoning: level,
            ..Default::default()
        };
        let body = build_request(&model(), &Context::default(), &opts, false);
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], effort);
    }
}

#[test]
fn stop_reasons_map_to_unified_variants() {
    assert_eq!(map_stop_reason(Some("end_turn")), StopReason::Stop);
    assert_eq!(map_stop_reason(Some("stop_sequence")), StopReason::Stop);
    assert_eq!(map_stop_reason(Some("max_tokens")), StopReason::Length);
    assert_eq!(map_stop_reason(Some("tool_use")), StopReason::ToolUse);
    assert_eq!(map_stop_reason(Some("refusal")), StopReason::Refusal);
    assert_eq!(map_stop_reason(Some("something_new")), StopReason::Error);
    assert_eq!(map_stop_reason(None), StopReason::Error);
}

#[test]
fn stream_accumulator_assembles_message_and_usage() {
    let mut acc = StreamAccumulator::default();
    let events: Vec<serde_json::Value> = vec![
        serde_json::json!({"type": "message_start", "message": {"usage": {"input_tokens": 5}}}),
        serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text"}}),
        serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hel"}}),
        serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "lo"}}),
        serde_json::json!({"type": "content_block_stop", "index": 0}),
        serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 7}}),
        serde_json::json!({"type": "message_stop"}),
    ];
    let mut out = Vec::new();
    for ev in &events {
        if let Some(e) = acc.handle(ev) {
            out.push(e);
        }
    }
    // Two text deltas, then Done.
    assert_eq!(out.len(), 3);
    match out.last().unwrap() {
        StreamEvent::Done {
            stop_reason,
            usage,
            message,
        } => {
            assert_eq!(*stop_reason, StopReason::Stop);
            assert_eq!(usage.input, 5);
            assert_eq!(usage.output, 7);
            assert_eq!(message.content, vec![ContentBlock::text("Hello")]);
        }
        other => panic!("expected Done, got {other:?}"),
    }
}
