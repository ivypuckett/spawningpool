//! Tests for [`super`]. Extracted from `validation.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

fn weather_tool() -> Tool {
    Tool {
        name: "get_weather".into(),
        description: "Get weather for a city".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
            "additionalProperties": false,
        }),
    }
}

fn call(name: &str, arguments: serde_json::Value) -> ContentBlock {
    ContentBlock::ToolCall {
        id: "call_1".into(),
        name: name.into(),
        arguments,
    }
}

#[test]
fn valid_arguments_pass() {
    let result = validate_tool_call(
        &weather_tool(),
        &call("get_weather", serde_json::json!({ "city": "Paris" })),
    );
    assert_eq!(result, Ok(()));
}

#[test]
fn missing_required_field_reports_violations() {
    let err = validate_tool_call(&weather_tool(), &call("get_weather", serde_json::json!({})))
        .unwrap_err();
    match err {
        ToolValidationError::Invalid(errors) => assert!(!errors.is_empty()),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn wrong_type_is_invalid() {
    let err = validate_tool_call(
        &weather_tool(),
        &call("get_weather", serde_json::json!({ "city": 42 })),
    )
    .unwrap_err();
    assert!(matches!(err, ToolValidationError::Invalid(_)));
}

#[test]
fn name_mismatch_is_reported() {
    let err = validate_tool_call(
        &weather_tool(),
        &call("get_time", serde_json::json!({ "city": "Paris" })),
    )
    .unwrap_err();
    assert_eq!(
        err,
        ToolValidationError::NameMismatch {
            expected: "get_weather".into(),
            actual: "get_time".into(),
        }
    );
}

#[test]
fn non_tool_call_block_is_rejected() {
    let err = validate_tool_call(&weather_tool(), &ContentBlock::text("hi")).unwrap_err();
    assert_eq!(err, ToolValidationError::NotAToolCall);
}

#[test]
fn unusable_schema_is_reported() {
    let tool = Tool {
        name: "broken".into(),
        description: "bad schema".into(),
        // `type` must be a string or array of strings, not a number.
        parameters: serde_json::json!({ "type": 5 }),
    };
    let err = validate_tool_call(&tool, &call("broken", serde_json::json!({}))).unwrap_err();
    assert!(matches!(err, ToolValidationError::InvalidSchema(_)));
}
