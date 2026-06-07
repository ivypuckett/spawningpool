//! Opt-in runtime validation of tool-call arguments against a tool's schema.
//!
//! Tools here are built dynamically, so their `parameters` is a runtime JSON
//! Schema with no compile-time type behind it. [`validate_tool_call`] is the
//! only safety net available: a caller that wants strictness runs it against a
//! model's tool call and, on failure, feeds an error [`ContentBlock::ToolResult`]
//! back to the model so it can retry. Callers that want raw pass-through simply
//! never call it.

use crate::ai::message::ContentBlock;
use crate::ai::model::Tool;

/// Why a tool call failed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolValidationError {
    /// The content block was not a [`ContentBlock::ToolCall`].
    NotAToolCall,
    /// The call's tool name did not match the tool it was validated against.
    NameMismatch { expected: String, actual: String },
    /// The tool's `parameters` was not a usable JSON Schema.
    InvalidSchema(String),
    /// The arguments did not satisfy the schema. Carries one message per
    /// schema violation.
    Invalid(Vec<String>),
}

impl std::fmt::Display for ToolValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolValidationError::NotAToolCall => write!(f, "content block is not a tool call"),
            ToolValidationError::NameMismatch { expected, actual } => {
                write!(
                    f,
                    "tool name mismatch: expected `{expected}`, got `{actual}`"
                )
            }
            ToolValidationError::InvalidSchema(m) => write!(f, "invalid tool schema: {m}"),
            ToolValidationError::Invalid(errors) => {
                write!(f, "tool arguments invalid: {}", errors.join("; "))
            }
        }
    }
}

impl std::error::Error for ToolValidationError {}

/// Validate a model's tool call against `tool`'s schema.
///
/// `call` must be a [`ContentBlock::ToolCall`] for the same tool. Returns the
/// full list of schema violations on failure so the caller can surface them to
/// the model.
pub fn validate_tool_call(tool: &Tool, call: &ContentBlock) -> Result<(), ToolValidationError> {
    let (name, arguments) = match call {
        ContentBlock::ToolCall {
            name, arguments, ..
        } => (name, arguments),
        _ => return Err(ToolValidationError::NotAToolCall),
    };
    if name != &tool.name {
        return Err(ToolValidationError::NameMismatch {
            expected: tool.name.clone(),
            actual: name.clone(),
        });
    }
    let validator = jsonschema::validator_for(&tool.parameters)
        .map_err(|e| ToolValidationError::InvalidSchema(e.to_string()))?;
    let errors: Vec<String> = validator
        .iter_errors(arguments)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ToolValidationError::Invalid(errors))
    }
}

#[cfg(test)]
mod tests {
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
}
