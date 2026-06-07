//! Models, the API protocols they speak, and the request context.
//!
//! A model is *data*: which protocol (`Api`) to use, where to send the
//! request (`base_url`), and what it costs. Capabilities and pricing being
//! declarative means cost tracking and validation stay generic — the core
//! never needs a per-model match arm.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ai::message::{Cost, Message};

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
            other => Err(format!("unknown api: {other}")),
        }
    }
}

/// Per-million-token pricing in USD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CostRates {
    pub input: f64,
    pub output: f64,
}

impl CostRates {
    pub const FREE: CostRates = CostRates {
        input: 0.0,
        output: 0.0,
    };
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
    pub cost: CostRates,
}

impl Model {
    /// Compute the dollar cost of a response given its token usage.
    pub fn cost_for(&self, input_tokens: u32, output_tokens: u32) -> Cost {
        let input = input_tokens as f64 / 1_000_000.0 * self.cost.input;
        let output = output_tokens as f64 / 1_000_000.0 * self.cost.output;
        Cost {
            input,
            output,
            total: input + output,
        }
    }
}

/// A tool the model may call.
///
/// `parameters` is an untyped JSON Schema for now; see the FUTURE_AGENT note in
/// [`crate::ai`] for the planned typed-validation work.
#[derive(Clone, Debug)]
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
        assert!(Api::from_str("nope").is_err());
    }

    #[test]
    fn cost_is_computed_from_per_million_rates() {
        let model = Model {
            id: "m".into(),
            name: "m".into(),
            api: Api::AnthropicMessages,
            provider: "anthropic".into(),
            base_url: "https://example".into(),
            max_tokens: 1000,
            context_window: 1000,
            cost: CostRates {
                input: 5.0,
                output: 25.0,
            },
        };
        let cost = model.cost_for(1_000_000, 2_000_000);
        assert_eq!(cost.input, 5.0);
        assert_eq!(cost.output, 50.0);
        assert_eq!(cost.total, 55.0);
    }

    #[test]
    fn free_rates_produce_zero_cost() {
        let model = Model {
            id: "local".into(),
            name: "local".into(),
            api: Api::OpenAiCompletions,
            provider: "lmstudio".into(),
            base_url: "http://localhost:1234".into(),
            max_tokens: 4096,
            context_window: 8192,
            cost: CostRates::FREE,
        };
        assert_eq!(model.cost_for(100, 200), Cost::default());
    }
}
