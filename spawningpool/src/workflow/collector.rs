//! Accumulates a specialist run's [`RunEvent`] stream into the workflow
//! result envelope (workflow-dsl.md §4), consumed by [`super::eval`].

use crate::ai::StopReason;
use crate::run::RunEvent;

/// Accumulates [`RunEvent`]s from a specialist run into the JSON envelope
/// (workflow-dsl.md §4).
#[derive(Default)]
pub(super) struct Collector {
    output: String,
    thinking: String,
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: Option<StopReason>,
    turns: u32,
    tool_calls: Vec<serde_json::Value>,
}

impl Collector {
    pub(super) fn observe(&mut self, event: RunEvent<'_>) {
        match event {
            RunEvent::TextDelta(t) | RunEvent::Text(t) => self.output.push_str(t),
            RunEvent::ThinkingDelta(t) | RunEvent::Thinking(t) => self.thinking.push_str(t),
            RunEvent::TurnDone { stop_reason } => {
                self.stop_reason = Some(stop_reason);
                self.turns += 1;
            }
            RunEvent::Usage(u) => {
                self.input_tokens += u.input;
                self.output_tokens += u.output;
            }
            RunEvent::ToolRan {
                name,
                output,
                success,
            } => {
                self.tool_calls.push(serde_json::json!({
                    "name": name,
                    "success": success,
                    "output": output,
                }));
            }
            RunEvent::ToolFailed { name, message } => {
                self.tool_calls.push(serde_json::json!({
                    "name": name,
                    "success": false,
                    "output": message,
                }));
            }
        }
    }

    pub(super) fn into_envelope(self, specialist: &str, model: &str) -> serde_json::Value {
        serde_json::json!({
            "output": self.output,
            "thinking": self.thinking,
            "inputTokens": self.input_tokens,
            "outputTokens": self.output_tokens,
            "stopReason": self.stop_reason,
            "model": model,
            "specialist": specialist,
            "turns": self.turns,
            "toolCalls": self.tool_calls,
        })
    }
}
