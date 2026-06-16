//! The definition layer: persisted templates that compile down into the
//! runtime [`crate::ai`] types.
//!
//! Nothing here talks to a provider or the filesystem. A [`ProviderDef`]/
//! [`ModelDef`]/[`Specialist`] is plain, serializable data that `spawningpool define`
//! writes and `spawningpool list` reads. The bridges ([`ModelDef::resolve`],
//! [`Registry::resolve_model`]) lower these definitions into the [`crate::ai::Model`]
//! the client executes. Tools are the exception: they live as scripts in a
//! folder (see [`crate::tools`]), not in the [`Registry`], so a [`ToolDef`] here
//! is a derived view of one of those scripts rather than persisted data.
//!
//! Provider/model split follows option A: a [`ModelDef`] references a provider
//! by name and *derives* its `api`/`base_url` from the [`ProviderDef`] rather
//! than carrying its own copy.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ai::{Api, CompleteOptions, Model, Reasoning, Tool};
use crate::types::{Param, Type};

/// Which kind of registry entity a reference or referrer points at. Carried by
/// [`MissingRef`] and [`Referrer`] so a front-end can describe it however it
/// likes; [`Display`](std::fmt::Display) gives the lowercase noun.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    Provider,
    Model,
    Tool,
    Specialist,
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EntityKind::Provider => "provider",
            EntityKind::Model => "model",
            EntityKind::Tool => "tool",
            EntityKind::Specialist => "specialist",
        })
    }
}

/// A reference, on a definition, to an entity the registry doesn't contain.
/// Holds only the facts (what kind, which name); how to phrase the fix is left
/// to the caller, so the CLI and a UI can render it differently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingRef {
    pub kind: EntityKind,
    pub name: String,
}

/// An entity that references some target, collected before a delete so the
/// caller can warn about the references it would leave dangling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Referrer {
    pub kind: EntityKind,
    pub name: String,
}

/// A defined provider (`spawningpool define provider`): a name bound to a wire protocol,
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
    /// Whether this provider's endpoint supports true constrained decoding
    /// (grammar-constrained `response_format`). User-declared, since it can't be
    /// inferred from the wire protocol — two `openai-completions` endpoints can
    /// differ.
    ///
    /// Only `openai-completions` providers honor this: when set, a constrained
    /// specialist realizes its forced call via constrained decoding; otherwise via
    /// the "tool-call trick" (a forced `tool_choice`). The `anthropic-messages`
    /// adapter ignores the flag and always uses native forced tool choice, so
    /// setting it there has no effect.
    #[serde(default)]
    pub constrained_decoding: bool,
}

/// A defined model (`spawningpool define model`). Per option A it names its provider and
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

/// A defined specialist (`spawningpool define specialist`): the (provider, model, system prompt,
/// tools) template that gets instantiated with a user prompt and called.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Specialist {
    pub name: String,
    /// References a [`ProviderDef`] by name.
    pub provider: String,
    /// References a [`ModelDef`] by id.
    pub model: String,
    pub system_prompt: String,
    /// References [`ToolDef`]s by name.
    #[serde(default)]
    pub tools: Vec<String>,
    /// A tool the model is forced to call (via the tool-call trick, or true
    /// constrained decoding when the provider declares it). Consumed when the
    /// specialist is run, not when its context is built.
    #[serde(default)]
    pub constraint: Option<String>,
    #[serde(default)]
    pub reasoning: Reasoning,
    /// Stream the response incrementally rather than awaiting the full
    /// completion. A property of the specialist, not a per-run flag.
    #[serde(default)]
    pub stream: bool,
}

impl Specialist {
    /// The per-request options this specialist implies: its reasoning effort and,
    /// from [`Specialist::constraint`], a forced tool choice. `max_tokens` and
    /// `api_key` are left at their defaults for the caller to fill in.
    pub fn complete_options(&self) -> CompleteOptions {
        CompleteOptions {
            reasoning: self.reasoning,
            tool_choice: self.constraint.clone(),
            ..Default::default()
        }
    }

    /// A specialist exposes EITHER a set of freely-callable [`Specialist::tools`]
    /// (the model decides what to call, and the runner loops until it stops) OR a
    /// single forced [`Specialist::constraint`] (one guaranteed call) — never
    /// both. A forced tool can't be combined with cursory ones at the provider
    /// level, so this rejects the clash up front rather than producing a request
    /// the model can't satisfy.
    ///
    /// A constraint is also incompatible with reasoning: a forced `tool_choice`
    /// combined with extended thinking is rejected by Anthropic (see
    /// [`crate::ai::CompleteOptions::tool_choice`]), so this rejects the pairing
    /// at define time instead of letting it fail as a runtime API error.
    pub fn validate(&self) -> Result<(), String> {
        if self.constraint.is_some() && !self.tools.is_empty() {
            return Err(format!(
                "specialist '{}' sets both tools and a constraint; use one or the other",
                self.name
            ));
        }
        if self.constraint.is_some() && self.reasoning != Reasoning::Off {
            return Err(format!(
                "specialist '{}' forces a tool call with reasoning enabled; a forced tool call is incompatible with reasoning, so set --reasoning off",
                self.name
            ));
        }
        Ok(())
    }

    /// The tools to expose to the model: the freely-callable [`Specialist::tools`],
    /// or — when a [`Specialist::constraint`] is set — just that single forced
    /// tool. A forced `tool_choice` requires its tool to be present in the
    /// request, so the constraint case still needs the tool resolved.
    pub fn tool_names(&self) -> &[String] {
        match &self.constraint {
            Some(constraint) => std::slice::from_ref(constraint),
            None => &self.tools,
        }
    }
}

/// A tool, backed by one executable script in the [`crate::tools`] folder. The
/// script's `# desc:` header becomes the description and its `# params:` header
/// the parameters — see [`crate::summarize`]. This is a derived view read from
/// the script, not persisted data; only [`Serialize`] is needed, for `spawningpool show`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub script: PathBuf,
    pub description: String,
    pub params: Vec<Param>,
    /// The tool's declared `# output:` type (workflow-dsl §3), or `None`.
    pub output: Option<Type>,
}

impl ToolDef {
    /// Lower into the runtime [`Tool`] the model sees, with each parameter
    /// declared as a required property of its declared type (a bare, untyped
    /// param is `string`; see [`crate::types`]).
    pub fn to_tool(&self) -> Tool {
        let properties: serde_json::Map<String, serde_json::Value> = self
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty.to_schema()))
            .collect();
        let required: Vec<&String> = self.params.iter().map(|p| &p.name).collect();
        Tool {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required,
            }),
        }
    }
}

/// The on-disk catalog backing `spawningpool define` / `spawningpool list` / `spawningpool delete`. Tools
/// aren't here — they live as scripts in the [`crate::tools`] folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub providers: HashMap<String, ProviderDef>,
    #[serde(default)]
    pub models: HashMap<String, ModelDef>,
    #[serde(default)]
    pub specialists: HashMap<String, Specialist>,
}

impl Registry {
    /// Resolve a specialist's provider + model into a runtime [`Model`].
    pub fn resolve_model(&self, specialist: &Specialist) -> Result<Model, String> {
        let provider = self
            .providers
            .get(&specialist.provider)
            .ok_or_else(|| format!("unknown provider: {}", specialist.provider))?;
        let model = self
            .models
            .get(&specialist.model)
            .ok_or_else(|| format!("unknown model: {}", specialist.model))?;
        Ok(model.resolve(provider))
    }

    /// The first reference a model makes that the registry can't resolve — only
    /// its provider — or `None` when every reference is satisfied. The gate
    /// `spawningpool define` (or a UI) runs before persisting a model.
    pub fn missing_model_ref(&self, model: &ModelDef) -> Option<MissingRef> {
        if !self.providers.contains_key(&model.provider) {
            return Some(MissingRef {
                kind: EntityKind::Provider,
                name: model.provider.clone(),
            });
        }
        None
    }

    /// The first reference a specialist makes that the registry can't resolve —
    /// its provider, then its model, then each tool — or `None` when all resolve.
    /// Uses [`Specialist::tool_names`], so a constrained specialist's forced tool
    /// is checked too. Tools live in a folder, not the registry, so the caller
    /// supplies `tool_exists` (e.g. [`crate::tools::exists`]) to check them.
    pub fn missing_specialist_ref(
        &self,
        specialist: &Specialist,
        tool_exists: impl Fn(&str) -> bool,
    ) -> Option<MissingRef> {
        if !self.providers.contains_key(&specialist.provider) {
            return Some(MissingRef {
                kind: EntityKind::Provider,
                name: specialist.provider.clone(),
            });
        }
        if !self.models.contains_key(&specialist.model) {
            return Some(MissingRef {
                kind: EntityKind::Model,
                name: specialist.model.clone(),
            });
        }
        for tool in specialist.tool_names() {
            if !tool_exists(tool) {
                return Some(MissingRef {
                    kind: EntityKind::Tool,
                    name: tool.clone(),
                });
            }
        }
        None
    }

    /// Entities that reference `name` as a `target`, so a delete can warn about
    /// the references it would orphan. For a provider that's the specialists
    /// pointing at it plus the models defined under it; specialists are listed
    /// before models, each group sorted by name.
    pub fn referrers(&self, target: EntityKind, name: &str) -> Vec<Referrer> {
        match target {
            EntityKind::Provider => {
                let mut refs = self.referrer_specialists(|s| s.provider == name);
                let mut models: Vec<Referrer> = self
                    .models
                    .values()
                    .filter(|m| m.provider == name)
                    .map(|m| Referrer {
                        kind: EntityKind::Model,
                        name: m.id.clone(),
                    })
                    .collect();
                models.sort_by(|a, b| a.name.cmp(&b.name));
                refs.extend(models);
                refs
            }
            EntityKind::Model => self.referrer_specialists(|s| s.model == name),
            EntityKind::Tool => {
                self.referrer_specialists(|s| s.tool_names().iter().any(|t| t == name))
            }
            EntityKind::Specialist => Vec::new(),
        }
    }

    /// Specialists matching `pred`, as [`Referrer`]s sorted by name.
    fn referrer_specialists(&self, pred: impl Fn(&Specialist) -> bool) -> Vec<Referrer> {
        let mut refs: Vec<Referrer> = self
            .specialists
            .values()
            .filter(|s| pred(s))
            .map(|s| Referrer {
                kind: EntityKind::Specialist,
                name: s.name.clone(),
            })
            .collect();
        refs.sort_by(|a, b| a.name.cmp(&b.name));
        refs
    }
}

#[cfg(test)]
#[path = "domain_tests.rs"]
mod tests;
