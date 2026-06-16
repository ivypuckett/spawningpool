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
#[path = "validation_tests.rs"]
mod tests;
