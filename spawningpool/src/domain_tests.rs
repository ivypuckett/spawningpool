//! Tests for [`super`] — the definition layer. Extracted from `domain.rs`
//! to keep the production module lean; included via `#[path]` so it stays a
//! child module with access to private items.

use super::*;

fn anthropic_provider() -> ProviderDef {
    ProviderDef {
        name: "anthropic".to_string(),
        api: Api::AnthropicMessages,
        base_url: "https://api.anthropic.com".to_string(),
        api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
        constrained_decoding: false,
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
fn tool_def_lowers_params_into_required_typed_props() {
    let tool = ToolDef {
        name: "deploy".to_string(),
        script: PathBuf::from("deploy.sh"),
        description: "Deploy a service".to_string(),
        params: vec![
            Param {
                name: "env".to_string(),
                ty: Type::String,
            },
            Param {
                name: "replicas".to_string(),
                ty: Type::Number,
            },
        ],
        output: None,
    }
    .to_tool();

    assert_eq!(tool.name, "deploy");
    assert_eq!(tool.parameters["type"], "object");
    // Each param lowers to its declared type.
    assert_eq!(tool.parameters["properties"]["env"]["type"], "string");
    assert_eq!(tool.parameters["properties"]["replicas"]["type"], "number");
    assert_eq!(
        tool.parameters["required"],
        serde_json::json!(["env", "replicas"])
    );
}

#[test]
fn resolve_model_pairs_specialist_provider_and_model() {
    let mut registry = Registry::default();
    registry
        .providers
        .insert("anthropic".to_string(), anthropic_provider());
    registry
        .models
        .insert("claude-opus-4-8".to_string(), opus());
    let specialist = Specialist {
        name: "x".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-opus-4-8".to_string(),
        system_prompt: String::new(),
        tools: vec![],
        constraint: None,
        reasoning: Reasoning::default(),
        stream: false,
    };

    let model = registry.resolve_model(&specialist).unwrap();
    assert_eq!(model.id, "claude-opus-4-8");
    assert_eq!(model.api, Api::AnthropicMessages);
}

#[test]
fn complete_options_carry_reasoning_and_constraint_as_tool_choice() {
    let specialist = Specialist {
        name: "classifier".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-opus-4-8".to_string(),
        system_prompt: String::new(),
        tools: vec![],
        constraint: Some("classify".to_string()),
        reasoning: Reasoning::Low,
        stream: false,
    };
    let opts = specialist.complete_options();
    assert_eq!(opts.tool_choice.as_deref(), Some("classify"));
    assert_eq!(opts.reasoning, Reasoning::Low);

    // No constraint -> the model chooses.
    let unconstrained = Specialist {
        constraint: None,
        ..specialist
    };
    assert_eq!(unconstrained.complete_options().tool_choice, None);
}

#[test]
fn validate_rejects_both_tools_and_constraint() {
    let mut specialist = Specialist {
        name: "x".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-opus-4-8".to_string(),
        system_prompt: String::new(),
        tools: vec!["a".to_string()],
        constraint: Some("a".to_string()),
        reasoning: Reasoning::Off,
        stream: false,
    };
    assert!(specialist.validate().is_err());

    // Tools only is fine.
    specialist.constraint = None;
    assert!(specialist.validate().is_ok());

    // Constraint only is fine.
    specialist.tools = vec![];
    specialist.constraint = Some("a".to_string());
    assert!(specialist.validate().is_ok());

    // A constraint with reasoning on is rejected.
    specialist.reasoning = Reasoning::High;
    let err = specialist.validate().unwrap_err();
    assert!(err.contains("incompatible with reasoning"));

    // The same reasoning without a constraint is fine.
    specialist.constraint = None;
    assert!(specialist.validate().is_ok());
}

#[test]
fn tool_names_prefers_the_constraint_when_set() {
    let mut specialist = Specialist {
        name: "x".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-opus-4-8".to_string(),
        system_prompt: String::new(),
        tools: vec!["a".to_string(), "b".to_string()],
        constraint: None,
        reasoning: Reasoning::Off,
        stream: false,
    };
    assert_eq!(specialist.tool_names(), ["a".to_string(), "b".to_string()]);

    specialist.tools = vec![];
    specialist.constraint = Some("forced".to_string());
    assert_eq!(specialist.tool_names(), ["forced".to_string()]);
}

#[test]
fn resolve_model_reports_a_missing_provider() {
    let registry = Registry::default();
    let specialist = Specialist {
        name: "x".to_string(),
        provider: "ghost".to_string(),
        model: "nope".to_string(),
        system_prompt: String::new(),
        tools: vec![],
        constraint: None,
        reasoning: Reasoning::default(),
        stream: false,
    };
    assert_eq!(
        registry.resolve_model(&specialist),
        Err("unknown provider: ghost".to_string())
    );
}

fn populated_registry() -> Registry {
    let mut registry = Registry::default();
    registry
        .providers
        .insert("anthropic".to_string(), anthropic_provider());
    registry.models.insert(
        "claude".to_string(),
        ModelDef {
            id: "claude".to_string(),
            name: "Claude".to_string(),
            provider: "anthropic".to_string(),
            max_tokens: 1024,
            context_window: 200_000,
        },
    );
    registry
}

fn spec(provider: &str, model: &str, tools: Vec<String>, constraint: Option<String>) -> Specialist {
    Specialist {
        name: "spec".to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        system_prompt: String::new(),
        tools,
        constraint,
        reasoning: Reasoning::Off,
        stream: false,
    }
}

#[test]
fn missing_model_ref_flags_an_undefined_provider() {
    let registry = populated_registry();
    assert_eq!(missing_provider(&registry, "anthropic"), None);
    assert_eq!(
        missing_provider(&registry, "ghost"),
        Some(MissingRef {
            kind: EntityKind::Provider,
            name: "ghost".to_string(),
        })
    );
}

fn missing_provider(registry: &Registry, provider: &str) -> Option<MissingRef> {
    registry.missing_model_ref(&ModelDef {
        provider: provider.to_string(),
        ..opus()
    })
}

#[test]
fn missing_specialist_ref_reports_provider_then_model_then_tool() {
    let registry = populated_registry();
    // The caller supplies tool existence; here only "ping" exists.
    let tool_exists = |name: &str| name == "ping";
    // Everything present: no missing reference.
    assert_eq!(
        registry.missing_specialist_ref(
            &spec("anthropic", "claude", vec!["ping".into()], None),
            tool_exists
        ),
        None
    );
    // Provider is checked first.
    assert_eq!(
        registry.missing_specialist_ref(
            &spec("ghost", "nope", vec!["absent".into()], None),
            tool_exists
        ),
        Some(MissingRef {
            kind: EntityKind::Provider,
            name: "ghost".to_string(),
        })
    );
    // Then model.
    assert_eq!(
        registry.missing_specialist_ref(&spec("anthropic", "nope", vec![], None), tool_exists),
        Some(MissingRef {
            kind: EntityKind::Model,
            name: "nope".to_string(),
        })
    );
    // Then tools — including a constrained tool.
    assert_eq!(
        registry.missing_specialist_ref(
            &spec("anthropic", "claude", vec![], Some("forced".into())),
            tool_exists
        ),
        Some(MissingRef {
            kind: EntityKind::Tool,
            name: "forced".to_string(),
        })
    );
}

#[test]
fn referrers_lists_specialists_before_models_sorted() {
    let mut registry = populated_registry();
    registry.specialists.insert(
        "spec".into(),
        spec("anthropic", "claude", vec!["ping".into()], None),
    );

    // A provider is referenced by the specialist and the model under it.
    assert_eq!(
        registry.referrers(EntityKind::Provider, "anthropic"),
        vec![
            Referrer {
                kind: EntityKind::Specialist,
                name: "spec".to_string()
            },
            Referrer {
                kind: EntityKind::Model,
                name: "claude".to_string()
            },
        ]
    );
    assert_eq!(
        registry.referrers(EntityKind::Model, "claude"),
        vec![Referrer {
            kind: EntityKind::Specialist,
            name: "spec".to_string()
        }]
    );
    assert_eq!(
        registry.referrers(EntityKind::Tool, "ping"),
        vec![Referrer {
            kind: EntityKind::Specialist,
            name: "spec".to_string()
        }]
    );
    // An unreferenced name has none.
    assert!(registry
        .referrers(EntityKind::Provider, "openai")
        .is_empty());
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
    registry.specialists.insert(
        "netop".to_string(),
        Specialist {
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
