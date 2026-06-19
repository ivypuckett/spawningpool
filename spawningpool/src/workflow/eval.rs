//! Async evaluator for the workflow DSL (workflow-dsl.md §5–6).
//!
//! Values are [`serde_json::Value`]s. Tool calls run scripts via
//! [`crate::run_script`]; specialist calls run via [`crate::run::run_specialist`].
//! The evaluator processes statements sequentially and returns the value of the
//! last statement (v1 — workflow output designation is deferred per §8.1).

use std::collections::HashMap;

use futures::future::LocalBoxFuture;

use crate::ai::Client;
use crate::domain::{Registry, ToolDef};

use super::ast::{AccessKey, BinOp, Expr, Workflow};
use super::collector::Collector;

/// An evaluation error.
#[derive(Debug)]
pub struct WorkflowError(pub String);

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for WorkflowError {}

/// Bundles the immutable evaluation context passed through all recursive calls.
struct EvalCtx<'a> {
    registry: &'a Registry,
    tools: &'a [ToolDef],
    client: &'a Client,
    keys: &'a HashMap<String, String>,
    workflows: &'a HashMap<String, Workflow>,
}

type Env = HashMap<String, serde_json::Value>;

/// Evaluate a workflow, executing its statements in order.
///
/// `tools` must contain every tool referenced by `run tool` expressions in the
/// workflow, pre-resolved with script paths. `keys` maps a provider name to its
/// API key; each specialist call is authenticated with its own provider's key
/// (a provider absent from the map runs without one), and its constrained-decoding
/// mode comes from that provider's definition in `registry`. `inputs` supplies a
/// value for each declared `# inputs:` entry (workflow-dsl.md §5.1), seeded into
/// scope before the first statement; build it with [`super::resolve_inputs`].
/// `workflows` maps each name a `run` may invoke (the transitive closure) to its
/// parsed AST; an empty map is fine for a workflow that never uses `run`.
///
/// Returns the value produced by the last statement, or `Null` if the workflow
/// has no statements. Tool and specialist outputs compose through JSON values.
pub async fn eval(
    workflow: &Workflow,
    registry: &Registry,
    tools: &[ToolDef],
    client: &Client,
    keys: &HashMap<String, String>,
    inputs: &HashMap<String, serde_json::Value>,
    workflows: &HashMap<String, Workflow>,
) -> Result<serde_json::Value, WorkflowError> {
    let ctx = EvalCtx {
        registry,
        tools,
        client,
        keys,
        workflows,
    };
    eval_workflow(workflow, &ctx, inputs, Vec::new()).await
}

/// Evaluate one workflow's statements in order, seeded with `inputs`, returning
/// the last statement's value (or `Null` when there are none). `visited` is the
/// stack of `run`-nested workflow names in progress, so a cycle is caught rather
/// than recursing forever.
async fn eval_workflow<'a>(
    workflow: &'a Workflow,
    ctx: &'a EvalCtx<'a>,
    inputs: &HashMap<String, serde_json::Value>,
    visited: Vec<String>,
) -> Result<serde_json::Value, WorkflowError> {
    let mut env: Env = inputs.clone();
    let mut last = serde_json::Value::Null;

    for stmt in &workflow.statements {
        let val = eval_expr(&stmt.expr, env.clone(), ctx, visited.clone()).await?;
        last = val.clone();
        env.insert(stmt.name.clone(), val);
    }

    Ok(last)
}

/// Evaluate a single expression. Uses [`LocalBoxFuture`] to support recursive
/// async evaluation without infinite type expansion.
fn eval_expr<'ctx>(
    expr: &'ctx Expr,
    env: Env,
    ctx: &'ctx EvalCtx<'ctx>,
    visited: Vec<String>,
) -> LocalBoxFuture<'ctx, Result<serde_json::Value, WorkflowError>> {
    Box::pin(async move {
        match expr {
            Expr::Str(s) => Ok(serde_json::Value::String(s.clone())),

            Expr::Num(n) => Ok(serde_json::json!(*n)),

            Expr::Bool(b) => Ok(serde_json::Value::Bool(*b)),

            Expr::Object(fields) => {
                let mut map = serde_json::Map::new();
                for (k, v_expr) in fields {
                    let val = eval_expr(v_expr, env.clone(), ctx, visited.clone()).await?;
                    map.insert(k.clone(), val);
                }
                Ok(serde_json::Value::Object(map))
            }

            Expr::Var(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| WorkflowError(format!("undefined variable `{name}`"))),

            Expr::Not(inner) => {
                let val = eval_expr(inner, env, ctx, visited.clone()).await?;
                let b = val.as_bool().ok_or_else(|| {
                    WorkflowError(format!("operator `!` requires bool, got {val}"))
                })?;
                Ok(serde_json::Value::Bool(!b))
            }

            Expr::BinOp { op, lhs, rhs } => {
                let l = eval_expr(lhs, env.clone(), ctx, visited.clone()).await?;
                let r = eval_expr(rhs, env, ctx, visited.clone()).await?;
                eval_binop(op, l, r)
            }

            Expr::Access { base, keys } => {
                let mut val = eval_expr(base, env.clone(), ctx, visited.clone()).await?;
                for key in keys {
                    val = eval_access(val, key, env.clone(), ctx, visited.clone()).await?;
                }
                Ok(val)
            }

            Expr::If { branches, default } => {
                for (cond, result) in branches {
                    let cond_val = eval_expr(cond, env.clone(), ctx, visited.clone()).await?;
                    let b = cond_val.as_bool().ok_or_else(|| {
                        WorkflowError(format!("if condition must be bool, got {cond_val}"))
                    })?;
                    if b {
                        return eval_expr(result, env, ctx, visited.clone()).await;
                    }
                }
                eval_expr(default, env, ctx, visited.clone()).await
            }

            Expr::For { item, array, body } => {
                let arr_val = eval_expr(array, env.clone(), ctx, visited.clone()).await?;
                let elements = arr_val
                    .as_array()
                    .ok_or_else(|| {
                        WorkflowError(format!("for loop requires array, got {arr_val}"))
                    })?
                    .clone();
                let mut results = Vec::with_capacity(elements.len());
                for element in elements {
                    let mut inner_env = env.clone();
                    inner_env.insert(item.clone(), element);
                    let result = eval_expr(body, inner_env, ctx, visited.clone()).await?;
                    results.push(result);
                }
                Ok(serde_json::Value::Array(results))
            }

            Expr::Do {
                var,
                body,
                cond,
                max,
            } => {
                // Cap the loop up front, in the outer scope (workflow-dsl.md §6.5).
                let max_val = eval_expr(max, env.clone(), ctx, visited.clone()).await?;
                let cap = max_val.as_f64().ok_or_else(|| {
                    WorkflowError(format!("do loop `max` must be a number, got {max_val}"))
                })?;
                if cap.is_nan() || cap < 1.0 {
                    return Err(WorkflowError(format!(
                        "do loop `max` must be at least 1, got {cap}"
                    )));
                }
                // Run the body at least once, then re-run while `cond` holds and
                // the cap allows. `cond` sees the latest value bound to `var`.
                let mut count = 0.0;
                loop {
                    let val = eval_expr(body, env.clone(), ctx, visited.clone()).await?;
                    count += 1.0;
                    if count >= cap {
                        return Ok(val);
                    }
                    let mut cond_env = env.clone();
                    cond_env.insert(var.clone(), val.clone());
                    let again = eval_expr(cond, cond_env, ctx, visited.clone()).await?;
                    match again.as_bool() {
                        Some(true) => continue,
                        Some(false) => return Ok(val),
                        None => {
                            return Err(WorkflowError(format!(
                                "do loop `while` condition must be a bool, got {again}"
                            )))
                        }
                    }
                }
            }

            Expr::RunTool {
                tool,
                args,
                recover,
                recover_default,
            } => {
                let tool_def = ctx
                    .tools
                    .iter()
                    .find(|t| t.name == *tool)
                    .ok_or_else(|| WorkflowError(format!("unknown tool `{tool}`")))?;

                let mut vars = HashMap::new();
                for (key, val_expr) in args {
                    let val = eval_expr(val_expr, env.clone(), ctx, visited.clone()).await?;
                    let s = match &val {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    vars.insert(key.clone(), s);
                }

                let run = crate::run_script(&tool_def.script, &vars)
                    .map_err(|e| WorkflowError(format!("failed to run tool `{tool}`: {e}")))?;

                // A non-zero exit (workflow-dsl.md §7) is recovered by the `else`
                // block: map the code to its declared name and take that arm,
                // falling back to the `_` default. With no matching arm the
                // failure aborts the workflow.
                if !run.success {
                    let arm = run
                        .code
                        .and_then(|code| tool_def.exits.iter().find(|e| e.code == code))
                        .and_then(|exit| recover.iter().find(|(name, _)| *name == exit.name))
                        .map(|(_, arm)| arm)
                        .or(recover_default.as_deref());
                    return match arm {
                        Some(arm) => eval_expr(arm, env, ctx, visited.clone()).await,
                        None => Err(WorkflowError(format!(
                            "tool `{tool}` exited with {} and no `else` arm handles it",
                            match run.code {
                                Some(code) => format!("code {code}"),
                                None => "a signal".to_string(),
                            }
                        ))),
                    };
                }

                let output_str = run.structured_output.ok_or_else(|| {
                    WorkflowError(format!(
                        "tool `{tool}` didn't write to $SP_OUTPUT_PATH; \
                         workflow tools must write their structured result there"
                    ))
                })?;

                serde_json::from_str(&output_str).map_err(|e| {
                    WorkflowError(format!("tool `{tool}` output is not valid JSON: {e}"))
                })
            }

            Expr::RunSpecialist {
                specialist: spec_name,
                prompt,
            } => {
                let prompt_val = eval_expr(prompt, env, ctx, visited.clone()).await?;
                let prompt_str = prompt_val.as_str().ok_or_else(|| {
                    WorkflowError(format!(
                        "specialist prompt must be a string, got {prompt_val}"
                    ))
                })?;

                let specialist = ctx
                    .registry
                    .specialists
                    .get(spec_name.as_str())
                    .ok_or_else(|| WorkflowError(format!("unknown specialist `{spec_name}`")))?;

                // Resolve the specialist's tools from those available in this
                // workflow context.
                let spec_tool_names = specialist.tool_names();
                let spec_tools: Vec<ToolDef> = spec_tool_names
                    .iter()
                    .map(|name| {
                        ctx.tools
                            .iter()
                            .find(|t| &t.name == name)
                            .cloned()
                            .ok_or_else(|| {
                                WorkflowError(format!(
                                    "specialist `{spec_name}` requires tool `{name}` \
                                     which isn't in this workflow's tool set"
                                ))
                            })
                    })
                    .collect::<Result<_, _>>()?;

                // Authenticate with the specialist's own provider: its key (if
                // supplied) and its declared constrained-decoding capability.
                let mut spec_opts = specialist.complete_options();
                if let Some(key) = ctx.keys.get(&specialist.provider) {
                    spec_opts.api_key = Some(key.clone());
                }
                if let Some(provider) = ctx.registry.providers.get(&specialist.provider) {
                    spec_opts.constrained_decoding = provider.constrained_decoding;
                }

                let mut collected = Collector::default();
                crate::run::run_specialist(
                    ctx.client,
                    ctx.registry,
                    specialist,
                    prompt_str,
                    &spec_tools,
                    &spec_opts,
                    &mut |event| collected.observe(event),
                )
                .await
                .map_err(|e| WorkflowError(format!("specialist `{spec_name}` failed: {e}")))?;

                Ok(collected.into_envelope(spec_name, &specialist.model))
            }

            Expr::RunWorkflow {
                workflow: wf_name,
                args,
            } => {
                let callee = ctx
                    .workflows
                    .get(wf_name)
                    .ok_or_else(|| WorkflowError(format!("unknown workflow `{wf_name}`")))?;

                // Evaluate each argument to a typed JSON value and seed it as the
                // callee's input — workflow inputs flow as values, not stringified
                // env vars the way `run tool` passes them.
                let mut sub_inputs = HashMap::new();
                for (key, val_expr) in args {
                    let val = eval_expr(val_expr, env.clone(), ctx, visited.clone()).await?;
                    sub_inputs.insert(key.clone(), val);
                }

                if visited.iter().any(|n| n == wf_name) {
                    return Err(WorkflowError(format!(
                        "workflow cycle detected: {} -> {wf_name}",
                        visited.join(" -> ")
                    )));
                }
                let mut chain = visited.clone();
                chain.push(wf_name.clone());
                eval_workflow(callee, ctx, &sub_inputs, chain).await
            }
        }
    })
}

/// Apply a single access key to a JSON value, returning the accessed element.
fn eval_access<'ctx>(
    val: serde_json::Value,
    key: &'ctx AccessKey,
    env: Env,
    ctx: &'ctx EvalCtx<'ctx>,
    visited: Vec<String>,
) -> LocalBoxFuture<'ctx, Result<serde_json::Value, WorkflowError>> {
    Box::pin(async move {
        match key {
            AccessKey::Ident(k) | AccessKey::Quoted(k) => match val {
                serde_json::Value::Object(map) => map
                    .get(k)
                    .cloned()
                    .ok_or_else(|| WorkflowError(format!("object has no field `{k}`"))),
                other => Err(WorkflowError(format!(
                    "cannot access field `{k}` on {other}"
                ))),
            },
            AccessKey::Index(i) => match val {
                serde_json::Value::Array(arr) => arr.get(*i).cloned().ok_or_else(|| {
                    WorkflowError(format!("array index {i} out of bounds (len {})", arr.len()))
                }),
                other => Err(WorkflowError(format!(
                    "integer index requires array, got {other}"
                ))),
            },
            AccessKey::Computed(expr) => {
                let idx = eval_expr(expr, env, ctx, visited.clone()).await?;
                match val {
                    serde_json::Value::Array(arr) => {
                        let i = idx.as_u64().ok_or_else(|| {
                            WorkflowError(format!(
                                "array index must be a non-negative integer, got {idx}"
                            ))
                        })? as usize;
                        arr.get(i).cloned().ok_or_else(|| {
                            WorkflowError(format!(
                                "array index {i} out of bounds (len {})",
                                arr.len()
                            ))
                        })
                    }
                    other => Err(WorkflowError(format!(
                        "computed access requires array, got {other}"
                    ))),
                }
            }
        }
    })
}

fn eval_binop(
    op: &BinOp,
    l: serde_json::Value,
    r: serde_json::Value,
) -> Result<serde_json::Value, WorkflowError> {
    match op {
        BinOp::Or => {
            let lb = l
                .as_bool()
                .ok_or_else(|| WorkflowError(format!("operator `||` requires bool, got {l}")))?;
            let rb = r
                .as_bool()
                .ok_or_else(|| WorkflowError(format!("operator `||` requires bool, got {r}")))?;
            Ok(serde_json::Value::Bool(lb || rb))
        }
        BinOp::And => {
            let lb = l
                .as_bool()
                .ok_or_else(|| WorkflowError(format!("operator `&&` requires bool, got {l}")))?;
            let rb = r
                .as_bool()
                .ok_or_else(|| WorkflowError(format!("operator `&&` requires bool, got {r}")))?;
            Ok(serde_json::Value::Bool(lb && rb))
        }
        BinOp::Add => match (&l, &r) {
            (serde_json::Value::String(ls), serde_json::Value::String(rs)) => {
                Ok(serde_json::Value::String(ls.clone() + rs))
            }
            (serde_json::Value::Number(_), serde_json::Value::Number(_)) => {
                let ln = num_val(&l, "+")?;
                let rn = num_val(&r, "+")?;
                Ok(serde_json::json!(ln + rn))
            }
            _ => Err(WorkflowError(format!(
                "operator `+` requires two strings or two numbers, got {l} and {r}"
            ))),
        },
        BinOp::Sub => {
            let ln = num_val(&l, "-")?;
            let rn = num_val(&r, "-")?;
            Ok(serde_json::json!(ln - rn))
        }
        BinOp::Mul => {
            let ln = num_val(&l, "*")?;
            let rn = num_val(&r, "*")?;
            Ok(serde_json::json!(ln * rn))
        }
        BinOp::Div => {
            let ln = num_val(&l, "/")?;
            let rn = num_val(&r, "/")?;
            Ok(serde_json::json!(ln / rn))
        }
        BinOp::Rem => {
            let ln = num_val(&l, "%")?;
            let rn = num_val(&r, "%")?;
            Ok(serde_json::json!(ln % rn))
        }
        BinOp::Pow => {
            let ln = num_val(&l, "^")?;
            let rn = num_val(&r, "^")?;
            Ok(serde_json::json!(ln.powf(rn)))
        }
        BinOp::Eq | BinOp::Ne => {
            let equal = json_eq(&l, &r);
            let result = if matches!(op, BinOp::Eq) {
                equal
            } else {
                !equal
            };
            Ok(serde_json::Value::Bool(result))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ordering = match (&l, &r) {
                (serde_json::Value::Number(_), serde_json::Value::Number(_)) => {
                    num_val(&l, "comparison")?.partial_cmp(&num_val(&r, "comparison")?)
                }
                (serde_json::Value::String(a), serde_json::Value::String(b)) => Some(a.cmp(b)),
                _ => {
                    return Err(WorkflowError(format!(
                        "comparison requires two numbers or two strings, got {l} and {r}"
                    )))
                }
            };
            // A `None` ordering means a NaN operand; per IEEE it compares false
            // in every direction.
            let result = match ordering {
                Some(std::cmp::Ordering::Less) => matches!(op, BinOp::Lt | BinOp::Le),
                Some(std::cmp::Ordering::Equal) => matches!(op, BinOp::Le | BinOp::Ge),
                Some(std::cmp::Ordering::Greater) => matches!(op, BinOp::Gt | BinOp::Ge),
                None => false,
            };
            Ok(serde_json::Value::Bool(result))
        }
    }
}

/// Structural equality that compares numbers by their `f64` value, so an integer
/// `1` from a tool's JSON output equals the `1` literal (always an `f64` here).
/// serde_json's own `==` distinguishes the integer and float representations, so
/// it can't be used directly for the DSL's `==`.
fn json_eq(l: &serde_json::Value, r: &serde_json::Value) -> bool {
    use serde_json::Value::{Array, Number, Object};
    match (l, r) {
        (Number(a), Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
        (Array(a), Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| json_eq(x, y)),
        (Object(a), Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|w| json_eq(v, w)))
        }
        _ => l == r,
    }
}

fn num_val(v: &serde_json::Value, op: &str) -> Result<f64, WorkflowError> {
    v.as_f64()
        .ok_or_else(|| WorkflowError(format!("operator `{op}` requires a number, got {v}")))
}

#[cfg(test)]
#[path = "eval_tests.rs"]
mod tests;
