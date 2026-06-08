//! Models, the API protocols they speak, and the request context.
//!
//! A model is *data*: which protocol (`Api`) to use and where to send the
//! request (`base_url`). Keeping capabilities declarative means the core never
//! needs a per-model match arm. Pricing is intentionally absent — it goes
//! stale, so cost is left to the caller (see [`crate::ai::Usage`]).

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ai::message::Message;

/// The wire protocol a model speaks.
///
/// This is the axis that selects an adapter — it is deliberately separate from
/// the provider/brand. Many providers (LM Studio, vLLM, Ollama, Groq, …) all
/// speak [`Api::OpenAiCompletions`], so they reuse a single adapter and differ
/// only by `base_url` and auth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Api {
    AnthropicMessages,
    OpenAiCompletions,
}

impl FromStr for Api {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "anthropic-messages" | "anthropic" => Ok(Api::AnthropicMessages),
            "openai-completions" | "openai" => Ok(Api::OpenAiCompletions),
            other => Err(format!(
                "unknown api '{other}'\n\n  Expected one of:\n      \
                 anthropic-messages  (alias: anthropic)\n      \
                 openai-completions  (alias: openai)"
            )),
        }
    }
}

/// A model definition. Plain data, looked up from a catalog or constructed for
/// a local endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: Api,
    pub provider: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub context_window: u32,
}

/// A tool the model may call.
///
/// `parameters` is a JSON Schema, built dynamically at runtime. Tool-call
/// arguments are passed through unvalidated by default; see the FUTURE_AGENT
/// note in [`crate::ai`] for the planned opt-in runtime validator.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Everything that goes into a single request, provider-agnostic.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
}

impl Context {
    /// A context with just a system prompt and a list of messages.
    pub fn new(system: Option<String>, messages: Vec<Message>) -> Self {
        Context {
            system,
            messages,
            tools: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_parses_from_protocol_or_brand_name() {
        assert_eq!(
            Api::from_str("anthropic-messages"),
            Ok(Api::AnthropicMessages)
        );
        assert_eq!(Api::from_str("anthropic"), Ok(Api::AnthropicMessages));
        assert_eq!(
            Api::from_str("openai-completions"),
            Ok(Api::OpenAiCompletions)
        );
        assert_eq!(Api::from_str("openai"), Ok(Api::OpenAiCompletions));

        // An unknown api names the valid options rather than just rejecting.
        let err = Api::from_str("nope").unwrap_err();
        assert!(err.contains("anthropic-messages"));
        assert!(err.contains("openai-completions"));
    }
}
