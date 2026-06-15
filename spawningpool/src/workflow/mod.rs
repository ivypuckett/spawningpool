//! Workflow DSL: parse, type-check, and evaluate orchestration scripts that
//! chain tools and specialists with typed data flow.
//!
//! See `docs/workflow-dsl.md` for the full language specification (v1).
//!
//! ## Usage
//!
//! ```rust,no_run
//! use std::collections::HashMap;
//! use spawningpool::workflow::{parse, check, eval};
//! use spawningpool::ai::Client;
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
//! // Map each provider to its API key (here: none needed for a tool-only workflow).
//! let keys: HashMap<String, String> = HashMap::new();
//! let result = eval(&workflow, &registry, &tools, &client, &keys).await?;
//! println!("{result}");
//! # Ok(())
//! # }
//! ```

pub mod ast;
pub mod check;
pub mod eval;
pub mod parse;

use std::collections::BTreeSet;
use std::path::Path;

use crate::domain::Registry;

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
    let matches = crate::store::entries_with_stem(dir, name)
        .map_err(|e| format!("can't read workflows dir {}: {e}", dir.display()))?;
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

/// The tools and specialists a workflow references. Resolve exactly these tools
/// (not the whole catalog) before evaluating, so an unrelated broken tool can't
/// block a workflow that doesn't use it; the specialists are used to pre-flight
/// API keys.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Referenced {
    /// Tool names to resolve: every `call` tool, plus the tools each invoked
    /// specialist needs (looked up in the registry).
    pub tools: BTreeSet<String>,
    /// Specialist names invoked via `ask`.
    pub specialists: BTreeSet<String>,
}

/// Walk a workflow to collect every tool and specialist it references.
pub fn referenced(workflow: &Workflow, registry: &Registry) -> Referenced {
    let mut refs = Referenced::default();
    for stmt in &workflow.statements {
        collect(&stmt.expr, registry, &mut refs);
    }
    refs
}

fn collect(expr: &Expr, registry: &Registry, refs: &mut Referenced) {
    match expr {
        Expr::Str(_) | Expr::Num(_) | Expr::Bool(_) | Expr::Var(_) => {}
        Expr::Object(fields) => {
            for (_, e) in fields {
                collect(e, registry, refs);
            }
        }
        Expr::Not(inner) => collect(inner, registry, refs),
        Expr::BinOp { lhs, rhs, .. } => {
            collect(lhs, registry, refs);
            collect(rhs, registry, refs);
        }
        Expr::Access { base, keys } => {
            collect(base, registry, refs);
            for key in keys {
                if let AccessKey::Computed(e) = key {
                    collect(e, registry, refs);
                }
            }
        }
        Expr::If { branches, default } => {
            for (cond, result) in branches {
                collect(cond, registry, refs);
                collect(result, registry, refs);
            }
            collect(default, registry, refs);
        }
        Expr::For { array, body, .. } => {
            collect(array, registry, refs);
            collect(body, registry, refs);
        }
        Expr::Call { tool, args } => {
            refs.tools.insert(tool.clone());
            for (_, e) in args {
                collect(e, registry, refs);
            }
        }
        Expr::Ask { specialist, prompt } => {
            refs.specialists.insert(specialist.clone());
            if let Some(spec) = registry.specialists.get(specialist) {
                for tool in spec.tool_names() {
                    refs.tools.insert(tool.clone());
                }
            }
            collect(prompt, registry, refs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{referenced, source};
    use crate::domain::{Registry, Specialist};
    use crate::workflow::parse;
    use std::path::PathBuf;

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
        let dir = temp_dir("missing");
        let err = source(&dir.join("nope"), "any").unwrap_err();
        assert!(err.contains("unknown workflow: any"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
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

    #[test]
    fn referenced_collects_call_tools_and_specialist_tools() {
        let mut registry = Registry::default();
        registry.specialists.insert(
            "writer".to_string(),
            Specialist {
                name: "writer".to_string(),
                provider: "anthropic".to_string(),
                model: "m".to_string(),
                system_prompt: String::new(),
                tools: vec!["search".to_string()],
                constraint: None,
                reasoning: crate::ai::Reasoning::Off,
                stream: false,
            },
        );
        let wf = parse("a = call fetch {}\n\nb = ask writer \"hi\"").unwrap();
        let refs = referenced(&wf, &registry);
        // Direct `call` tool plus the specialist's own tool, and the specialist.
        assert_eq!(
            refs.tools,
            ["fetch".to_string(), "search".to_string()]
                .into_iter()
                .collect()
        );
        assert_eq!(
            refs.specialists,
            ["writer".to_string()].into_iter().collect()
        );
    }
}
