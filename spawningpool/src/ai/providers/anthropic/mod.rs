//! Adapter for the Anthropic Messages API (`anthropic-messages`).
//!
//! Translates the unified message types to and from the `/v1/messages` wire
//! format. This is the adapter's only job; nothing about Anthropic leaks past
//! this module.
//!
//! Split across [`request`] (outbound serialization and HTTP), [`response`]
//! (the non-streaming reply), and [`stream`] (SSE accumulation); the stop-reason
//! mapping shared by the latter two lives here.

mod request;
mod response;
mod stream;

use async_trait::async_trait;
use futures::StreamExt;

use crate::ai::message::StopReason;
use crate::ai::model::{Context, Model};
use crate::ai::provider::{CompleteOptions, Completion, Error, EventStream, Provider};
use crate::ai::sse;

use request::{build_request, send, send_streaming};
use response::WireResponse;
use stream::StreamAccumulator;

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
        let parsed: WireResponse = serde_json::from_str(&resp).map_err(|_| {
            let preview = resp.chars().take(200).collect::<String>();
            Error::Parse(format!(
                "server response was not in the expected format; got: {preview}"
            ))
        })?;
        Ok(parsed.into_completion())
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
        let stream = async_stream::try_stream! {
            let mut lines = Box::pin(sse::data_lines(resp));
            let mut acc = StreamAccumulator::default();
            while let Some(line) = lines.next().await {
                let line = line?;
                let value: serde_json::Value = serde_json::from_str(&line)
                    .map_err(|_| Error::Parse(format!("stream contained an unexpected event: {line}")))?;
                if let Some(event) = acc.handle(&value) {
                    yield event;
                }
            }
        };
        Ok(Box::pin(stream))
    }
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

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;
