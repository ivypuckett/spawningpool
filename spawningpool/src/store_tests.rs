//! Tests for [`super`]. Extracted from `store.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use crate::ai::Reasoning;
use crate::Specialist;

/// Serializes tests that mutate process-wide environment variables, since the
/// registry path is resolved from them and tests otherwise run in parallel.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn save_then_load_round_trips() {
    let dir = std::env::temp_dir().join(format!("sp_store_{}", std::process::id()));
    let path = dir.join("registry.json");
    let mut registry = Registry::default();
    registry.specialists.insert(
        "x".into(),
        Specialist {
            name: "x".into(),
            provider: "p".into(),
            model: "m".into(),
            system_prompt: "s".into(),
            tools: vec![],
            constraint: None,
            reasoning: Reasoning::Off,
            stream: false,
        },
    );

    save_to(&path, &registry).unwrap();
    let back = load_from(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(registry, back);
}

#[test]
fn save_to_replaces_existing_atomically() {
    // A second save overwrites the first cleanly, leaving no temp file behind.
    let dir = std::env::temp_dir().join(format!("sp_store_atomic_{}", std::process::id()));
    let path = dir.join("registry.json");

    save_to(&path, &Registry::default()).unwrap();
    let mut registry = Registry::default();
    registry.providers.insert(
        "anthropic".into(),
        crate::ProviderDef {
            name: "anthropic".into(),
            api: crate::ai::Api::AnthropicMessages,
            base_url: "https://api.anthropic.com".into(),
            api_key_env: None,
            constrained_decoding: false,
        },
    );
    save_to(&path, &registry).unwrap();

    assert_eq!(load_from(&path).unwrap(), registry);
    // The temp sibling was renamed away, not left lying around.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .collect();
    std::fs::remove_dir_all(&dir).ok();
    assert!(leftovers.is_empty());
}

#[test]
fn missing_file_loads_empty() {
    let path = std::env::temp_dir().join("sp_absent_dir_xyz/registry.json");
    assert_eq!(load_from(&path).unwrap(), Registry::default());
}

fn restore(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
}

#[test]
fn registry_path_follows_env_precedence() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = (
        std::env::var_os("SPAWNINGPOOL_REGISTRY"),
        std::env::var_os("SPAWNINGPOOL_HOME"),
        std::env::var_os("HOME"),
    );

    // An explicit registry path wins over everything else.
    std::env::set_var("SPAWNINGPOOL_REGISTRY", "/tmp/explicit.json");
    std::env::set_var("SPAWNINGPOOL_HOME", "/tmp/home");
    std::env::set_var("HOME", "/tmp/user");
    assert_eq!(registry_path(), PathBuf::from("/tmp/explicit.json"));

    // Then SPAWNINGPOOL_HOME: registry.json directly under it.
    std::env::remove_var("SPAWNINGPOOL_REGISTRY");
    assert_eq!(registry_path(), PathBuf::from("/tmp/home/registry.json"));

    // Then HOME: ~/.spawningpool/registry.json.
    std::env::remove_var("SPAWNINGPOOL_HOME");
    assert_eq!(
        registry_path(),
        PathBuf::from("/tmp/user/.spawningpool/registry.json")
    );

    // Nothing set: a relative .spawningpool/registry.json.
    std::env::remove_var("HOME");
    assert_eq!(
        registry_path(),
        PathBuf::from(".spawningpool/registry.json")
    );

    restore("SPAWNINGPOOL_REGISTRY", saved.0);
    restore("SPAWNINGPOOL_HOME", saved.1);
    restore("HOME", saved.2);
}

#[test]
fn tools_dir_sits_beside_the_registry_file() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = (
        std::env::var_os("SPAWNINGPOOL_REGISTRY"),
        std::env::var_os("SPAWNINGPOOL_HOME"),
        std::env::var_os("HOME"),
    );

    // An explicit registry path puts tools/ next to that file.
    std::env::set_var("SPAWNINGPOOL_REGISTRY", "/tmp/explicit.json");
    assert_eq!(tools_dir(), PathBuf::from("/tmp/tools"));

    // Under HOME: ~/.spawningpool/tools.
    std::env::remove_var("SPAWNINGPOOL_REGISTRY");
    std::env::remove_var("SPAWNINGPOOL_HOME");
    std::env::set_var("HOME", "/tmp/user");
    assert_eq!(tools_dir(), PathBuf::from("/tmp/user/.spawningpool/tools"));

    restore("SPAWNINGPOOL_REGISTRY", saved.0);
    restore("SPAWNINGPOOL_HOME", saved.1);
    restore("HOME", saved.2);
}

#[test]
fn workflows_dir_sits_beside_the_registry_file() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = (
        std::env::var_os("SPAWNINGPOOL_REGISTRY"),
        std::env::var_os("SPAWNINGPOOL_HOME"),
        std::env::var_os("HOME"),
    );

    // An explicit registry path puts workflows/ next to that file.
    std::env::set_var("SPAWNINGPOOL_REGISTRY", "/tmp/explicit.json");
    assert_eq!(workflows_dir(), PathBuf::from("/tmp/workflows"));

    // Under HOME: ~/.spawningpool/workflows.
    std::env::remove_var("SPAWNINGPOOL_REGISTRY");
    std::env::remove_var("SPAWNINGPOOL_HOME");
    std::env::set_var("HOME", "/tmp/user");
    assert_eq!(
        workflows_dir(),
        PathBuf::from("/tmp/user/.spawningpool/workflows")
    );

    restore("SPAWNINGPOOL_REGISTRY", saved.0);
    restore("SPAWNINGPOOL_HOME", saved.1);
    restore("HOME", saved.2);
}
