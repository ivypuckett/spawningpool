//! The non-streaming reply: the Chat Completions response body mapped back into
//! the unified [`Completion`].

use crate::ai::message::{ContentBlock, Message, Role, Usage};
use crate::ai::provider::{Completion, Error};

#[derive(serde::Deserialize)]
pub(super) struct WireResponse {
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: WireUsage,
}

#[derive(serde::Deserialize)]
struct WireChoice {
    message: WireResponseMessage,
    finish_reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct WireResponseMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireResponseToolCall>,
}

#[derive(serde::Deserialize)]
struct WireResponseToolCall {
    id: String,
    function: WireResponseFunction,
}

#[derive(serde::Deserialize)]
struct WireResponseFunction {
    name: String,
    arguments: String,
}

#[derive(serde::Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

impl WireResponse {
    pub(super) fn into_completion(self) -> Result<Completion, Error> {
        let choice = self
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Parse("response had no choices".into()))?;
        let mut content = Vec::new();
        if let Some(text) = choice.message.content {
            if !text.is_empty() {
                content.push(ContentBlock::Text { text });
            }
        }
        for call in choice.message.tool_calls {
            content.push(ContentBlock::ToolCall {
                id: call.id,
                name: call.function.name,
                arguments: super::parse_args(&call.function.arguments),
            });
        }
        Ok(Completion {
            message: Message {
                role: Role::Assistant,
                content,
            },
            stop_reason: super::map_finish_reason(choice.finish_reason.as_deref()),
            usage: Usage {
                input: self.usage.prompt_tokens,
                output: self.usage.completion_tokens,
            },
        })
    }
}
