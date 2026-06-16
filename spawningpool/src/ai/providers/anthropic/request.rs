//! Outbound serialization and HTTP: the unified [`Context`]/[`CompleteOptions`]
//! lowered into a `/v1/messages` request body and sent.

use serde::Serialize;

use crate::ai::message::{ContentBlock, Message, Role};
use crate::ai::model::{Context, Model};
use crate::ai::provider::{CompleteOptions, Error, Reasoning};

const API_VERSION: &str = "2023-06-01";

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
    tool_choice: Option<WireToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<Thinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<OutputConfig>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Serialize)]
pub(super) struct WireMessage {
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
struct WireToolChoice {
    #[serde(rename = "type")]
    kind: &'static str,
    name: String,
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

pub(super) fn build_request(
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
        tool_choice: opts.tool_choice.as_ref().map(|name| WireToolChoice {
            kind: "tool",
            name: name.clone(),
        }),
        // Opus 4.8/4.7 accept only adaptive thinking; effort carries the level.
        thinking: thinking.map(|_| Thinking { kind: "adaptive" }),
        output_config: output_config.map(|effort| OutputConfig { effort }),
        stream,
    };
    serde_json::to_value(request).expect("request serializes")
}

pub(super) fn to_wire_message(message: &Message) -> WireMessage {
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

pub(super) async fn send(
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

pub(super) async fn send_streaming(
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
