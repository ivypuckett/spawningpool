//! The model catalog: looking up models by provider and id at runtime.
//!
//! Claude models are embedded as static data (ids, context windows, pricing).
//! LM Studio serves whatever local model you point it at, so its models are
//! constructed on demand; use [`crate::ai::Client::list_models`] to discover
//! what a running LM Studio instance actually has loaded.

use crate::ai::model::{Api, CostRates, Model};
use crate::ai::provider::Error;

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const LMSTUDIO_DEFAULT_BASE_URL: &str = "http://localhost:1234";

/// Default output cap and context window for a discovered/constructed local
/// model, which reports neither over the OpenAI API.
const LMSTUDIO_DEFAULT_MAX_TOKENS: u32 = 4096;
const LMSTUDIO_DEFAULT_CONTEXT_WINDOW: u32 = 8192;

struct CatalogEntry {
    id: &'static str,
    name: &'static str,
    max_tokens: u32,
    context_window: u32,
    input_cost: f64,
    output_cost: f64,
}

/// Embedded Claude catalog. Pricing is USD per million tokens.
const ANTHROPIC_MODELS: &[CatalogEntry] = &[
    CatalogEntry {
        id: "claude-opus-4-8",
        name: "Claude Opus 4.8",
        max_tokens: 128_000,
        context_window: 1_000_000,
        input_cost: 5.0,
        output_cost: 25.0,
    },
    CatalogEntry {
        id: "claude-opus-4-7",
        name: "Claude Opus 4.7",
        max_tokens: 128_000,
        context_window: 1_000_000,
        input_cost: 5.0,
        output_cost: 25.0,
    },
    CatalogEntry {
        id: "claude-opus-4-6",
        name: "Claude Opus 4.6",
        max_tokens: 128_000,
        context_window: 1_000_000,
        input_cost: 5.0,
        output_cost: 25.0,
    },
    CatalogEntry {
        id: "claude-sonnet-4-6",
        name: "Claude Sonnet 4.6",
        max_tokens: 64_000,
        context_window: 1_000_000,
        input_cost: 3.0,
        output_cost: 15.0,
    },
    CatalogEntry {
        id: "claude-haiku-4-5",
        name: "Claude Haiku 4.5",
        max_tokens: 64_000,
        context_window: 200_000,
        input_cost: 1.0,
        output_cost: 5.0,
    },
];

pub(crate) fn lmstudio_base_url() -> String {
    std::env::var("LMSTUDIO_BASE_URL").unwrap_or_else(|_| LMSTUDIO_DEFAULT_BASE_URL.to_string())
}

fn entry_to_model(entry: &CatalogEntry) -> Model {
    Model {
        id: entry.id.to_string(),
        name: entry.name.to_string(),
        api: Api::AnthropicMessages,
        provider: "anthropic".to_string(),
        base_url: ANTHROPIC_BASE_URL.to_string(),
        max_tokens: entry.max_tokens,
        context_window: entry.context_window,
        cost: CostRates {
            input: entry.input_cost,
            output: entry.output_cost,
        },
    }
}

/// Construct a model for a local LM Studio id, pointing at the configured
/// base URL with zero cost and conservative defaults.
pub(crate) fn lmstudio_model(id: &str) -> Model {
    Model {
        id: id.to_string(),
        name: id.to_string(),
        api: Api::OpenAiCompletions,
        provider: "lmstudio".to_string(),
        base_url: lmstudio_base_url(),
        max_tokens: LMSTUDIO_DEFAULT_MAX_TOKENS,
        context_window: LMSTUDIO_DEFAULT_CONTEXT_WINDOW,
        cost: CostRates::FREE,
    }
}

/// The providers this catalog knows about.
pub fn get_providers() -> Vec<&'static str> {
    vec!["anthropic", "lmstudio"]
}

/// Look up a model by provider and id. Network-free for both providers: the
/// Claude id is validated against the embedded catalog, and an LM Studio id is
/// taken on trust (use [`crate::ai::Client::list_models`] to discover ids).
pub fn get_model(provider: &str, id: &str) -> Result<Model, Error> {
    match provider {
        "anthropic" => ANTHROPIC_MODELS
            .iter()
            .find(|e| e.id == id)
            .map(entry_to_model)
            .ok_or_else(|| Error::Config(format!("unknown anthropic model: {id}"))),
        "lmstudio" => Ok(lmstudio_model(id)),
        other => Err(Error::Config(format!("unknown provider: {other}"))),
    }
}

/// All statically-known models for a provider. LM Studio returns an empty list
/// here because its models are discovered at runtime, not embedded.
pub fn get_models(provider: &str) -> Vec<Model> {
    match provider {
        "anthropic" => ANTHROPIC_MODELS.iter().map(entry_to_model).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_anthropic_model_resolves_with_pricing() {
        let model = get_model("anthropic", "claude-opus-4-8").unwrap();
        assert_eq!(model.api, Api::AnthropicMessages);
        assert_eq!(model.cost.input, 5.0);
        assert_eq!(model.cost.output, 25.0);
        assert_eq!(model.context_window, 1_000_000);
        assert_eq!(model.base_url, ANTHROPIC_BASE_URL);
    }

    #[test]
    fn unknown_anthropic_model_is_an_error() {
        assert!(get_model("anthropic", "claude-nonexistent").is_err());
    }

    #[test]
    fn lmstudio_model_is_constructed_on_trust() {
        let model = get_model("lmstudio", "qwen2.5-coder-7b").unwrap();
        assert_eq!(model.api, Api::OpenAiCompletions);
        assert_eq!(model.provider, "lmstudio");
        assert_eq!(model.cost, CostRates::FREE);
    }

    #[test]
    fn unknown_provider_is_an_error() {
        assert!(get_model("openai", "gpt-4").is_err());
    }

    #[test]
    fn anthropic_catalog_is_listed() {
        let ids: Vec<_> = get_models("anthropic").into_iter().map(|m| m.id).collect();
        assert!(ids.contains(&"claude-opus-4-8".to_string()));
        assert!(get_models("lmstudio").is_empty());
    }
}
