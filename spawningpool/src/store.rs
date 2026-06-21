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
//! dropped. To close that window, [`load_versioned`] captures the file's
//! modified-time and size as a token and [`save_checked`] refuses to overwrite
//! when the file changed since — turning a silent lost update into a visible
//! "reload and retry". The plain [`load`]/[`save`] pair keeps the unchecked
//! behavior for read-only and short-lived read-modify-write callers.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// The directory holding run logs: a `logs/` folder alongside the registry file
/// (so `~/.spawningpool/logs/` by default). Each invocation writes one NDJSON
/// file here (docs/workflow-logging.md), rather than into the current working
/// directory, so logs collect in one place regardless of where the CLI is run.
pub fn logs_dir() -> PathBuf {
    match registry_path().parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("logs"),
        _ => PathBuf::from("logs"),
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

/// A token capturing the registry file's on-disk state — its modified time and
/// size — at the moment it was loaded. Compared at save time by [`save_checked`]
/// to detect a concurrent writer. `None` means the file did not exist when
/// loaded, so the first write creates it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Version(Option<(SystemTime, u64)>);

/// The current version of the file at `path`: its modified time and size, or an
/// absent token if the file is missing (or its mtime is unreadable, treated as
/// absent so any real version compares unequal and forces a reload).
fn version_of(path: &Path) -> Version {
    match std::fs::metadata(path) {
        Ok(meta) => Version(meta.modified().ok().map(|t| (t, meta.len()))),
        Err(_) => Version(None),
    }
}

/// Load the registry along with a [`Version`] token of the file it came from, so
/// a later [`save_checked`] can refuse to overwrite a concurrent change.
pub fn load_versioned() -> Result<(Registry, Version), String> {
    load_versioned_from(&registry_path())
}

/// Load the registry and its [`Version`] from an explicit path.
pub fn load_versioned_from(path: &Path) -> Result<(Registry, Version), String> {
    // Stat *before* reading so the token can only be older than the bytes we
    // read, never newer: a write racing this load yields a stale token (a
    // harmless false "changed on disk"), never a fresh one that would let a lost
    // update slip through the check.
    let version = version_of(path);
    let registry = load_from(path)?;
    Ok((registry, version))
}

/// Save the registry to its resolved path only if the file still matches
/// `expected` — the [`Version`] captured by [`load_versioned`].
pub fn save_checked(registry: &Registry, expected: Version) -> Result<Version, String> {
    save_to_checked(&registry_path(), registry, expected)
}

/// Save the registry to an explicit path only if the file still matches
/// `expected`. If another writer changed the file since it was loaded, the write
/// is refused with an actionable error and the on-disk registry is left
/// untouched, so the caller can reload rather than silently clobber that change.
/// On success returns the new [`Version`] of the written file.
///
/// The check is optimistic: a small window remains between the stat and the
/// rename, which a local single-user tool can accept. See the [module-level
/// concurrency note](self#concurrency).
pub fn save_to_checked(
    path: &Path,
    registry: &Registry,
    expected: Version,
) -> Result<Version, String> {
    if version_of(path) != expected {
        return Err(format!(
            "{} changed on disk since it was loaded — reload and retry",
            path.display()
        ));
    }
    save_to(path, registry)?;
    Ok(version_of(path))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
