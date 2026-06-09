/// Serializes tests that set `$SPAWNINGPOOL_REGISTRY`, since the env var is
/// process-wide and tests otherwise run in parallel.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A guard that, on construction, records the previous `$SPAWNINGPOOL_REGISTRY`
/// value, creates a unique temp dir, and points `$SPAWNINGPOOL_REGISTRY` at a
/// `registry.json` inside it. On `Drop`, restores the previous env value and
/// removes the temp dir.
pub(crate) struct TempRegistry {
    dir: std::path::PathBuf,
    saved: Option<std::ffi::OsString>,
}

pub(crate) fn point_registry_at_temp() -> TempRegistry {
    let saved = std::env::var_os("SPAWNINGPOOL_REGISTRY");
    let dir = std::env::temp_dir().join(format!(
        "sp_app_reg_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("registry.json");
    std::env::set_var("SPAWNINGPOOL_REGISTRY", &path);
    TempRegistry { dir, saved }
}

impl Drop for TempRegistry {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(v) => std::env::set_var("SPAWNINGPOOL_REGISTRY", v),
            None => std::env::remove_var("SPAWNINGPOOL_REGISTRY"),
        }
        std::fs::remove_dir_all(&self.dir).ok();
    }
}
