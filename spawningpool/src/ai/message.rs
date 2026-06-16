//! Provider-agnostic conversation types.
//!
//! Everything in the core speaks this vocabulary. Each provider adapter is
//! responsible for translating these types to and from its own wire format,
//! so the rest of the crate never branches on which provider is in use.

use serde::{Deserialize, Serialize};

/// A single typed piece of message content.
///
/// Content is always an array of these blocks, which lets interleaved
/// thinking, text, and tool calls be represented uniformly regardless of
/// provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text { text: text.into() }
    }

    /// A successful tool result to feed back to the model.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    /// An error tool result (e.g. a validation failure) to feed back to the
    /// model so it can retry.
    pub fn tool_error(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            is_error: true,
        }
    }
}

/// The author of a message. Tool results are carried as `ContentBlock`s inside
/// a `User` message, matching how both target providers expect them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One turn in a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    /// A user turn containing a single text block.
    pub fn user(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![ContentBlock::text(text)],
        }
    }

    /// An assistant turn containing a single text block.
    pub fn assistant(text: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::text(text)],
        }
    }
}

/// Why the model stopped generating, normalized across providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Natural completion.
    Stop,
    /// Hit the output token cap.
    Length,
    /// The model wants one or more tools executed.
    ToolUse,
    /// The model declined to answer.
    Refusal,
    /// The provider reported an error stop.
    Error,
}

/// Token usage for a single response.
///
/// Token counts come straight from the provider and never go stale. Dollar
/// cost is deliberately *not* computed here: pricing changes over time and is
/// the caller's concern — multiply these counts by whatever current rates you
/// maintain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
}

#[cfg(test)]
#[path = "message_tests.rs"]
mod tests;
