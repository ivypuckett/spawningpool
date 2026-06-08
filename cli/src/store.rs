//! Persistence for the [`Registry`]: a single JSON file on disk.
//!
//! The location is `$SPAWNINGPOOL_HOME/registry.json` (default
//! `~/.spawningpool/registry.json`), or the exact path in `$SPAWNINGPOOL_REGISTRY`
//! when set. A missing file loads as an empty registry, so the first `define`
//! creates it.

use std::path::{Path, PathBuf};

use spawningpool::Registry;

/// Serializes tests that mutate process-wide environment variables, since the
/// registry path is resolved from them and tests otherwise run in parallel.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The resolved path to the registry file.
pub fn registry_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SPAWNINGPOOL_REGISTRY") {
        return PathBuf::from(path);
    }
    let dir = match std::env::var_os("SPAWNINGPOOL_HOME") {
        Some(home) => PathBuf::from(home),
        None => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(".spawningpool"),
            None => PathBuf::from(".spawningpool"),
        },
    };
    dir.join("registry.json")
}

pub fn load() -> Result<Registry, String> {
    load_from(&registry_path())
}

pub fn save(registry: &Registry) -> Result<(), String> {
    save_to(&registry_path(), registry)
}

fn load_from(path: &Path) -> Result<Registry, String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|e| format!("failed to parse {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(format!("failed to read {}: {e}", path.display())),
    }
}

fn save_to(path: &Path, registry: &Registry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(registry).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spawningpool::ai::Reasoning;
    use spawningpool::Specialist;

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
}
