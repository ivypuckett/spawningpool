//! Outbound serialization: the unified [`Context`]/[`CompleteOptions`] lowered
//! into the OpenAI Chat Completions request body, plus model discovery.

use serde::Serialize;

use crate::ai::message::{ContentBlock, Message, Role};
use crate::ai::model::{Context, Model};
use crate::ai::provider::{CompleteOptions, Error, Reasoning};

#[derive(Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
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

pub(super) fn build_request(
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
    let all_tools: Vec<WireTool> = ctx
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
        .collect();
    // A forced tool call is realized one of two ways. With constrained decoding
    // the model is grammar-constrained to the tool's argument schema via
    // `response_format` (no tools sent — the harness synthesizes the call from
    // the JSON). Otherwise we force it with `tool_choice: "required"`, which —
    // because a constrained specialist sends only that one tool — forces exactly
    // it, and is far more portable than the per-function object form.
    let (tools, tool_choice, response_format) = match constrained_schema(ctx, opts) {
        Some((name, schema)) => (Vec::new(), None, Some(json_schema_format(name, schema))),
        None if opts.tool_choice.is_some() => (all_tools, Some("required"), None),
        None => (all_tools, None, None),
    };
    let request = WireRequest {
        model: model.id.clone(),
        messages,
        max_tokens: opts.max_tokens.unwrap_or(model.max_tokens),
        tools,
        tool_choice,
        response_format,
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

/// When constrained decoding is requested for a forced tool call, return that
/// tool's name and parameter schema. The tool must be present in the request
/// (`build_context` resolves it); if it isn't, we return `None` and fall back to
/// `tool_choice` forcing instead.
pub(super) fn constrained_schema<'a>(
    ctx: &'a Context,
    opts: &'a CompleteOptions,
) -> Option<(&'a str, serde_json::Value)> {
    if !opts.constrained_decoding {
        return None;
    }
    let name = opts.tool_choice.as_deref()?;
    let schema = ctx
        .tools
        .iter()
        .find(|t| t.name == name)?
        .parameters
        .clone();
    Some((name, schema))
}

/// A strict `json_schema` response_format built from a tool's parameter schema,
/// so the model's output is grammar-constrained to valid arguments.
fn json_schema_format(name: &str, mut schema: serde_json::Value) -> serde_json::Value {
    // OpenAI strict mode requires `additionalProperties: false` on the object.
    if let Some(obj) = schema.as_object_mut() {
        obj.insert(
            "additionalProperties".into(),
            serde_json::Value::Bool(false),
        );
    }
    serde_json::json!({
        "type": "json_schema",
        "json_schema": { "name": name, "schema": schema, "strict": true },
    })
}

/// In constrained-decoding mode the model returns the tool's arguments as JSON
/// text rather than a `tool_call`, so synthesize the call the run loop expects.
pub(super) fn synthesize_constrained_call(
    content: &[ContentBlock],
    tool: &str,
) -> Vec<ContentBlock> {
    let args = content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("");
    vec![ContentBlock::ToolCall {
        id: format!("constrained_{tool}"),
        name: tool.to_string(),
        arguments: super::parse_args(args),
    }]
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

pub(super) fn request(
    http: &reqwest::Client,
    model: &Model,
    opts: &CompleteOptions,
    body: &serde_json::Value,
) -> reqwest::RequestBuilder {
    let base = model.base_url.trim_end_matches("/v1");
    let mut builder = http
        .post(format!("{base}/v1/chat/completions"))
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
