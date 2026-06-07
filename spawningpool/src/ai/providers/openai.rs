//! Adapter for the OpenAI Chat Completions API (`openai-completions`).
//!
//! This single adapter serves LM Studio and any other OpenAI-compatible
//! endpoint; they differ only by `base_url` and (optional) auth. It folds the
//! unified `system` field into `messages[0]`, emits `tool_calls`, and maps the
//! responses back into the unified types.

use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;

use crate::ai::message::{ContentBlock, Message, Role, StopReason, Usage};
use crate::ai::model::{Context, Model};
use crate::ai::provider::{
    CompleteOptions, Completion, Error, EventStream, Provider, Reasoning, StreamEvent,
};
use crate::ai::sse;

pub struct OpenAi;

#[async_trait]
impl Provider for OpenAi {
    async fn complete(
        &self,
        http: &reqwest::Client,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<Completion, Error> {
        let body = build_request(model, ctx, opts, false);
        let resp = request(http, model, opts, &body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Api {
                status: status.as_u16(),
                message: text,
            });
        }
        let parsed: WireResponse =
            serde_json::from_str(&text).map_err(|e| Error::Parse(e.to_string()))?;
        parsed.into_completion()
    }

    async fn stream(
        &self,
        http: &reqwest::Client,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<EventStream, Error> {
        let body = build_request(model, ctx, opts, true);
        let resp = request(http, model, opts, &body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await?;
            return Err(Error::Api {
                status: status.as_u16(),
                message: text,
            });
        }
        let stream = async_stream::try_stream! {
            let mut lines = Box::pin(sse::data_lines(resp));
            let mut acc = StreamAccumulator::default();
            let mut finished = false;
            while let Some(line) = lines.next().await {
                let line = line?;
                if line == "[DONE]" {
                    yield acc.finish();
                    finished = true;
                    break;
                }
                let value: serde_json::Value =
                    serde_json::from_str(&line).map_err(|e| Error::Parse(e.to_string()))?;
                for event in acc.handle(&value) {
                    yield event;
                }
            }
            if !finished {
                yield acc.finish();
            }
        };
        Ok(Box::pin(stream))
    }
}

// --- request construction --------------------------------------------------

#[derive(Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<WireToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Serialize)]
struct WireMessage {
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<WireToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunction,
}

#[derive(Serialize)]
struct WireFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireToolFunction,
}

#[derive(Serialize)]
struct WireToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct WireToolChoice {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireToolChoiceFunction,
}

#[derive(Serialize)]
struct WireToolChoiceFunction {
    name: String,
}

fn build_request(
    model: &Model,
    ctx: &Context,
    opts: &CompleteOptions,
    stream: bool,
) -> serde_json::Value {
    let mut messages = Vec::new();
    if let Some(system) = &ctx.system {
        messages.push(WireMessage {
            role: "system",
            content: Some(system.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
    }
    for message in &ctx.messages {
        append_message(&mut messages, message);
    }
    let request = WireRequest {
        model: model.id.clone(),
        messages,
        max_tokens: opts.max_tokens.unwrap_or(model.max_tokens),
        tools: ctx
            .tools
            .iter()
            .map(|t| WireTool {
                kind: "function",
                function: WireToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect(),
        tool_choice: opts.tool_choice.as_ref().map(|name| WireToolChoice {
            kind: "function",
            function: WireToolChoiceFunction { name: name.clone() },
        }),
        reasoning_effort: match opts.reasoning {
            Reasoning::Off => None,
            Reasoning::Low => Some("low"),
            Reasoning::Medium => Some("medium"),
            Reasoning::High => Some("high"),
        },
        stream,
    };
    serde_json::to_value(request).expect("request serializes")
}

fn append_message(messages: &mut Vec<WireMessage>, message: &Message) {
    match message.role {
        Role::User => {
            let mut text = String::new();
            for block in &message.content {
                match block {
                    ContentBlock::Text { text: t } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                    // Tool results are their own `tool`-role messages.
                    ContentBlock::ToolResult {
                        tool_call_id,
                        content,
                        ..
                    } => {
                        messages.push(WireMessage {
                            role: "tool",
                            content: Some(content.clone()),
                            tool_calls: None,
                            tool_call_id: Some(tool_call_id.clone()),
                        });
                    }
                    _ => {}
                }
            }
            if !text.is_empty() {
                messages.push(WireMessage {
                    role: "user",
                    content: Some(text),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }
        Role::Assistant => {
            let mut text = String::new();
            let mut tool_calls = Vec::new();
            for block in &message.content {
                match block {
                    ContentBlock::Text { text: t } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                    ContentBlock::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        tool_calls.push(WireToolCall {
                            id: id.clone(),
                            kind: "function",
                            function: WireFunction {
                                name: name.clone(),
                                arguments: arguments.to_string(),
                            },
                        });
                    }
                    _ => {}
                }
            }
            messages.push(WireMessage {
                role: "assistant",
                content: (!text.is_empty()).then_some(text),
                tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                tool_call_id: None,
            });
        }
    }
}

fn request(
    http: &reqwest::Client,
    model: &Model,
    opts: &CompleteOptions,
    body: &serde_json::Value,
) -> reqwest::RequestBuilder {
    let mut builder = http
        .post(format!("{}/v1/chat/completions", model.base_url))
        .header("content-type", "application/json")
        .json(body);
    // LM Studio does not require a key, but honor one if provided.
    if let Some(key) = opts
        .api_key
        .clone()
        .or_else(|| std::env::var("LMSTUDIO_API_KEY").ok())
    {
        builder = builder.bearer_auth(key);
    }
    builder
}

/// Discover models from a running OpenAI-compatible server via `GET /v1/models`.
pub(crate) async fn list_models(http: &reqwest::Client) -> Result<Vec<Model>, Error> {
    let base_url = crate::ai::catalog::lmstudio_base_url();
    let resp = http.get(format!("{base_url}/v1/models")).send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(Error::Api {
            status: status.as_u16(),
            message: text,
        });
    }
    let parsed: WireModelList =
        serde_json::from_str(&text).map_err(|e| Error::Parse(e.to_string()))?;
    Ok(parsed
        .data
        .into_iter()
        .map(|m| crate::ai::catalog::lmstudio_model(&m.id))
        .collect())
}

#[derive(serde::Deserialize)]
struct WireModelList {
    data: Vec<WireModelEntry>,
}

#[derive(serde::Deserialize)]
struct WireModelEntry {
    id: String,
}

// --- response parsing ------------------------------------------------------

#[derive(serde::Deserialize)]
struct WireResponse {
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

fn map_finish_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("stop") => StopReason::Stop,
        Some("length") => StopReason::Length,
        Some("tool_calls") => StopReason::ToolUse,
        Some("content_filter") => StopReason::Refusal,
        _ => StopReason::Error,
    }
}

impl WireResponse {
    fn into_completion(self) -> Result<Completion, Error> {
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
                arguments: parse_args(&call.function.arguments),
            });
        }
        Ok(Completion {
            message: Message {
                role: Role::Assistant,
                content,
            },
            stop_reason: map_finish_reason(choice.finish_reason.as_deref()),
            usage: Usage {
                input: self.usage.prompt_tokens,
                output: self.usage.completion_tokens,
            },
        })
    }
}

// --- streaming accumulation ------------------------------------------------

#[derive(Default)]
struct ToolCallBuild {
    id: String,
    name: String,
    args: String,
}

#[derive(Default)]
struct StreamAccumulator {
    text: String,
    tool_calls: Vec<ToolCallBuild>,
    finish_reason: Option<String>,
    prompt_tokens: u32,
    completion_tokens: u32,
}

impl StreamAccumulator {
    fn handle(&mut self, value: &serde_json::Value) -> Vec<StreamEvent> {
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

    fn finish(&mut self) -> StreamEvent {
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
                arguments: parse_args(&call.args),
            });
        }
        StreamEvent::Done {
            stop_reason: map_finish_reason(self.finish_reason.as_deref()),
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

fn parse_args(raw: &str) -> serde_json::Value {
    if raw.trim().is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::catalog::lmstudio_model;

    fn model() -> Model {
        lmstudio_model("local-model")
    }

    #[test]
    fn system_is_folded_into_messages_and_tool_result_is_a_tool_message() {
        let ctx = Context {
            system: Some("be terse".into()),
            messages: vec![Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult {
                        tool_call_id: "call_1".into(),
                        content: "42".into(),
                        is_error: false,
                    },
                    ContentBlock::text("thanks"),
                ],
            }],
            tools: vec![],
        };
        let body = build_request(&model(), &ctx, &CompleteOptions::default(), false);
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], "thanks");
    }

    #[test]
    fn assistant_tool_call_serializes_as_function_call() {
        let ctx = Context {
            system: None,
            messages: vec![Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "get_weather".into(),
                    arguments: serde_json::json!({ "city": "Paris" }),
                }],
            }],
            tools: vec![],
        };
        let body = build_request(&model(), &ctx, &CompleteOptions::default(), false);
        let call = &body["messages"][0]["tool_calls"][0];
        assert_eq!(call["function"]["name"], "get_weather");
        assert_eq!(call["function"]["arguments"], r#"{"city":"Paris"}"#);
    }

    #[test]
    fn forced_tool_choice_serializes_and_default_omits_it() {
        let opts = CompleteOptions {
            tool_choice: Some("get_weather".into()),
            ..Default::default()
        };
        let body = build_request(&model(), &Context::default(), &opts, false);
        assert_eq!(body["tool_choice"]["type"], "function");
        assert_eq!(body["tool_choice"]["function"]["name"], "get_weather");

        let default = build_request(
            &model(),
            &Context::default(),
            &CompleteOptions::default(),
            false,
        );
        assert!(default.get("tool_choice").is_none());
    }

    #[test]
    fn parses_response_with_tool_call() {
        let raw = r#"{
            "choices": [{
                "message": {"content": null, "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}
                ]},
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 8, "completion_tokens": 4}
        }"#;
        let parsed: WireResponse = serde_json::from_str(raw).unwrap();
        let completion = parsed.into_completion().unwrap();
        assert_eq!(completion.stop_reason, StopReason::ToolUse);
        assert_eq!(
            completion.message.content[0],
            ContentBlock::ToolCall {
                id: "call_1".into(),
                name: "get_weather".into(),
                arguments: serde_json::json!({ "city": "Paris" }),
            }
        );
        assert_eq!(completion.usage.input, 8);
        assert_eq!(completion.usage.output, 4);
    }

    #[test]
    fn stream_accumulator_assembles_text_then_done() {
        let mut acc = StreamAccumulator::default();
        let mut out = Vec::new();
        for chunk in [
            serde_json::json!({"choices": [{"delta": {"content": "Hel"}}]}),
            serde_json::json!({"choices": [{"delta": {"content": "lo"}}]}),
            serde_json::json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}),
        ] {
            out.extend(acc.handle(&chunk));
        }
        out.push(acc.finish());
        assert_eq!(out.len(), 3);
        match out.last().unwrap() {
            StreamEvent::Done {
                stop_reason,
                usage,
                message,
            } => {
                assert_eq!(*stop_reason, StopReason::Stop);
                assert_eq!(usage.output, 2);
                assert_eq!(message.content, vec![ContentBlock::text("Hello")]);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }
}
