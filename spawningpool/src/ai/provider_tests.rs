//! Tests for [`super`]. Extracted from `provider.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

#[test]
fn reasoning_parses_and_displays_each_level() {
    for (text, level) in [
        ("off", Reasoning::Off),
        ("low", Reasoning::Low),
        ("medium", Reasoning::Medium),
        ("high", Reasoning::High),
    ] {
        assert_eq!(text.parse::<Reasoning>(), Ok(level));
        assert_eq!(level.to_string(), text);
    }
    let err = "ultra".parse::<Reasoning>().unwrap_err();
    assert!(err.contains("off|low|medium|high"));
}

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
