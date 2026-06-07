//! Adapter for the Anthropic Messages API (`anthropic-messages`).
//!
//! Translates the unified message types to and from the `/v1/messages` wire
//! format. This is the adapter's only job; nothing about Anthropic leaks past
//! this module.

use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;

use crate::ai::message::{ContentBlock, Message, Role, StopReason, Usage};
use crate::ai::model::{Context, Model};
use crate::ai::provider::{
    CompleteOptions, Completion, Error, EventStream, Provider, Reasoning, StreamEvent,
};
use crate::ai::sse;

const API_VERSION: &str = "2023-06-01";

pub struct Anthropic;

#[async_trait]
impl Provider for Anthropic {
    async fn complete(
        &self,
        http: &reqwest::Client,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<Completion, Error> {
        let body = build_request(model, ctx, opts, false);
        let resp = send(http, model, opts, &body).await?;
        let parsed: WireResponse =
            serde_json::from_str(&resp).map_err(|e| Error::Parse(e.to_string()))?;
        Ok(parsed.into_completion(model))
    }

    async fn stream(
        &self,
        http: &reqwest::Client,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<EventStream, Error> {
        let body = build_request(model, ctx, opts, true);
        let resp = send_streaming(http, model, opts, &body).await?;
        let model = model.clone();
        let stream = async_stream::try_stream! {
            let mut lines = Box::pin(sse::data_lines(resp));
            let mut acc = StreamAccumulator::default();
            while let Some(line) = lines.next().await {
                let line = line?;
                let value: serde_json::Value =
                    serde_json::from_str(&line).map_err(|e| Error::Parse(e.to_string()))?;
                if let Some(event) = acc.handle(&value, &model) {
                    yield event;
                }
            }
        };
        Ok(Box::pin(stream))
    }
}

// --- request construction --------------------------------------------------

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<OutputConfig>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Serialize)]
struct WireMessage {
    role: &'static str,
    content: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct WireTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct OutputConfig {
    effort: &'static str,
}

fn build_request(
    model: &Model,
    ctx: &Context,
    opts: &CompleteOptions,
    stream: bool,
) -> serde_json::Value {
    let (thinking, output_config) = match opts.reasoning {
        Reasoning::Off => (None, None),
        Reasoning::Low => (Some("low"), Some("low")),
        Reasoning::Medium => (Some("medium"), Some("medium")),
        Reasoning::High => (Some("high"), Some("high")),
    };
    let request = WireRequest {
        model: &model.id,
        max_tokens: opts.max_tokens.unwrap_or(model.max_tokens),
        system: ctx.system.as_deref(),
        messages: ctx.messages.iter().map(to_wire_message).collect(),
        tools: ctx
            .tools
            .iter()
            .map(|t| WireTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect(),
        // Opus 4.8/4.7 accept only adaptive thinking; effort carries the level.
        thinking: thinking.map(|_| Thinking { kind: "adaptive" }),
        output_config: output_config.map(|effort| OutputConfig { effort }),
        stream,
    };
    serde_json::to_value(request).expect("request serializes")
}

fn to_wire_message(message: &Message) -> WireMessage {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let content = message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(serde_json::json!({ "type": "text", "text": text })),
            ContentBlock::ToolCall { id, name, arguments } => {
                Some(serde_json::json!({ "type": "tool_use", "id": id, "name": name, "input": arguments }))
            }
            ContentBlock::ToolResult { tool_call_id, content, is_error } => Some(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": content,
                "is_error": is_error,
            })),
            // Thinking blocks are not echoed back to the API in v1; see the
            // FUTURE_AGENT note in crate::ai about preserving thinking
            // signatures for multi-turn tool use.
            ContentBlock::Thinking { .. } => None,
        })
        .collect();
    WireMessage { role, content }
}

fn api_key(model: &Model, opts: &CompleteOptions) -> Result<String, Error> {
    if let Some(key) = &opts.api_key {
        return Ok(key.clone());
    }
    std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
        Error::Config(format!(
            "missing API key for {}: set ANTHROPIC_API_KEY or pass CompleteOptions::api_key",
            model.provider
        ))
    })
}

async fn send(
    http: &reqwest::Client,
    model: &Model,
    opts: &CompleteOptions,
    body: &serde_json::Value,
) -> Result<String, Error> {
    let resp = request(http, model, opts, body)?.send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(Error::Api {
            status: status.as_u16(),
            message: text,
        });
    }
    Ok(text)
}

async fn send_streaming(
    http: &reqwest::Client,
    model: &Model,
    opts: &CompleteOptions,
    body: &serde_json::Value,
) -> Result<reqwest::Response, Error> {
    let resp = request(http, model, opts, body)?.send().await?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await?;
        return Err(Error::Api {
            status: status.as_u16(),
            message: text,
        });
    }
    Ok(resp)
}

fn request(
    http: &reqwest::Client,
    model: &Model,
    opts: &CompleteOptions,
    body: &serde_json::Value,
) -> Result<reqwest::RequestBuilder, Error> {
    let key = api_key(model, opts)?;
    Ok(http
        .post(format!("{}/v1/messages", model.base_url))
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
        .json(body))
}

// --- response parsing ------------------------------------------------------

#[derive(serde::Deserialize)]
struct WireResponse {
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

fn map_stop_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("end_turn") | Some("stop_sequence") => StopReason::Stop,
        Some("max_tokens") => StopReason::Length,
        Some("tool_use") => StopReason::ToolUse,
        Some("refusal") => StopReason::Refusal,
        _ => StopReason::Error,
    }
}

impl WireResponse {
    fn into_completion(self, model: &Model) -> Completion {
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
        let cost = model.cost_for(self.usage.input_tokens, self.usage.output_tokens);
        Completion {
            message: Message {
                role: Role::Assistant,
                content,
            },
            stop_reason: map_stop_reason(self.stop_reason.as_deref()),
            usage: Usage {
                input: self.usage.input_tokens,
                output: self.usage.output_tokens,
                cost,
            },
        }
    }
}

// --- streaming accumulation ------------------------------------------------

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
struct StreamAccumulator {
    blocks: Vec<Option<BlockBuild>>,
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: Option<String>,
}

impl StreamAccumulator {
    fn handle(&mut self, value: &serde_json::Value, model: &Model) -> Option<StreamEvent> {
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
            "message_stop" => Some(self.finish(model)),
            _ => None,
        }
    }

    fn set_block(&mut self, index: usize, build: BlockBuild) {
        if index >= self.blocks.len() {
            self.blocks.resize_with(index + 1, || None);
        }
        self.blocks[index] = Some(build);
    }

    fn finish(&mut self, model: &Model) -> StreamEvent {
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
        let cost = model.cost_for(self.input_tokens, self.output_tokens);
        StreamEvent::Done {
            stop_reason: map_stop_reason(self.stop_reason.as_deref()),
            usage: Usage {
                input: self.input_tokens,
                output: self.output_tokens,
                cost,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::catalog::get_model;

    fn model() -> Model {
        get_model("anthropic", "claude-opus-4-8").unwrap()
    }

    #[test]
    fn request_puts_system_top_level_and_maps_tool_result() {
        let ctx = Context {
            system: Some("be terse".into()),
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_call_id: "call_1".into(),
                    content: "42".into(),
                    is_error: false,
                }],
            }],
            tools: vec![],
        };
        let body = build_request(&model(), &ctx, &CompleteOptions::default(), false);
        assert_eq!(body["system"], "be terse");
        assert_eq!(body["messages"][0]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][0]["content"][0]["tool_use_id"], "call_1");
        // Off reasoning sends no thinking field.
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn high_reasoning_sends_adaptive_thinking_and_effort() {
        let opts = CompleteOptions {
            reasoning: Reasoning::High,
            ..Default::default()
        };
        let body = build_request(&model(), &Context::default(), &opts, false);
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
    }

    #[test]
    fn parses_response_with_text_and_tool_use() {
        let raw = r#"{
            "content": [
                {"type": "text", "text": "Let me check."},
                {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "Paris"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 20}
        }"#;
        let parsed: WireResponse = serde_json::from_str(raw).unwrap();
        let completion = parsed.into_completion(&model());
        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        assert_eq!(completion.message.content.len(), 2);
        assert_eq!(
            completion.message.content[1],
            ContentBlock::ToolCall {
                id: "toolu_1".into(),
                name: "get_weather".into(),
                arguments: serde_json::json!({ "city": "Paris" }),
            }
        );
        // 10/1e6*5 + 20/1e6*25
        assert!(
            (completion.usage.cost.total - (10.0 / 1e6 * 5.0 + 20.0 / 1e6 * 25.0)).abs() < 1e-12
        );
    }

    #[test]
    fn unknown_blocks_are_ignored() {
        let raw = r#"{
            "content": [{"type": "redacted_thinking", "data": "xxx"}, {"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }"#;
        let parsed: WireResponse = serde_json::from_str(raw).unwrap();
        let completion = parsed.into_completion(&model());
        assert_eq!(completion.message.content, vec![ContentBlock::text("hi")]);
        assert_eq!(completion.stop_reason, StopReason::Stop);
    }

    #[test]
    fn stream_accumulator_assembles_message_and_usage() {
        let model = model();
        let mut acc = StreamAccumulator::default();
        let events: Vec<serde_json::Value> = vec![
            serde_json::json!({"type": "message_start", "message": {"usage": {"input_tokens": 5}}}),
            serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text"}}),
            serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hel"}}),
            serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "lo"}}),
            serde_json::json!({"type": "content_block_stop", "index": 0}),
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 7}}),
            serde_json::json!({"type": "message_stop"}),
        ];
        let mut out = Vec::new();
        for ev in &events {
            if let Some(e) = acc.handle(ev, &model) {
                out.push(e);
            }
        }
        // Two text deltas, then Done.
        assert_eq!(out.len(), 3);
        match out.last().unwrap() {
            StreamEvent::Done {
                stop_reason,
                usage,
                message,
            } => {
                assert_eq!(*stop_reason, StopReason::Stop);
                assert_eq!(usage.input, 5);
                assert_eq!(usage.output, 7);
                assert_eq!(message.content, vec![ContentBlock::text("Hello")]);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }
}
