//! Tests for [`super`]. Extracted from `openai.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use crate::ai::catalog::lmstudio_model;
use crate::ai::message::{ContentBlock, Message, Role, StopReason};
use crate::ai::model::{Context, Model};
use crate::ai::provider::{CompleteOptions, Error, Reasoning, StreamEvent};

fn model() -> Model {
    lmstudio_model("local-model")
}

#[test]
fn system_is_folded_into_messages_and_tool_result_is_a_tool_message() {
    let ctx = Context {
        system: Some("be terse".into()),
        messages: vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::ToolResult {
                    tool_call_id: "call_1".into(),
                    content: "42".into(),
                    is_error: false,
                },
                ContentBlock::text("thanks"),
            ],
        }],
        tools: vec![],
    };
    let body = build_request(&model(), &ctx, &CompleteOptions::default(), false);
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["role"], "tool");
    assert_eq!(messages[1]["tool_call_id"], "call_1");
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"], "thanks");
}

#[test]
fn assistant_tool_call_serializes_as_function_call() {
    let ctx = Context {
        system: None,
        messages: vec![Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "get_weather".into(),
                arguments: serde_json::json!({ "city": "Paris" }),
            }],
        }],
        tools: vec![],
    };
    let body = build_request(&model(), &ctx, &CompleteOptions::default(), false);
    let call = &body["messages"][0]["tool_calls"][0];
    assert_eq!(call["function"]["name"], "get_weather");
    assert_eq!(call["function"]["arguments"], r#"{"city":"Paris"}"#);
}

#[test]
fn forced_tool_choice_uses_required_and_default_omits_it() {
    // A forced tool call serializes as the portable "required" string, not
    // the per-function object form that some OpenAI-compatible servers reject.
    let opts = CompleteOptions {
        tool_choice: Some("get_weather".into()),
        ..Default::default()
    };
    let body = build_request(&model(), &Context::default(), &opts, false);
    assert_eq!(body["tool_choice"], "required");
    assert!(body.get("response_format").is_none());

    let default = build_request(
        &model(),
        &Context::default(),
        &CompleteOptions::default(),
        false,
    );
    assert!(default.get("tool_choice").is_none());
}

#[test]
fn constrained_decoding_emits_response_format_and_drops_tools() {
    // With the provider's constrained-decoding capability declared, a forced
    // call is realized via response_format json_schema built from the tool's
    // parameter schema — no tools or tool_choice sent.
    let ctx = Context {
        system: None,
        messages: vec![],
        tools: vec![crate::ai::model::Tool {
            name: "classify".into(),
            description: "Classify".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "label": { "type": "string" } },
                "required": ["label"],
            }),
        }],
    };
    let opts = CompleteOptions {
        tool_choice: Some("classify".into()),
        constrained_decoding: true,
        ..Default::default()
    };
    let body = build_request(&model(), &ctx, &opts, false);
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    let format = &body["response_format"];
    assert_eq!(format["type"], "json_schema");
    assert_eq!(format["json_schema"]["name"], "classify");
    assert_eq!(format["json_schema"]["strict"], true);
    assert_eq!(
        format["json_schema"]["schema"]["additionalProperties"],
        false
    );
    assert_eq!(
        format["json_schema"]["schema"]["properties"]["label"]["type"],
        "string"
    );
}

#[test]
fn synthesize_constrained_call_wraps_json_text_as_a_tool_call() {
    let content = vec![ContentBlock::Text {
        text: r#"{"label":"spam"}"#.into(),
    }];
    let call = synthesize_constrained_call(&content, "classify");
    assert_eq!(
        call,
        vec![ContentBlock::ToolCall {
            id: "constrained_classify".into(),
            name: "classify".into(),
            arguments: serde_json::json!({ "label": "spam" }),
        }]
    );
}

#[test]
fn parses_response_with_tool_call() {
    let raw = r#"{
        "choices": [{
            "message": {"content": null, "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}
            ]},
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 8, "completion_tokens": 4}
    }"#;
    let parsed: WireResponse = serde_json::from_str(raw).unwrap();
    let completion = parsed.into_completion().unwrap();
    assert_eq!(completion.stop_reason, StopReason::ToolUse);
    assert_eq!(
        completion.message.content[0],
        ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "get_weather".into(),
            arguments: serde_json::json!({ "city": "Paris" }),
        }
    );
    assert_eq!(completion.usage.input, 8);
    assert_eq!(completion.usage.output, 4);
}

#[test]
fn reasoning_effort_maps_per_level_and_off_is_omitted() {
    let off = build_request(
        &model(),
        &Context::default(),
        &CompleteOptions::default(),
        false,
    );
    assert!(off.get("reasoning_effort").is_none());

    for (level, effort) in [
        (Reasoning::Low, "low"),
        (Reasoning::Medium, "medium"),
        (Reasoning::High, "high"),
    ] {
        let opts = CompleteOptions {
            reasoning: level,
            ..Default::default()
        };
        let body = build_request(&model(), &Context::default(), &opts, false);
        assert_eq!(body["reasoning_effort"], effort);
    }
}

#[test]
fn parses_text_only_response() {
    let raw = r#"{
        "choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 1}
    }"#;
    let parsed: WireResponse = serde_json::from_str(raw).unwrap();
    let completion = parsed.into_completion().unwrap();
    assert_eq!(
        completion.message.content,
        vec![ContentBlock::text("hello")]
    );
    assert_eq!(completion.stop_reason, StopReason::Stop);
}

#[test]
fn response_without_choices_is_a_parse_error() {
    let raw = r#"{"choices": [], "usage": {"prompt_tokens": 0, "completion_tokens": 0}}"#;
    let parsed: WireResponse = serde_json::from_str(raw).unwrap();
    assert!(matches!(parsed.into_completion(), Err(Error::Parse(_))));
}

#[test]
fn finish_reasons_map_to_unified_variants() {
    assert_eq!(map_finish_reason(Some("stop")), StopReason::Stop);
    assert_eq!(map_finish_reason(Some("length")), StopReason::Length);
    assert_eq!(map_finish_reason(Some("tool_calls")), StopReason::ToolUse);
    assert_eq!(
        map_finish_reason(Some("content_filter")),
        StopReason::Refusal
    );
    assert_eq!(map_finish_reason(Some("something_new")), StopReason::Error);
    assert_eq!(map_finish_reason(None), StopReason::Error);
}

#[test]
fn stream_accumulator_assembles_text_then_done() {
    let mut acc = StreamAccumulator::default();
    let mut out = Vec::new();
    for chunk in [
        serde_json::json!({"choices": [{"delta": {"content": "Hel"}}]}),
        serde_json::json!({"choices": [{"delta": {"content": "lo"}}]}),
        serde_json::json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}),
    ] {
        out.extend(acc.handle(&chunk));
    }
    out.push(acc.finish());
    assert_eq!(out.len(), 3);
    match out.last().unwrap() {
        StreamEvent::Done {
            stop_reason,
            usage,
            message,
        } => {
            assert_eq!(*stop_reason, StopReason::Stop);
            assert_eq!(usage.output, 2);
            assert_eq!(message.content, vec![ContentBlock::text("Hello")]);
        }
        other => panic!("expected Done, got {other:?}"),
    }
}
