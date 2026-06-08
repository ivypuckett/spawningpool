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

#[cfg(test)]
mod tests {
    use super::*;

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
