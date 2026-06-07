//! Minimal Server-Sent Events reader shared by the streaming adapters.
//!
//! Both target protocols deliver streams as `data:` lines whose payload is a
//! JSON object (or the `[DONE]` sentinel), so a single helper that yields each
//! `data:` payload serves both.

use futures::{Stream, StreamExt};

use crate::ai::provider::Error;

/// Yield the payload of each SSE `data:` field from a streaming HTTP response.
pub fn data_lines(resp: reqwest::Response) -> impl Stream<Item = Result<String, Error>> {
    async_stream::try_stream! {
        let mut bytes = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk?;
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end_matches(['\r', '\n']);
                if let Some(rest) = line.strip_prefix("data:") {
                    yield rest.trim().to_string();
                }
            }
        }
    }
}
