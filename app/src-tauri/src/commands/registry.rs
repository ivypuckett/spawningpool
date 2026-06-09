use serde::Serialize;
use spawningpool::{store, tools};

/// A flat, name-sorted view of everything in the registry plus the tools folder,
/// shaped for the frontend lists.
#[derive(Debug, Serialize)]
pub struct RegistrySnapshot {
    pub providers: Vec<String>,
    pub models: Vec<String>,
    pub specialists: Vec<String>,
    pub tools: Vec<String>,
    pub registry_path: String,
}

pub fn load_snapshot() -> Result<RegistrySnapshot, String> {
    let registry = store::load()?;
    let tool_names = tools::list(&store::tools_dir())?;
    Ok(RegistrySnapshot {
        providers: sorted(registry.providers.keys()),
        models: sorted(registry.models.keys()),
        specialists: sorted(registry.specialists.keys()),
        tools: sorted(tool_names.iter()),
        registry_path: store::registry_path().display().to_string(),
    })
}

fn sorted<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut v: Vec<String> = keys.cloned().collect();
    v.sort();
    v
}

#[tauri::command]
pub fn list_entities() -> Result<RegistrySnapshot, String> {
    load_snapshot()
}

/// Return one entity's full definition as JSON, or an error if it doesn't exist.
/// `kind` is one of "provider" | "model" | "specialist" | "tool". Mirrors `sp show`.
#[tauri::command]
pub fn show_entity(kind: String, name: String) -> Result<serde_json::Value, String> {
    let registry = store::load()?;
    match kind.as_str() {
        "provider" => match registry.providers.get(&name) {
            Some(def) => serde_json::to_value(def).map_err(|e| e.to_string()),
            None => Err(format!("no such {kind} {name}")),
        },
        "model" => match registry.models.get(&name) {
            Some(def) => serde_json::to_value(def).map_err(|e| e.to_string()),
            None => Err(format!("no such {kind} {name}")),
        },
        "specialist" => match registry.specialists.get(&name) {
            Some(def) => serde_json::to_value(def).map_err(|e| e.to_string()),
            None => Err(format!("no such {kind} {name}")),
        },
        "tool" => tools::resolve(&store::tools_dir(), &name)
            .and_then(|def| serde_json::to_value(def).map_err(|e| e.to_string())),
        _ => Err(format!("unknown entity kind: {kind}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_entity_returns_provider_definition() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let mut reg = spawningpool::Registry::default();
        reg.providers.insert(
            "anthropic".into(),
            spawningpool::ProviderDef {
                name: "anthropic".into(),
                api: spawningpool::ai::Api::AnthropicMessages,
                base_url: "https://api.anthropic.com".into(),
                api_key_env: None,
                constrained_decoding: false,
            },
        );
        spawningpool::store::save(&reg).unwrap();

        let result = show_entity("provider".into(), "anthropic".into()).unwrap();
        assert_eq!(result["name"], "anthropic");
    }

    #[test]
    fn show_entity_returns_error_for_missing_provider() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let err = show_entity("provider".into(), "ghost".into()).unwrap_err();
        assert!(err.contains("no such provider ghost"), "got: {err}");
    }

    #[test]
    fn show_entity_returns_error_for_unknown_kind() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let err = show_entity("bogus".into(), "x".into()).unwrap_err();
        assert!(err.contains("unknown entity kind"), "got: {err}");
    }

    #[test]
    fn show_entity_returns_tool_definition() {
        use std::os::unix::fs::PermissionsExt;

        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let tools_dir = spawningpool::store::tools_dir();
        std::fs::create_dir_all(&tools_dir).unwrap();
        let script_path = tools_dir.join("ping");
        std::fs::write(
            &script_path,
            "#!/bin/sh\n# desc: Ping\n# params: HOST\necho hi\n",
        )
        .unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let result = show_entity("tool".into(), "ping".into()).unwrap();
        assert_eq!(result["description"], "Ping");
        let params = result["params"].as_array().unwrap();
        assert!(
            params.iter().any(|p| p.as_str() == Some("HOST")),
            "params should contain HOST, got: {params:?}"
        );
    }

    #[test]
    fn show_entity_returns_model_definition() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let mut reg = spawningpool::Registry::default();
        reg.models.insert(
            "claude-3-5-sonnet".into(),
            spawningpool::ModelDef {
                id: "claude-3-5-sonnet".into(),
                name: "Claude 3.5 Sonnet".into(),
                provider: "anthropic".into(),
                max_tokens: 8192,
                context_window: 200_000,
            },
        );
        spawningpool::store::save(&reg).unwrap();

        let result = show_entity("model".into(), "claude-3-5-sonnet".into()).unwrap();
        assert_eq!(result["name"], "Claude 3.5 Sonnet");
    }

    #[test]
    fn show_entity_returns_error_for_missing_model() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let err = show_entity("model".into(), "ghost".into()).unwrap_err();
        assert!(err.contains("no such model ghost"), "got: {err}");
    }

    #[test]
    fn show_entity_returns_specialist_definition() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let mut reg = spawningpool::Registry::default();
        reg.specialists.insert(
            "summarizer".into(),
            spawningpool::Specialist {
                name: "summarizer".into(),
                provider: "anthropic".into(),
                model: "claude-3-5-sonnet".into(),
                system_prompt: "Summarize the input.".into(),
                tools: vec![],
                constraint: None,
                reasoning: spawningpool::ai::Reasoning::Off,
                stream: false,
            },
        );
        spawningpool::store::save(&reg).unwrap();

        let result = show_entity("specialist".into(), "summarizer".into()).unwrap();
        assert_eq!(result["name"], "summarizer");
    }

    #[test]
    fn show_entity_returns_error_for_missing_specialist() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let err = show_entity("specialist".into(), "ghost".into()).unwrap_err();
        assert!(err.contains("no such specialist ghost"), "got: {err}");
    }

    #[test]
    fn list_entities_reads_an_empty_registry() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();
        let snapshot = load_snapshot().unwrap();
        assert!(snapshot.providers.is_empty());
        assert!(snapshot.models.is_empty());
        assert!(snapshot.specialists.is_empty());
        assert!(snapshot.tools.is_empty());
    }

    #[test]
    fn list_entities_reads_a_populated_registry() {
        let _g = crate::test_support::env_lock();
        let _tmp = crate::test_support::point_registry_at_temp();

        let mut reg = spawningpool::Registry::default();
        reg.providers.insert(
            "anthropic".into(),
            spawningpool::ProviderDef {
                name: "anthropic".into(),
                api: spawningpool::ai::Api::AnthropicMessages,
                base_url: "https://api.anthropic.com".into(),
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
                constrained_decoding: false,
            },
        );
        spawningpool::store::save(&reg).unwrap();

        let snapshot = load_snapshot().unwrap();
        assert_eq!(snapshot.providers, vec!["anthropic".to_string()]);
        assert!(snapshot.models.is_empty());
        assert!(snapshot.specialists.is_empty());
        assert!(snapshot.tools.is_empty());
    }
}
