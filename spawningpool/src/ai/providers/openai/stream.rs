//! SSE accumulation: the `delta` chunks of a streamed completion folded into a
//! growing message, emitted as unified [`StreamEvent`]s.

use crate::ai::message::{ContentBlock, Message, Role, Usage};
use crate::ai::provider::StreamEvent;

#[derive(Default)]
struct ToolCallBuild {
    id: String,
    name: String,
    args: String,
}

#[derive(Default)]
pub(super) struct StreamAccumulator {
    text: String,
    tool_calls: Vec<ToolCallBuild>,
    finish_reason: Option<String>,
    prompt_tokens: u32,
    completion_tokens: u32,
}

impl StreamAccumulator {
    pub(super) fn handle(&mut self, value: &serde_json::Value) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
            self.prompt_tokens = usage["prompt_tokens"]
                .as_u64()
                .unwrap_or(self.prompt_tokens as u64) as u32;
            self.completion_tokens = usage["completion_tokens"]
                .as_u64()
                .unwrap_or(self.completion_tokens as u64)
                as u32;
        }
        let Some(choice) = value["choices"].get(0) else {
            return events;
        };
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        let delta = &choice["delta"];
        if let Some(text) = delta["content"].as_str() {
            if !text.is_empty() {
                self.text.push_str(text);
                events.push(StreamEvent::TextDelta {
                    content_index: 0,
                    delta: text.to_string(),
                });
            }
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                let index = call["index"].as_u64().unwrap_or(0) as usize;
                if index >= self.tool_calls.len() {
                    self.tool_calls
                        .resize_with(index + 1, ToolCallBuild::default);
                }
                let build = &mut self.tool_calls[index];
                let id = call["id"].as_str();
                if let Some(id) = id {
                    build.id = id.to_string();
                }
                let name = call["function"]["name"].as_str();
                if let Some(name) = name {
                    build.name = name.to_string();
                }
                let args = call["function"]["arguments"].as_str().unwrap_or("");
                build.args.push_str(args);
                events.push(StreamEvent::ToolCallDelta {
                    content_index: index + 1,
                    id: id.map(str::to_string),
                    name: name.map(str::to_string),
                    arguments_delta: args.to_string(),
                });
            }
        }
        events
    }

    pub(super) fn finish(&mut self) -> StreamEvent {
        let mut content = Vec::new();
        if !self.text.is_empty() {
            content.push(ContentBlock::Text {
                text: std::mem::take(&mut self.text),
            });
        }
        for call in self.tool_calls.drain(..) {
            content.push(ContentBlock::ToolCall {
                id: call.id,
                name: call.name,
                arguments: super::parse_args(&call.args),
            });
        }
        StreamEvent::Done {
            stop_reason: super::map_finish_reason(self.finish_reason.as_deref()),
            usage: Usage {
                input: self.prompt_tokens,
                output: self.completion_tokens,
            },
            message: Message {
                role: Role::Assistant,
                content,
            },
        }
    }
}
