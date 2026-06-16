//! Tests for [`super`]. Extracted from `mod.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use spawningpool::ai::{Api, Reasoning};
use spawningpool::{ModelDef, ProviderDef, Registry, Specialist};

/// A registry with one provider, one model, and one specialist, each keyed
/// the way the app keys them (providers/specialists by name, models by id).
fn registry() -> Registry {
    let mut registry = Registry::default();
    registry.providers.insert(
        "anthropic".into(),
        ProviderDef {
            name: "anthropic".into(),
            api: Api::AnthropicMessages,
            base_url: "https://api.anthropic.com".into(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
            constrained_decoding: false,
        },
    );
    registry.models.insert(
        "claude-opus".into(),
        ModelDef {
            id: "claude-opus".into(),
            name: "Claude Opus".into(),
            provider: "anthropic".into(),
            max_tokens: 4096,
            context_window: 200_000,
        },
    );
    registry.specialists.insert(
        "classifier".into(),
        Specialist {
            name: "classifier".into(),
            provider: "anthropic".into(),
            model: "claude-opus".into(),
            system_prompt: "sort it".into(),
            tools: Vec::new(),
            constraint: None,
            reasoning: Reasoning::Off,
            stream: false,
        },
    );
    registry
}

/// Editing any registry entity must hand the editor that entity's populated
/// JSON — never a blank buffer. Covers all three editable kinds.
#[test]
fn entity_json_serializes_each_kind_with_its_fields() {
    let reg = registry();

    let (key, json) = entity_json(&reg, &EditTarget::Provider("anthropic".into())).unwrap();
    assert_eq!(key, "anthropic");
    assert!(json.contains("\"name\": \"anthropic\""), "{json}");
    assert!(json.contains("base_url"), "{json}");

    let (key, json) = entity_json(&reg, &EditTarget::Model("claude-opus".into())).unwrap();
    assert_eq!(key, "claude-opus");
    assert!(json.contains("\"id\": \"claude-opus\""), "{json}");
    assert!(json.contains("context_window"), "{json}");

    let (key, json) = entity_json(&reg, &EditTarget::Specialist("classifier".into())).unwrap();
    assert_eq!(key, "classifier");
    assert!(json.contains("\"name\": \"classifier\""), "{json}");
    assert!(json.contains("system_prompt"), "{json}");
}

/// A missing entity is a reported error, not a blank edit.
#[test]
fn entity_json_reports_a_missing_entity() {
    let reg = registry();
    let err = entity_json(&reg, &EditTarget::Model("ghost".into())).unwrap_err();
    assert!(err.contains("no such model 'ghost'"), "{err}");
}

/// Applying edited JSON updates the entity and re-keys it when its identity
/// field (name / id) changed, for each editable kind.
#[test]
fn apply_entity_json_round_trips_and_rekeys() {
    // Provider: round-trip an edited base_url under the same key.
    let mut reg = registry();
    let json = r#"{"name":"anthropic","api":"anthropic-messages","base_url":"https://example.test","api_key_env":"ANTHROPIC_API_KEY","constrained_decoding":false}"#;
    apply_entity_json(
        &mut reg,
        &EditTarget::Provider("anthropic".into()),
        "anthropic",
        json,
    )
    .unwrap();
    assert_eq!(reg.providers["anthropic"].base_url, "https://example.test");

    // Model: renaming the id re-keys the map.
    let json = r#"{"id":"claude-opus-4","name":"Claude Opus","provider":"anthropic","max_tokens":4096,"context_window":200000}"#;
    apply_entity_json(
        &mut reg,
        &EditTarget::Model("claude-opus".into()),
        "claude-opus",
        json,
    )
    .unwrap();
    assert!(!reg.models.contains_key("claude-opus"));
    assert!(reg.models.contains_key("claude-opus-4"));

    // Specialist: renaming the name re-keys the map.
    let json = r#"{"name":"sorter","provider":"anthropic","model":"claude-opus-4","system_prompt":"sort it","tools":[],"constraint":null,"reasoning":"off","stream":false}"#;
    apply_entity_json(
        &mut reg,
        &EditTarget::Specialist("classifier".into()),
        "classifier",
        json,
    )
    .unwrap();
    assert!(!reg.specialists.contains_key("classifier"));
    assert!(reg.specialists.contains_key("sorter"));
}

/// A specialist whose edited JSON is invalid (both tools and a constraint)
/// is rejected, leaving the registry untouched.
#[test]
fn apply_entity_json_rejects_an_invalid_specialist() {
    let mut reg = registry();
    let json = r#"{"name":"classifier","provider":"anthropic","model":"claude-opus","system_prompt":"","tools":["a"],"constraint":"a","reasoning":"off","stream":false}"#;
    let err = apply_entity_json(
        &mut reg,
        &EditTarget::Specialist("classifier".into()),
        "classifier",
        json,
    )
    .unwrap_err();
    assert!(err.contains("tools and a constraint"), "{err}");
    // The original specialist is still intact.
    assert!(reg.specialists.contains_key("classifier"));
}

#[test]
fn tab_hit_testing_matches_render_layout() {
    // Leading space at x=0 hits nothing.
    assert_eq!(tab_at_x(0), None);
    // "[Providers]" spans x=1..12.
    assert_eq!(tab_at_x(1), Some(0));
    assert_eq!(tab_at_x(11), Some(0));
    // The space between chips hits nothing.
    assert_eq!(tab_at_x(12), None);
    // "[Specialists]" starts at x=13.
    assert_eq!(tab_at_x(13), Some(1));
    // Far right is past the tools chip.
    assert_eq!(tab_at_x(200), None);
}
