//! The tool catalog as an `ls`-able folder of executable scripts.
//!
//! A tool is one executable script (or a symlink to one) living in the
//! [`crate::store::tools_dir`] folder; the tool's name is the script's file name
//! with any extension stripped, so `ping.sh` and a symlink named `ping` both
//! back the `ping` tool. Nothing is recorded in `registry.json` — the folder is
//! the source of truth, so a tool's description and parameters are read from its
//! `# desc:`/`# params:` header (see [`crate::summarize`]) every time it's
//! resolved rather than snapshotted at define time.

use std::path::{Path, PathBuf};

use crate::domain::ToolDef;
use crate::script;

/// Resolve a tool by name: find the script backing it and read its header into a
/// [`ToolDef`]. The returned `script` is canonical, so it runs regardless of the
/// directory `spawningpool run` is invoked from.
pub fn resolve(dir: &Path, name: &str) -> Result<ToolDef, String> {
    let path = find(dir, name)?;
    // Canonicalize so a symlink resolves to the real executable; a broken
    // symlink (dangling target) surfaces here as an unreadable tool.
    let script = std::fs::canonicalize(&path)
        .map_err(|e| format!("tool '{name}' script {} can't be read: {e}", path.display()))?;
    let summary = script::summarize(&script).map_err(|e| {
        format!(
            "tool '{name}' script {} can't be read: {e}",
            script.display()
        )
    })?;
    Ok(ToolDef {
        name: name.to_string(),
        script,
        description: summary.desc.unwrap_or_default(),
        params: summary.params,
        output: summary.output,
    })
}

/// Resolve several tools by name, in order. The error is the first name that
/// can't be resolved, matching the per-name message from [`resolve`].
pub fn resolve_all(dir: &Path, names: &[String]) -> Result<Vec<ToolDef>, String> {
    names.iter().map(|name| resolve(dir, name)).collect()
}

/// Whether a tool named `name` has a backing script in `dir`.
pub fn exists(dir: &Path, name: &str) -> bool {
    entries_with_stem(dir, name).is_ok_and(|m| !m.is_empty())
}

/// The names of every tool in `dir`, sorted and de-duplicated. A missing folder
/// lists as empty. Entries whose name isn't a valid tool name (dotfiles, odd
/// characters) are skipped rather than reported.
pub fn list(dir: &Path) -> Result<Vec<String>, String> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("can't read tools dir {}: {e}", dir.display())),
    };
    let mut names = std::collections::BTreeSet::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if is_valid_tool_name(stem) {
                names.insert(stem.to_string());
            }
        }
    }
    Ok(names.into_iter().collect())
}

/// Remove every script backing `name` from `dir`, returning whether any was
/// removed. Clears the whole name (e.g. both `ping` and a stray `ping.sh`) so a
/// redefine or delete can't leave an ambiguous pair behind.
pub fn remove(dir: &Path, name: &str) -> Result<bool, String> {
    let mut removed = false;
    for path in entries_with_stem(dir, name)? {
        std::fs::remove_file(&path)
            .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
        removed = true;
    }
    Ok(removed)
}

/// Whether `name` is usable as a tool name — what providers accept for a tool
/// (ASCII alphanumerics, `_`, `-`) and what keeps the name<->file-stem mapping
/// unambiguous. Used to gate `spawningpool define tool` and to skip junk in [`list`].
pub fn is_valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The unique script backing `name`, or an error naming the problem.
fn find(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let mut matches = entries_with_stem(dir, name)?;
    match matches.len() {
        0 => Err(format!("unknown tool: {name}")),
        1 => Ok(matches.pop().expect("len checked")),
        n => Err(format!(
            "tool '{name}' is ambiguous: {n} files in {} share that name; keep one",
            dir.display()
        )),
    }
}

/// Every entry in `dir` whose file name, minus a single extension, equals `name`.
/// A missing folder yields none. Directories are ignored; symlinks to files are
/// kept (their target is followed when the tool is resolved or run).
fn entries_with_stem(dir: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    crate::store::entries_with_stem(dir, name)
        .map_err(|e| format!("can't read tools dir {}: {e}", dir.display()))
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
