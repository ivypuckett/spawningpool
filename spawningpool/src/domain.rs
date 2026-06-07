//! The definition layer: persisted templates that compile down into the
//! runtime [`crate::ai`] types.
//!
//! Nothing here talks to a provider. A [`ProviderDef`]/[`ModelDef`]/[`Expert`]/
//! [`ToolDef`] is plain, serializable data that `sp define` writes and `sp list`
//! reads. The bridges ([`ToolDef::to_tool`], [`ModelDef::resolve`],
//! [`Registry::build_context`], [`Registry::resolve_model`]) lower these
//! definitions into the [`ai::Context`], [`ai::Tool`], and [`ai::Model`] the
//! client actually executes.
//!
//! Provider/model split follows option A: a [`ModelDef`] references a provider
//! by name and *derives* its `api`/`base_url` from the [`ProviderDef`] rather
//! than carrying its own copy.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ai::{Api, CompleteOptions, Context, Message, Model, Reasoning, Tool};

/// A defined provider (`sp define provider`): a name bound to a wire protocol,
/// endpoint, and the env var holding its API key. Generalizes the catalog's
/// hard-coded "anthropic"/"lmstudio".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDef {
    pub name: String,
    pub api: Api,
    pub base_url: String,
    /// Env var holding the API key, if the provider needs one.
    #[serde(default)]
    pub api_key_env: Option<String>,
}

/// A defined model (`sp define model`). Per option A it names its provider and
/// inherits that provider's `api`/`base_url`; only the model-specific limits
/// live here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDef {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub max_tokens: u32,
    pub context_window: u32,
}

impl ModelDef {
    /// Lower into a runtime [`Model`], drawing `api`/`base_url` from the
    /// provider this model was defined against.
    pub fn resolve(&self, provider: &ProviderDef) -> Model {
        Model {
            id: self.id.clone(),
            name: self.name.clone(),
            api: provider.api,
            provider: provider.name.clone(),
            base_url: provider.base_url.clone(),
            max_tokens: self.max_tokens,
            context_window: self.context_window,
        }
    }
}

/// A defined expert (`sp define expert`): the (provider, model, system prompt,
/// tools) template that gets instantiated with a user prompt and called.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expert {
    pub name: String,
    /// References a [`ProviderDef`] by name.
    pub provider: String,
    /// References a [`ModelDef`] by id.
    pub model: String,
    pub system_prompt: String,
    /// References [`ToolDef`]s by name.
    #[serde(default)]
    pub tools: Vec<String>,
    /// A tool the model is forced to call (constrained decoding). Consumed when
    /// the expert is run, not when its context is built.
    #[serde(default)]
    pub constraint: Option<String>,
    #[serde(default)]
    pub reasoning: Reasoning,
    /// Stream the response incrementally rather than awaiting the full
    /// completion. A property of the expert, not a per-run flag.
    #[serde(default)]
    pub stream: bool,
}

impl Expert {
    /// The per-request options this expert implies: its reasoning effort and,
    /// from [`Expert::constraint`], a forced tool choice. `max_tokens` and
    /// `api_key` are left at their defaults for the caller to fill in.
    pub fn complete_options(&self) -> CompleteOptions {
        CompleteOptions {
            reasoning: self.reasoning,
            tool_choice: self.constraint.clone(),
            ..Default::default()
        }
    }
}

/// A defined tool (`sp define tool`), backed by one Taskfile task. The task's
/// `desc` becomes the description and its referenced `{{.VARS}}` become the
/// parameters — see [`crate::summarize`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub taskfile: PathBuf,
    /// Task name within the Taskfile.
    pub task: String,
    pub description: String,
    pub params: Vec<String>,
}

impl ToolDef {
    /// Lower into the runtime [`Tool`] the model sees, with each parameter
    /// declared as a required string property.
    pub fn to_tool(&self) -> Tool {
        let properties: serde_json::Map<String, serde_json::Value> = self
            .params
            .iter()
            .map(|p| (p.clone(), serde_json::json!({ "type": "string" })))
            .collect();
        Tool {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": self.params,
            }),
        }
    }
}

/// The on-disk catalog backing `sp define` / `sp list` / `sp delete`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub providers: HashMap<String, ProviderDef>,
    #[serde(default)]
    pub models: HashMap<String, ModelDef>,
    #[serde(default)]
    pub experts: HashMap<String, Expert>,
    #[serde(default)]
    pub tools: HashMap<String, ToolDef>,
}

impl Registry {
    /// Resolve an expert's provider + model into a runtime [`Model`].
    pub fn resolve_model(&self, expert: &Expert) -> Result<Model, String> {
        let provider = self
            .providers
            .get(&expert.provider)
            .ok_or_else(|| format!("unknown provider: {}", expert.provider))?;
        let model = self
            .models
            .get(&expert.model)
            .ok_or_else(|| format!("unknown model: {}", expert.model))?;
        Ok(model.resolve(provider))
    }

    /// Resolve an expert's named tools into runtime [`Tool`]s.
    pub fn resolve_tools(&self, expert: &Expert) -> Result<Vec<Tool>, String> {
        expert
            .tools
            .iter()
            .map(|name| {
                self.tools
                    .get(name)
                    .map(ToolDef::to_tool)
                    .ok_or_else(|| format!("unknown tool: {name}"))
            })
            .collect()
    }

    /// Compile an expert + a user prompt into a runtime [`Context`] (system
    /// prompt, the user turn, and the expert's resolved tools).
    pub fn build_context(&self, expert: &Expert, prompt: &str) -> Result<Context, String> {
        let mut ctx = Context::new(
            Some(expert.system_prompt.clone()),
            vec![Message::user(prompt)],
        );
        ctx.tools = self.resolve_tools(expert)?;
        Ok(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ContentBlock;

    fn anthropic_provider() -> ProviderDef {
        ProviderDef {
            name: "anthropic".to_string(),
            api: Api::AnthropicMessages,
            base_url: "https://api.anthropic.com".to_string(),
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
        }
    }

    fn opus() -> ModelDef {
        ModelDef {
            id: "claude-opus-4-8".to_string(),
            name: "Claude Opus 4.8".to_string(),
            provider: "anthropic".to_string(),
            max_tokens: 128_000,
            context_window: 1_000_000,
        }
    }

    #[test]
    fn model_def_inherits_api_and_base_url_from_provider() {
        let model = opus().resolve(&anthropic_provider());
        assert_eq!(model.api, Api::AnthropicMessages);
        assert_eq!(model.base_url, "https://api.anthropic.com");
        assert_eq!(model.provider, "anthropic");
        assert_eq!(model.context_window, 1_000_000);
    }

    #[test]
    fn tool_def_lowers_vars_into_required_string_params() {
        let tool = ToolDef {
            name: "deploy".to_string(),
            taskfile: PathBuf::from("Taskfile.yml"),
            task: "deploy".to_string(),
            description: "Deploy a service".to_string(),
            params: vec!["env".to_string(), "region".to_string()],
        }
        .to_tool();

        assert_eq!(tool.name, "deploy");
        assert_eq!(tool.parameters["type"], "object");
        assert_eq!(tool.parameters["properties"]["env"]["type"], "string");
        assert_eq!(
            tool.parameters["required"],
            serde_json::json!(["env", "region"])
        );
    }

    #[test]
    fn build_context_carries_system_prompt_user_turn_and_tools() {
        let mut registry = Registry::default();
        registry.tools.insert(
            "ping".to_string(),
            ToolDef {
                name: "ping".to_string(),
                taskfile: PathBuf::from("Taskfile.yml"),
                task: "ping".to_string(),
                description: "Ping a host".to_string(),
                params: vec!["host".to_string()],
            },
        );
        let expert = Expert {
            name: "netop".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-opus-4-8".to_string(),
            system_prompt: "You ping hosts.".to_string(),
            tools: vec!["ping".to_string()],
            constraint: None,
            reasoning: Reasoning::Off,
            stream: false,
        };

        let ctx = registry.build_context(&expert, "ping example.com").unwrap();
        assert_eq!(ctx.system.as_deref(), Some("You ping hosts."));
        assert_eq!(ctx.tools.len(), 1);
        assert_eq!(ctx.tools[0].name, "ping");
        assert_eq!(
            ctx.messages[0].content,
            vec![ContentBlock::text("ping example.com")]
        );
    }

    #[test]
    fn resolve_model_pairs_expert_provider_and_model() {
        let mut registry = Registry::default();
        registry
            .providers
            .insert("anthropic".to_string(), anthropic_provider());
        registry
            .models
            .insert("claude-opus-4-8".to_string(), opus());
        let expert = Expert {
            name: "x".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-opus-4-8".to_string(),
            system_prompt: String::new(),
            tools: vec![],
            constraint: None,
            reasoning: Reasoning::default(),
            stream: false,
        };

        let model = registry.resolve_model(&expert).unwrap();
        assert_eq!(model.id, "claude-opus-4-8");
        assert_eq!(model.api, Api::AnthropicMessages);
    }

    #[test]
    fn complete_options_carry_reasoning_and_constraint_as_tool_choice() {
        let expert = Expert {
            name: "classifier".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-opus-4-8".to_string(),
            system_prompt: String::new(),
            tools: vec!["classify".to_string()],
            constraint: Some("classify".to_string()),
            reasoning: Reasoning::Low,
            stream: false,
        };
        let opts = expert.complete_options();
        assert_eq!(opts.tool_choice.as_deref(), Some("classify"));
        assert_eq!(opts.reasoning, Reasoning::Low);

        // No constraint -> the model chooses.
        let unconstrained = Expert {
            constraint: None,
            ..expert
        };
        assert_eq!(unconstrained.complete_options().tool_choice, None);
    }

    #[test]
    fn resolve_reports_missing_references() {
        let registry = Registry::default();
        let expert = Expert {
            name: "x".to_string(),
            provider: "ghost".to_string(),
            model: "nope".to_string(),
            system_prompt: String::new(),
            tools: vec!["absent".to_string()],
            constraint: None,
            reasoning: Reasoning::default(),
            stream: false,
        };
        assert_eq!(
            registry.resolve_model(&expert),
            Err("unknown provider: ghost".to_string())
        );
        assert_eq!(
            registry.resolve_tools(&expert),
            Err("unknown tool: absent".to_string())
        );
    }

    #[test]
    fn registry_round_trips_through_json() {
        let mut registry = Registry::default();
        registry
            .providers
            .insert("anthropic".to_string(), anthropic_provider());
        registry
            .models
            .insert("claude-opus-4-8".to_string(), opus());
        registry.experts.insert(
            "netop".to_string(),
            Expert {
                name: "netop".to_string(),
                provider: "anthropic".to_string(),
                model: "claude-opus-4-8".to_string(),
                system_prompt: "hi".to_string(),
                tools: vec![],
                constraint: Some("ping".to_string()),
                reasoning: Reasoning::High,
                stream: true,
            },
        );

        let json = serde_json::to_string(&registry).unwrap();
        let back: Registry = serde_json::from_str(&json).unwrap();
        assert_eq!(registry, back);
    }
}
