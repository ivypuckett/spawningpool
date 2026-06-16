//! Persistence for the [`Registry`]: a single JSON file on disk.
//!
//! The location is `$SPAWNINGPOOL_HOME/registry.json` (default
//! `~/.spawningpool/registry.json`), or the exact path in `$SPAWNINGPOOL_REGISTRY`
//! when set. A missing file loads as an empty registry, so the first write
//! creates it. Both `spawningpool` and any other front-end share this module so they read
//! and write the registry through one path-resolution policy.
//!
//! # Concurrency
//!
//! Each [`save_to`] is atomic (temp file + rename), so a crash or a second
//! writer can never leave a half-written, unparseable file. It does **not**,
//! however, make a read-modify-write *transactional*: this layer assumes a
//! single writer at a time. If two processes each [`load`], mutate, and save
//! concurrently, the later rename wins and the earlier process's change is
//! silently lost (e.g. a long-lived TUI session saving over a `define` run
//! meanwhile in another terminal). The data is never corrupted, only an update
//! dropped. Closing that window — an mtime check that refuses a stale
//! overwrite — is tracked as future work, not handled here.

use std::path::{Path, PathBuf};

use crate::Registry;

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

/// The directory holding tool scripts: a `tools/` folder alongside the registry
/// file. Tools aren't stored in `registry.json`; each is just an executable
/// script (or a symlink to one) in this folder, named after the tool.
pub fn tools_dir() -> PathBuf {
    match registry_path().parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("tools"),
        _ => PathBuf::from("tools"),
    }
}

/// The directory holding workflow scripts: a `workflows/` folder alongside the
/// registry file. Like tools, workflows aren't stored in `registry.json`; each
/// is just a DSL source file in this folder, named after the workflow.
pub fn workflows_dir() -> PathBuf {
    match registry_path().parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("workflows"),
        _ => PathBuf::from("workflows"),
    }
}

/// Every non-directory entry in `dir` whose file name, minus a single extension,
/// equals `name`. A missing directory yields an empty list. Shared by the tool
/// and workflow folders, which both name their files by stem (so `deploy.spool`
/// and a file named `deploy` both match `deploy`).
pub fn entries_with_stem(dir: &Path, name: &str) -> std::io::Result<Vec<PathBuf>> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut matches = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some(name) {
            matches.push(path);
        }
    }
    Ok(matches)
}

/// Load the registry from its resolved path. A missing file is an empty registry.
pub fn load() -> Result<Registry, String> {
    load_from(&registry_path())
}

/// Save the registry to its resolved path.
pub fn save(registry: &Registry) -> Result<(), String> {
    save_to(&registry_path(), registry)
}

/// Load the registry from an explicit path. A missing file is an empty registry.
pub fn load_from(path: &Path) -> Result<Registry, String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|e| format!("failed to parse {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(format!("failed to read {}: {e}", path.display())),
    }
}

/// Save the registry to an explicit path, creating parent directories as needed.
///
/// The write is atomic: the JSON goes to a sibling temp file that is then renamed
/// over the target, so a crash mid-write (or a concurrent writer) can't leave a
/// half-written, unparseable registry behind. Atomicity is per-write only; see
/// the [module-level concurrency note](self#concurrency) for the single-writer
/// assumption this does not cover.
pub fn save_to(path: &Path, registry: &Registry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(registry).map_err(|e| e.to_string())?;

    let file_name = path
        .file_name()
        .ok_or_else(|| format!("invalid registry path {}", path.display()))?;
    // A per-process temp name in the target's directory keeps the rename on the
    // same filesystem (so it's atomic) without two processes clobbering one temp.
    let tmp = path.with_file_name(format!(
        ".{}.tmp.{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&tmp, json).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("failed to write {}: {e}", path.display())
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
