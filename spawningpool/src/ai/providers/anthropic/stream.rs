//! SSE accumulation: the `content_block_*`/`message_*` events of a streamed
//! reply folded into a growing message, emitted as unified [`StreamEvent`]s.

use crate::ai::message::{ContentBlock, Message, Role, Usage};
use crate::ai::provider::StreamEvent;

enum BlockBuild {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        args: String,
    },
}

#[derive(Default)]
pub(super) struct StreamAccumulator {
    blocks: Vec<Option<BlockBuild>>,
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: Option<String>,
}

impl StreamAccumulator {
    pub(super) fn handle(&mut self, value: &serde_json::Value) -> Option<StreamEvent> {
        match value["type"].as_str()? {
            "message_start" => {
                self.input_tokens = value["message"]["usage"]["input_tokens"]
                    .as_u64()
                    .unwrap_or(0) as u32;
                None
            }
            "content_block_start" => {
                let index = value["index"].as_u64().unwrap_or(0) as usize;
                let block = &value["content_block"];
                let build = match block["type"].as_str() {
                    Some("text") => BlockBuild::Text(String::new()),
                    Some("thinking") => BlockBuild::Thinking(String::new()),
                    Some("tool_use") => BlockBuild::ToolUse {
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        name: block["name"].as_str().unwrap_or("").to_string(),
                        args: String::new(),
                    },
                    _ => BlockBuild::Text(String::new()),
                };
                self.set_block(index, build);
                None
            }
            "content_block_delta" => {
                let index = value["index"].as_u64().unwrap_or(0) as usize;
                let delta = &value["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        let text = delta["text"].as_str().unwrap_or("");
                        if let Some(Some(BlockBuild::Text(s))) = self.blocks.get_mut(index) {
                            s.push_str(text);
                        }
                        Some(StreamEvent::TextDelta {
                            content_index: index,
                            delta: text.to_string(),
                        })
                    }
                    Some("thinking_delta") => {
                        let text = delta["thinking"].as_str().unwrap_or("");
                        if let Some(Some(BlockBuild::Thinking(s))) = self.blocks.get_mut(index) {
                            s.push_str(text);
                        }
                        Some(StreamEvent::ThinkingDelta {
                            content_index: index,
                            delta: text.to_string(),
                        })
                    }
                    Some("input_json_delta") => {
                        let partial = delta["partial_json"].as_str().unwrap_or("");
                        if let Some(Some(BlockBuild::ToolUse { args, .. })) =
                            self.blocks.get_mut(index)
                        {
                            args.push_str(partial);
                        }
                        Some(StreamEvent::ToolCallDelta {
                            content_index: index,
                            id: None,
                            name: None,
                            arguments_delta: partial.to_string(),
                        })
                    }
                    _ => None,
                }
            }
            "message_delta" => {
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                if let Some(out) = value["usage"]["output_tokens"].as_u64() {
                    self.output_tokens = out as u32;
                }
                None
            }
            "message_stop" => Some(self.finish()),
            _ => None,
        }
    }

    fn set_block(&mut self, index: usize, build: BlockBuild) {
        if index >= self.blocks.len() {
            self.blocks.resize_with(index + 1, || None);
        }
        self.blocks[index] = Some(build);
    }

    fn finish(&mut self) -> StreamEvent {
        let content = self
            .blocks
            .drain(..)
            .flatten()
            .map(|build| match build {
                BlockBuild::Text(text) => ContentBlock::Text { text },
                BlockBuild::Thinking(thinking) => ContentBlock::Thinking { thinking },
                BlockBuild::ToolUse { id, name, args } => ContentBlock::ToolCall {
                    id,
                    name,
                    arguments: parse_args(&args),
                },
            })
            .collect();
        StreamEvent::Done {
            stop_reason: super::map_stop_reason(self.stop_reason.as_deref()),
            usage: Usage {
                input: self.input_tokens,
                output: self.output_tokens,
            },
            message: Message {
                role: Role::Assistant,
                content,
            },
        }
    }
}

fn parse_args(raw: &str) -> serde_json::Value {
    if raw.trim().is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}
