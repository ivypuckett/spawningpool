//! Adapter for the OpenAI Chat Completions API (`openai-completions`).
//!
//! This single adapter serves LM Studio and any other OpenAI-compatible
//! endpoint; they differ only by `base_url` and (optional) auth. It folds the
//! unified `system` field into `messages[0]`, emits `tool_calls`, and maps the
//! responses back into the unified types.
//!
//! Split across [`request`] (outbound serialization), [`response`] (the
//! non-streaming reply), and [`stream`] (SSE accumulation); the two small
//! helpers shared by those submodules ([`parse_args`], [`map_finish_reason`])
//! live here.

mod request;
mod response;
mod stream;

use async_trait::async_trait;
use futures::StreamExt;

use crate::ai::message::StopReason;
use crate::ai::model::{Context, Model};
use crate::ai::provider::{CompleteOptions, Completion, Error, EventStream, Provider};
use crate::ai::sse;

use request::{build_request, constrained_schema, request, synthesize_constrained_call};
use response::WireResponse;
use stream::StreamAccumulator;

pub(crate) use request::list_models;

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
        let parsed: WireResponse = serde_json::from_str(&text).map_err(|_| {
            let preview = text.chars().take(200).collect::<String>();
            Error::Parse(format!(
                "server response was not in the expected format; got: {preview}"
            ))
        })?;
        let mut completion = parsed.into_completion()?;
        if let Some((name, _)) = constrained_schema(ctx, opts) {
            completion.message.content =
                synthesize_constrained_call(&completion.message.content, name);
        }
        Ok(completion)
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
                let value: serde_json::Value = serde_json::from_str(&line)
                    .map_err(|_| Error::Parse(format!("stream contained an unexpected event: {line}")))?;
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

fn map_finish_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("stop") => StopReason::Stop,
        Some("length") => StopReason::Length,
        Some("tool_calls") => StopReason::ToolUse,
        Some("content_filter") => StopReason::Refusal,
        _ => StopReason::Error,
    }
}

fn parse_args(raw: &str) -> serde_json::Value {
    if raw.trim().is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
