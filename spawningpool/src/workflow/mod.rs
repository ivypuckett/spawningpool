//! Workflow DSL: parse, type-check, and evaluate orchestration scripts that
//! chain tools and specialists with typed data flow.
//!
//! See `docs/workflow-dsl.md` for the full language specification (v1).
//!
//! ## Usage
//!
//! ```rust,no_run
//! use spawningpool::workflow::{parse, check, eval};
//! use spawningpool::ai::{Client, CompleteOptions};
//! use spawningpool::store;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let source = r#"
//! city = "Portland"
//!
//! weather = call get_weather { CITY: city }
//!
//! result = { "city": city, "ok": weather.reachable }
//! "#;
//!
//! let workflow = parse(source)?;
//!
//! let registry = store::load()?;
//! let tools_dir = store::tools_dir();
//! let tools = spawningpool::tools::resolve_all(&tools_dir, &["get_weather".to_string()])?;
//!
//! // Optional: type-check before running.
//! check(&workflow, &registry, &tools)?;
//!
//! let client = Client::new();
//! let opts = CompleteOptions::default();
//! let result = eval(&workflow, &registry, &tools, &client, &opts).await?;
//! println!("{result}");
//! # Ok(())
//! # }
//! ```

pub mod ast;
pub mod check;
pub mod eval;
pub mod parse;

use std::path::Path;

pub use ast::{AccessKey, BinOp, Expr, Statement, Workflow};
pub use check::{check, specialist_return_type, TypeEnv, TypeError};
pub use eval::{eval, WorkflowError};
pub use parse::{parse, ParseError};

/// Read the DSL source of the workflow named `name` from `dir`.
///
/// Workflows live as source files in a `workflows/` folder (see
/// [`crate::store::workflows_dir`]); the name is the file name with any single
/// extension stripped, so `deploy.spool` and a file named `deploy` both back the
/// `deploy` workflow. Mirrors [`crate::tools::resolve`]'s stem-matching.
pub fn source(dir: &Path, name: &str) -> Result<String, String> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("unknown workflow: {name}"))
        }
        Err(e) => return Err(format!("can't read workflows dir {}: {e}", dir.display())),
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
    match matches.len() {
        0 => Err(format!("unknown workflow: {name}")),
        1 => {
            let path = &matches[0];
            std::fs::read_to_string(path)
                .map_err(|e| format!("workflow '{name}' at {} can't be read: {e}", path.display()))
        }
        n => Err(format!(
            "workflow '{name}' is ambiguous: {n} files in {} share that name; keep one",
            dir.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::source;
    use std::path::{Path, PathBuf};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sp_workflows_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn source_reads_file_and_strips_extension() {
        let dir = temp_dir("read");
        std::fs::write(dir.join("deploy.spool"), "x = 1").unwrap();
        assert_eq!(source(&dir, "deploy").unwrap(), "x = 1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn source_errors_on_unknown_name() {
        let dir = temp_dir("unknown");
        let err = source(&dir, "absent").unwrap_err();
        assert!(err.contains("unknown workflow: absent"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn source_errors_on_missing_dir() {
        let dir = Path::new("/tmp/sp_workflows_definitely_absent_xyz");
        let err = source(dir, "any").unwrap_err();
        assert!(err.contains("unknown workflow: any"), "{err}");
    }

    #[test]
    fn source_reports_ambiguity() {
        let dir = temp_dir("ambig");
        std::fs::write(dir.join("dup.spool"), "x = 1").unwrap();
        std::fs::write(dir.join("dup.wf"), "x = 2").unwrap();
        let err = source(&dir, "dup").unwrap_err();
        assert!(err.contains("ambiguous"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
