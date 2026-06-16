//! Tests for [`super`]. Extracted from `main.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

/// Serializes the tests below that point `$SPAWNINGPOOL_REGISTRY` at a temp
/// file, since that env var is process-wide and tests otherwise run parallel.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn parse_list_splits_and_trims() {
    assert_eq!(
        parse_list(Some("a, b ,c".into())),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
    assert!(parse_list(None).is_empty());
    assert!(parse_list(Some("  ,  ".into())).is_empty());
}

#[test]
fn affirmative_only_accepts_an_explicit_yes() {
    for yes in ["y", "Y", "yes", "Yes", " yes \n"] {
        assert!(affirmative(yes), "{yes:?} should confirm");
    }
    // Empty (the EOF case on a non-interactive stdin) and anything else decline.
    for no in ["", "\n", "n", "no", "yep", "1"] {
        assert!(!affirmative(no), "{no:?} should decline");
    }
}

#[test]
fn parse_reasoning_maps_levels_and_rejects_unknown() {
    assert_eq!(parse_reasoning("high"), Ok(Reasoning::High));
    assert_eq!(parse_reasoning("off"), Ok(Reasoning::Off));
    assert!(parse_reasoning("ultra").is_err());
}

fn write_script(body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!(
        "sp_cli_tool_{}_{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn restore_registry_env(saved: Option<std::ffi::OsString>) {
    match saved {
        Some(v) => std::env::set_var("SPAWNINGPOOL_REGISTRY", v),
        None => std::env::remove_var("SPAWNINGPOOL_REGISTRY"),
    }
}

#[test]
fn define_list_show_and_delete_round_trip_through_the_store() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = std::env::var_os("SPAWNINGPOOL_REGISTRY");
    let dir = std::env::temp_dir().join(format!("sp_cli_define_{}", std::process::id()));
    let path = dir.join("registry.json");
    std::env::set_var("SPAWNINGPOOL_REGISTRY", &path);

    define(DefineEntity::Provider {
        name: "anthropic".into(),
        api: "anthropic-messages".into(),
        base_url: "https://api.anthropic.com".into(),
        api_key_env: Some("ANTHROPIC_API_KEY".into()),
        constrained_decoding: false,
    })
    .unwrap();

    // The provider is persisted and reloads from disk.
    assert!(spawningpool::store::load()
        .unwrap()
        .providers
        .contains_key("anthropic"));
    // Listing succeeds against the populated registry. Driven on a local
    // runtime rather than `#[tokio::test]` so the env-serializing guard is
    // never held across an await point.
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(list(ListKind::Providers))
        .unwrap();
    // Showing a defined entity succeeds; an absent one errors.
    show(ShowEntity::Provider {
        name: "anthropic".into(),
    })
    .unwrap();
    let err = show(ShowEntity::Provider {
        name: "ghost".into(),
    })
    .unwrap_err();
    assert!(err.contains("no such"));

    // Deleting it removes it. `yes` skips the interactive confirmation.
    delete(
        DeleteEntity::Provider {
            name: "anthropic".into(),
        },
        true,
    )
    .unwrap();
    assert!(!spawningpool::store::load()
        .unwrap()
        .providers
        .contains_key("anthropic"));

    // Deleting something absent is an error.
    let err = delete(
        DeleteEntity::Provider {
            name: "ghost".into(),
        },
        true,
    )
    .unwrap_err();
    assert!(err.contains("no such"));

    std::fs::remove_dir_all(&dir).ok();
    restore_registry_env(saved);
}

#[test]
fn define_specialist_rejects_tools_and_constraint_together() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = std::env::var_os("SPAWNINGPOOL_REGISTRY");
    let dir = std::env::temp_dir().join(format!("sp_cli_val_{}", std::process::id()));
    let path = dir.join("registry.json");
    std::env::set_var("SPAWNINGPOOL_REGISTRY", &path);

    let err = define(DefineEntity::Specialist {
        name: "bad".into(),
        provider: "p".into(),
        model: "m".into(),
        system_prompt: "s".into(),
        tools: Some("a,b".into()),
        constraint: Some("a".into()),
        reasoning: "off".into(),
        stream: false,
    })
    .unwrap_err();
    assert!(err.contains("tools and a constraint"));

    std::fs::remove_dir_all(&dir).ok();
    restore_registry_env(saved);
}

fn populated_registry() -> Registry {
    let mut registry = Registry::default();
    registry.providers.insert(
        "anthropic".into(),
        ProviderDef {
            name: "anthropic".into(),
            api: spawningpool::ai::Api::AnthropicMessages,
            base_url: "https://api.anthropic.com".into(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
            constrained_decoding: false,
        },
    );
    registry.models.insert(
        "claude".into(),
        ModelDef {
            id: "claude".into(),
            name: "Claude".into(),
            provider: "anthropic".into(),
            max_tokens: 1024,
            context_window: 200_000,
        },
    );
    registry
}

/// A temp tools folder containing a single executable `ping` script, plus the
/// folder path. The caller removes the folder when done.
fn tools_dir_with_ping() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!(
        "sp_cli_tools_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("ping");
    std::fs::write(&script, "#!/bin/sh\n# desc: Ping\necho hi\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn specialist_ref(
    provider: &str,
    model: &str,
    tools: Vec<String>,
    constraint: Option<String>,
) -> Specialist {
    Specialist {
        name: "spec".into(),
        provider: provider.into(),
        model: model.into(),
        system_prompt: "s".into(),
        tools,
        constraint,
        reasoning: Reasoning::Off,
        stream: false,
    }
}

#[test]
fn available_names_lists_sorted_or_notes_emptiness() {
    let mut map: HashMap<String, u8> = HashMap::new();
    assert_eq!(available_names(&map), "(none defined yet)");
    map.insert("b".into(), 0);
    map.insert("a".into(), 0);
    assert_eq!(available_names(&map), "a, b");
}

#[test]
fn check_specialist_refs_passes_when_all_present() {
    let registry = populated_registry();
    let dir = tools_dir_with_ping();
    let spec = specialist_ref("anthropic", "claude", vec!["ping".into()], None);
    let result = check_specialist_refs(&registry, &spec, &dir);
    std::fs::remove_dir_all(&dir).ok();
    assert!(result.is_ok());
}

#[test]
fn check_specialist_refs_reports_missing_provider_model_and_tool() {
    let registry = populated_registry();
    let dir = tools_dir_with_ping();

    let err = check_specialist_refs(
        &registry,
        &specialist_ref("ghost", "claude", vec![], None),
        &dir,
    )
    .unwrap_err();
    assert!(err.contains("references provider 'ghost'"));
    assert!(err.contains("spawningpool define provider ghost"));

    let err = check_specialist_refs(
        &registry,
        &specialist_ref("anthropic", "nope", vec![], None),
        &dir,
    )
    .unwrap_err();
    assert!(err.contains("references model 'nope'"));
    assert!(err.contains("spawningpool define model nope"));

    let err = check_specialist_refs(
        &registry,
        &specialist_ref("anthropic", "claude", vec!["absent".into()], None),
        &dir,
    )
    .unwrap_err();
    std::fs::remove_dir_all(&dir).ok();
    assert!(err.contains("references tool 'absent'"));
    assert!(err.contains("spawningpool define tool absent"));
}

#[test]
fn check_specialist_refs_validates_the_constrained_tool() {
    let registry = populated_registry();
    let dir = tools_dir_with_ping();
    // A constraint names a tool too, so an undefined forced tool is caught.
    let spec = specialist_ref("anthropic", "claude", vec![], Some("absent".into()));
    let err = check_specialist_refs(&registry, &spec, &dir).unwrap_err();
    std::fs::remove_dir_all(&dir).ok();
    assert!(err.contains("references tool 'absent'"));
}

#[test]
fn check_model_refs_requires_a_defined_provider() {
    let registry = populated_registry();
    let ok = ModelDef {
        id: "m".into(),
        name: "m".into(),
        provider: "anthropic".into(),
        max_tokens: 1,
        context_window: 1,
    };
    assert!(check_model_refs(&registry, &ok).is_ok());

    let bad = ModelDef {
        provider: "ghost".into(),
        ..ok
    };
    let err = check_model_refs(&registry, &bad).unwrap_err();
    assert!(err.contains("references provider 'ghost'"));
    assert!(err.contains("spawningpool define provider ghost"));
}

#[test]
fn referrers_find_entities_pointing_at_a_target() {
    let mut registry = populated_registry();
    registry.specialists.insert(
        "spec".into(),
        specialist_ref("anthropic", "claude", vec!["ping".into()], None),
    );

    // A provider is referenced by both the specialist and the model under it.
    assert_eq!(
        referrers_of_provider(&registry, "anthropic"),
        vec![
            "specialist 'spec'".to_string(),
            "model 'claude'".to_string()
        ]
    );
    assert_eq!(
        referrers_of_model(&registry, "claude"),
        vec!["specialist 'spec'".to_string()]
    );
    assert_eq!(
        referrers_of_tool(&registry, "ping"),
        vec!["specialist 'spec'".to_string()]
    );

    // An unreferenced name has no referrers.
    assert!(referrers_of_provider(&registry, "openai").is_empty());
}

#[test]
fn referrers_of_tool_includes_a_constrained_tool() {
    let mut registry = populated_registry();
    registry.specialists.insert(
        "spec".into(),
        specialist_ref("anthropic", "claude", vec![], Some("ping".into())),
    );
    assert_eq!(
        referrers_of_tool(&registry, "ping"),
        vec!["specialist 'spec'".to_string()]
    );
}

#[test]
fn onboarding_message_walks_the_progression() {
    // Empty registry: step 1, both provider examples.
    let empty = Registry::default();
    let msg = onboarding_message(&empty);
    assert!(msg.contains("[1/4]"));
    assert!(msg.contains("spawningpool define provider anthropic"));
    assert!(msg.contains("spawningpool define provider lmstudio"));

    // Provider only: step 2, points at the real provider.
    let mut reg = Registry::default();
    reg.providers.insert(
        "anthropic".into(),
        ProviderDef {
            name: "anthropic".into(),
            api: Api::AnthropicMessages,
            base_url: "https://api.anthropic.com".into(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
            constrained_decoding: false,
        },
    );
    let msg = onboarding_message(&reg);
    assert!(msg.contains("[2/4]"));
    assert!(msg.contains("spawningpool define model"));
    // Anthropic has no discovery endpoint, so don't offer --remote.
    assert!(!msg.contains("--remote"));

    // Model present: step 3, example uses the real provider/model.
    reg.models.insert(
        "claude".into(),
        ModelDef {
            id: "claude".into(),
            name: "Claude".into(),
            provider: "anthropic".into(),
            max_tokens: 1024,
            context_window: 200_000,
        },
    );
    let msg = onboarding_message(&reg);
    assert!(msg.contains("[3/4]"));
    assert!(msg.contains("--provider anthropic --model claude"));

    // Specialist present: step 4, run command names the real specialist.
    reg.specialists.insert(
        "summarizer".into(),
        specialist_ref("anthropic", "claude", vec![], None),
    );
    let msg = onboarding_message(&reg);
    assert!(msg.contains("[4/4]"));
    assert!(msg.contains("spawningpool run specialist spec"));
}

#[test]
fn no_models_state_offers_discovery_for_openai_providers() {
    let mut reg = Registry::default();
    reg.providers.insert(
        "lmstudio".into(),
        ProviderDef {
            name: "lmstudio".into(),
            api: Api::OpenAiCompletions,
            base_url: "http://localhost:1234/v1".into(),
            api_key_env: None,
            constrained_decoding: false,
        },
    );
    let msg = onboarding_message(&reg);
    assert!(msg.contains("spawningpool list models --remote"));
}

#[test]
fn unset_key_warnings_flags_only_missing_env_vars() {
    let reg = populated_registry();
    // The anthropic provider wants ANTHROPIC_API_KEY.
    let warnings = unset_key_warnings(&reg, |_| false);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("ANTHROPIC_API_KEY"));
    // When it's set, nothing to warn about.
    assert!(unset_key_warnings(&reg, |_| true).is_empty());
}

#[test]
fn progress_checks_completed_rungs() {
    assert!(progress(0).starts_with("  [1/4]"));
    assert!(progress(3).contains("specialist \u{2713}"));
    assert!(progress(3).starts_with("  [4/4]"));
}

#[test]
fn resolve_script_returns_absolute_path_for_executable() {
    let script = write_script("#!/bin/sh\necho hi\n");
    let resolved = resolve_script(&script).unwrap();
    std::fs::remove_file(&script).ok();
    assert!(resolved.is_absolute());
}

#[test]
fn resolve_script_rejects_non_executable_with_chmod_hint() {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!(
        "sp_noexec_{}_{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let err = resolve_script(&path).unwrap_err();
    std::fs::remove_file(&path).ok();
    assert!(err.contains("isn't executable"));
    assert!(err.contains("chmod +x"));
}
