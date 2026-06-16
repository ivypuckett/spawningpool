//! Tests for [`super`]. Extracted from `catalog.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

#[test]
fn lmstudio_model_is_constructed_on_trust() {
    let model = lmstudio_model("qwen2.5-coder-7b");
    assert_eq!(model.api, Api::OpenAiCompletions);
    assert_eq!(model.provider, "lmstudio");
    assert_eq!(model.id, "qwen2.5-coder-7b");
}

#[test]
fn lmstudio_base_url_defaults_when_env_unset() {
    // Only assert the default when the env var is unset, to avoid depending
    // on the ambient environment.
    if std::env::var_os("LMSTUDIO_BASE_URL").is_none() {
        assert_eq!(lmstudio_base_url(), "http://localhost:1234");
    }
}
