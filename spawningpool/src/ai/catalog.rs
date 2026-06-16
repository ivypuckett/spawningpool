//! Constructing models for a local LM Studio server.
//!
//! spawningpool deliberately does **not** embed a catalog of hosted models or
//! their limits — those facts go stale and being their arbiter is a liability
//! (the same reasoning keeps pricing out of [`crate::ai::Usage`]). Models you
//! call are defined in your own registry. The one thing that must live in-tree
//! is constructing a [`Model`] for a local LM Studio server, whose ids are
//! discovered at runtime via [`crate::ai::Client::list_models`].

use crate::ai::model::{Api, Model};

const LMSTUDIO_DEFAULT_BASE_URL: &str = "http://localhost:1234";

/// Default output cap and context window for a discovered/constructed local
/// model, which reports neither over the OpenAI API.
const LMSTUDIO_DEFAULT_MAX_TOKENS: u32 = 4096;
const LMSTUDIO_DEFAULT_CONTEXT_WINDOW: u32 = 8192;

pub(crate) fn lmstudio_base_url() -> String {
    std::env::var("LMSTUDIO_BASE_URL").unwrap_or_else(|_| LMSTUDIO_DEFAULT_BASE_URL.to_string())
}

/// Construct a model for a local LM Studio id, pointing at the configured
/// base URL with conservative defaults.
pub(crate) fn lmstudio_model(id: &str) -> Model {
    Model {
        id: id.to_string(),
        name: id.to_string(),
        api: Api::OpenAiCompletions,
        provider: "lmstudio".to_string(),
        base_url: lmstudio_base_url(),
        max_tokens: LMSTUDIO_DEFAULT_MAX_TOKENS,
        context_window: LMSTUDIO_DEFAULT_CONTEXT_WINDOW,
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
