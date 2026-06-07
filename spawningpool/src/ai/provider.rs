//! The adapter boundary: the [`Provider`] trait and the runtime
//! [`ProviderRegistry`] that selects an adapter by [`Api`].
//!
//! Provider selection happens at runtime, not compile time. The registry maps
//! each `Api` to a boxed adapter; callers can swap in or register their own
//! adapters (including out-of-tree ones) without the core changing. Each
//! adapter's only job is translating between the unified message types and one
//! provider's wire format.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::ai::message::{Message, StopReason, Usage};
use crate::ai::model::{Api, Context};

/// How much the model should reason, mapped onto each provider's native knob
/// (Anthropic `thinking`/`effort`, OpenAI `reasoning_effort`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reasoning {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

/// Per-request options shared across providers.
#[derive(Clone, Debug, Default)]
pub struct CompleteOptions {
    /// Output token cap. Defaults to the model's `max_tokens` when `None`.
    pub max_tokens: Option<u32>,
    pub reasoning: Reasoning,
    /// Explicit API key, overriding any environment variable.
    pub api_key: Option<String>,
}

/// A non-streaming completion.
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// The assistant turn (text, thinking, and/or tool calls).
    pub message: Message,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

/// One normalized streaming event.
///
/// Events are *not* grouped by content block — text and tool-call deltas may
/// interleave — so consumers use `content_index` to reassemble blocks.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    TextDelta {
        content_index: usize,
        delta: String,
    },
    ThinkingDelta {
        content_index: usize,
        delta: String,
    },
    ToolCallDelta {
        content_index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    /// Terminal event with the fully assembled message and usage.
    Done {
        stop_reason: StopReason,
        usage: Usage,
        message: Message,
    },
}

/// A stream of normalized events from a provider.
pub type EventStream = BoxStream<'static, Result<StreamEvent, Error>>;

/// One provider adapter. Stateless: the shared HTTP client is passed in.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a request and return the full response.
    async fn complete(
        &self,
        http: &reqwest::Client,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<Completion, Error>;

    /// Send a request and return a stream of normalized events.
    async fn stream(
        &self,
        http: &reqwest::Client,
        model: &Model,
        ctx: &Context,
        opts: &CompleteOptions,
    ) -> Result<EventStream, Error>;
}

use crate::ai::model::Model;

/// Runtime map from `Api` to adapter. Construct with [`ProviderRegistry::with_builtins`]
/// for the shipped adapters, or [`ProviderRegistry::new`] for an empty one you
/// populate yourself.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: HashMap<Api, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        ProviderRegistry {
            providers: HashMap::new(),
        }
    }

    /// A registry with the built-in Anthropic and OpenAI-compatible adapters.
    pub fn with_builtins() -> Self {
        let mut registry = ProviderRegistry::new();
        registry.register(
            Api::AnthropicMessages,
            Arc::new(crate::ai::providers::anthropic::Anthropic),
        );
        registry.register(
            Api::OpenAiCompletions,
            Arc::new(crate::ai::providers::openai::OpenAi),
        );
        registry
    }

    /// Register (or replace) the adapter for an `Api`.
    pub fn register(&mut self, api: Api, provider: Arc<dyn Provider>) {
        self.providers.insert(api, provider);
    }

    /// Look up the adapter for an `Api`.
    pub fn get(&self, api: Api) -> Option<Arc<dyn Provider>> {
        self.providers.get(&api).cloned()
    }
}

/// Errors from model requests.
#[derive(Debug)]
pub enum Error {
    /// Transport-level failure.
    Http(reqwest::Error),
    /// The provider returned a non-success status.
    Api { status: u16, message: String },
    /// A response could not be parsed into the unified types.
    Parse(String),
    /// Misconfiguration: unknown provider/model, missing API key, etc.
    Config(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(e) => write!(f, "http error: {e}"),
            Error::Api { status, message } => write!(f, "api error {status}: {message}"),
            Error::Parse(m) => write!(f, "parse error: {m}"),
            Error::Config(m) => write!(f, "config error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_registry_resolves_both_apis() {
        let registry = ProviderRegistry::with_builtins();
        assert!(registry.get(Api::AnthropicMessages).is_some());
        assert!(registry.get(Api::OpenAiCompletions).is_some());
    }

    #[test]
    fn empty_registry_resolves_nothing() {
        let registry = ProviderRegistry::new();
        assert!(registry.get(Api::AnthropicMessages).is_none());
    }
}
