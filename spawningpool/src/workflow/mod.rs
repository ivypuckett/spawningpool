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
//! weather = run tool get_weather { CITY: city }
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
//! // No nested workflows here; a `run` would resolve against this map.
//! let workflows = HashMap::new();
//! check(&workflow, &registry, &tools, &workflows)?;
//!
//! let client = Client::new();
//! // Map each provider to its API key (here: none needed for a tool-only workflow).
//! let keys: HashMap<String, String> = HashMap::new();
//! // Values for the workflow's `# inputs:` (none declared here).
//! let inputs = spawningpool::workflow::resolve_inputs(&workflow.inputs, &HashMap::new())?;
//! let result = eval(&workflow, &registry, &tools, &client, &keys, &inputs, &workflows).await?;
//! println!("{result}");
//! # Ok(())
//! # }
//! ```

pub mod ast;
pub mod check;
mod collector;
pub mod eval;
pub mod parse;
pub mod render;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::domain::Registry;
use crate::types::{Param, Type};

pub use ast::{AccessKey, BinOp, Expr, Statement, Workflow};
pub use check::{check, specialist_return_type, TypeEnv, TypeError};
pub use eval::{eval, WorkflowError};
pub use parse::{parse, ParseError};
pub use render::mermaid;

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

/// Turn the raw `KEY=VALUE` strings supplied at run time into a value for each
/// declared input, coerced to its declared [`Type`] (workflow-dsl.md §5.1).
///
/// `provided` maps an input name to its raw string (as passed to `run workflow
/// --arg`). Every declared input must be present, and every provided key must
/// name a declared input — both are reported as errors, so a typo'd or missing
/// input fails before the workflow runs rather than as an `undefined variable`
/// mid-evaluation.
pub fn resolve_inputs(
    inputs: &[Param],
    provided: &HashMap<String, String>,
) -> Result<HashMap<String, serde_json::Value>, String> {
    for key in provided.keys() {
        if !inputs.iter().any(|p| &p.name == key) {
            return Err(format!("workflow has no input `{key}`"));
        }
    }

    let mut resolved = HashMap::new();
    for param in inputs {
        let raw = provided.get(&param.name).ok_or_else(|| {
            format!(
                "missing required input `{0}`; supply it with --arg {0}=<value>",
                param.name
            )
        })?;
        let value =
            coerce_input(raw, &param.ty).map_err(|e| format!("input `{}`: {e}", param.name))?;
        resolved.insert(param.name.clone(), value);
    }
    Ok(resolved)
}

/// Coerce one raw input string into its declared [`Type`]. A `string` input
/// takes the raw text verbatim; `number`/`bool` parse the scalar; `[T]`/object
/// inputs parse the text as JSON and must match that top-level kind. Deeper
/// shape is trusted, matching v1's stance that declared types aren't verified at
/// runtime (workflow-dsl.md §2).
fn coerce_input(raw: &str, ty: &Type) -> Result<serde_json::Value, String> {
    match ty {
        Type::String => Ok(serde_json::Value::String(raw.to_string())),
        Type::Number => raw
            .trim()
            .parse::<f64>()
            .map(|n| serde_json::json!(n))
            .map_err(|_| format!("expected a number, got `{raw}`")),
        Type::Bool => match raw.trim() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(format!("expected `true` or `false`, got `{raw}`")),
        },
        Type::Array(_) => {
            let value: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("expected JSON array: {e}"))?;
            if value.is_array() {
                Ok(value)
            } else {
                Err(format!("expected a JSON array, got `{raw}`"))
            }
        }
        Type::Object(_) => {
            let value: serde_json::Value =
                serde_json::from_str(raw).map_err(|e| format!("expected JSON object: {e}"))?;
            if value.is_object() {
                Ok(value)
            } else {
                Err(format!("expected a JSON object, got `{raw}`"))
            }
        }
    }
}

/// The tools and specialists a workflow references. Resolve exactly these tools
/// (not the whole catalog) before evaluating, so an unrelated broken tool can't
/// block a workflow that doesn't use it; the specialists are used to pre-flight
/// API keys.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Referenced {
    /// Tool names to resolve: every `run tool` target, plus the tools each
    /// invoked specialist needs (looked up in the registry).
    pub tools: BTreeSet<String>,
    /// Specialist names invoked via `run specialist`.
    pub specialists: BTreeSet<String>,
    /// Workflow names invoked via `run workflow` (workflow-dsl.md §6.6). Only the
    /// directly-referenced workflows; resolving the *transitive* closure (a
    /// `run` target's own `run` targets) is the caller's job, since this walk
    /// can't see other workflows' sources.
    pub workflows: BTreeSet<String>,
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
        Expr::Do { body, .. } => collect(body, registry, refs),
        Expr::RunTool {
            tool,
            args,
            recover,
            recover_default,
        } => {
            refs.tools.insert(tool.clone());
            for (_, e) in args {
                collect(e, registry, refs);
            }
            for (_, e) in recover {
                collect(e, registry, refs);
            }
            if let Some(e) = recover_default {
                collect(e, registry, refs);
            }
        }
        Expr::RunWorkflow { workflow, args } => {
            refs.workflows.insert(workflow.clone());
            for (_, e) in args {
                collect(e, registry, refs);
            }
        }
        Expr::RunSpecialist { specialist, prompt } => {
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
#[path = "mod_tests.rs"]
mod tests;
