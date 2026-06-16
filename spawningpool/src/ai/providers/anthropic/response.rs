//! The non-streaming reply: a `/v1/messages` response body mapped back into the
//! unified [`Completion`].

use crate::ai::message::{ContentBlock, Message, Role, Usage};
use crate::ai::provider::Completion;

#[derive(serde::Deserialize)]
pub(super) struct WireResponse {
    content: Vec<WireResponseBlock>,
    stop_reason: Option<String>,
    usage: WireUsage,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireResponseBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(other)]
    Unknown,
}

#[derive(serde::Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

impl WireResponse {
    pub(super) fn into_completion(self) -> Completion {
        let content = self
            .content
            .into_iter()
            .filter_map(|block| match block {
                WireResponseBlock::Text { text } => Some(ContentBlock::Text { text }),
                WireResponseBlock::Thinking { thinking } => {
                    Some(ContentBlock::Thinking { thinking })
                }
                WireResponseBlock::ToolUse { id, name, input } => Some(ContentBlock::ToolCall {
                    id,
                    name,
                    arguments: input,
                }),
                WireResponseBlock::Unknown => None,
            })
            .collect();
        Completion {
            message: Message {
                role: Role::Assistant,
                content,
            },
            stop_reason: super::map_stop_reason(self.stop_reason.as_deref()),
            usage: Usage {
                input: self.usage.input_tokens,
                output: self.usage.output_tokens,
            },
        }
    }
}
