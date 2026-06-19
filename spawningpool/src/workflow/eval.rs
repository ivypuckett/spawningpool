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
use crate::log::{self, LogSink, SpecialistLog};

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

/// The result of putting an `ask` question to the user (workflow-dsl.md §6.8,
/// docs/ask.md §5). Split the same way the rest of the DSL splits errors from
/// data: an [`AskOutcome::Answered`] reply (including the empty string for a bare
/// enter) is in-band data; [`AskOutcome::Unavailable`] is the out-of-band
/// "couldn't ask" — a headless run with no front-end, or the user cancelling /
/// closing input — that triggers the `ask`'s `else` fallback, or aborts when
/// there is none.
pub enum AskOutcome {
    /// The user replied; carries their answer verbatim (may be `""`).
    Answered(String),
    /// The question couldn't be put to anyone.
    Unavailable,
}

/// Handles an `ask`: given the prompt string, returns the user's [`AskOutcome`].
/// Injected by the caller so the library stays decoupled from any particular
/// front-end (the CLI reads a TTY; tests supply a canned handler).
pub type AskHandler<'a> = dyn Fn(&str) -> AskOutcome + 'a;

/// Bundles the immutable evaluation context passed through all recursive calls.
struct EvalCtx<'a> {
    registry: &'a Registry,
    tools: &'a [ToolDef],
    client: &'a Client,
    keys: &'a HashMap<String, String>,
    workflows: &'a HashMap<String, Workflow>,
    ask: &'a AskHandler<'a>,
    log: &'a LogSink<'a>,
}

type Env = HashMap<String, serde_json::Value>;

/// The logging context for the expression currently being evaluated: the name of
/// the workflow frame and the statement variable being assigned. Threaded
/// unchanged through an expression's sub-evaluations so any `tool.*`,
/// `specialist.*`, or `ask.*` event nested inside it carries the right `wf`/`stmt`
/// (docs/workflow-logging.md).
#[derive(Clone, Copy)]
struct Frame<'a> {
    wf: &'a str,
    stmt: &'a str,
}

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
/// parsed AST; an empty map is fine for a workflow that never uses `run`. `ask`
/// handles each `ask` expression (workflow-dsl.md §6.8): given the prompt, it
/// returns the user's [`AskOutcome`]; a handler that always returns
/// [`AskOutcome::Unavailable`] models a headless run. `name` is the root
/// workflow's name, used as the `wf` field of its log events. `log` records the
/// run's structured events (docs/workflow-logging.md); a no-op sink disables
/// logging.
///
/// Returns the value produced by the last statement, or `Null` if the workflow
/// has no statements. Tool and specialist outputs compose through JSON values.
#[allow(clippy::too_many_arguments)]
pub async fn eval(
    name: &str,
    workflow: &Workflow,
    registry: &Registry,
    tools: &[ToolDef],
    client: &Client,
    keys: &HashMap<String, String>,
    inputs: &HashMap<String, serde_json::Value>,
    workflows: &HashMap<String, Workflow>,
    ask: &AskHandler<'_>,
    log: &LogSink<'_>,
) -> Result<serde_json::Value, WorkflowError> {
    let ctx = EvalCtx {
        registry,
        tools,
        client,
        keys,
        workflows,
        ask,
        log,
    };
    eval_workflow(name, workflow, &ctx, inputs, Vec::new()).await
}

/// Evaluate one workflow's statements in order, seeded with `inputs`, returning
/// the last statement's value (or `Null` when there are none). `name` is the
/// workflow's name (the `wf` field of its log events). `visited` is the stack of
/// `run`-nested workflow names in progress, so a cycle is caught rather than
/// recursing forever.
///
/// The frame is bracketed by a `workflow.start` and exactly one terminal
/// `workflow.done` / `workflow.error` event; a failing sub-workflow therefore
/// logs its own `workflow.error` and so does each frame above it.
async fn eval_workflow<'a>(
    name: &'a str,
    workflow: &'a Workflow,
    ctx: &'a EvalCtx<'a>,
    inputs: &HashMap<String, serde_json::Value>,
    visited: Vec<String>,
) -> Result<serde_json::Value, WorkflowError> {
    let started = std::time::Instant::now();
    let inputs_obj = serde_json::Value::Object(inputs.clone().into_iter().collect());
    (ctx.log)(log::workflow_start(name, &inputs_obj));

    let mut env: Env = inputs.clone();
    let mut last = serde_json::Value::Null;

    for stmt in &workflow.statements {
        let frame = Frame {
            wf: name,
            stmt: &stmt.name,
        };
        let val = match eval_expr(&stmt.expr, env.clone(), ctx, visited.clone(), frame).await {
            Ok(val) => val,
            Err(e) => {
                (ctx.log)(log::workflow_error(
                    name,
                    started.elapsed().as_millis() as u64,
                    &e.0,
                ));
                return Err(e);
            }
        };
        last = val.clone();
        env.insert(stmt.name.clone(), val);
    }

    (ctx.log)(log::workflow_done(
        name,
        started.elapsed().as_millis() as u64,
    ));
    Ok(last)
}

/// Evaluate a single expression. Uses [`LocalBoxFuture`] to support recursive
/// async evaluation without infinite type expansion. `frame` carries the
/// workflow/statement logging context for any event the expression emits, and is
/// passed unchanged into its sub-evaluations.
fn eval_expr<'ctx>(
    expr: &'ctx Expr,
    env: Env,
    ctx: &'ctx EvalCtx<'ctx>,
    visited: Vec<String>,
    frame: Frame<'ctx>,
) -> LocalBoxFuture<'ctx, Result<serde_json::Value, WorkflowError>> {
    Box::pin(async move {
        match expr {
            Expr::Str(s) => Ok(serde_json::Value::String(s.clone())),

            Expr::Num(n) => Ok(serde_json::json!(*n)),

            Expr::Bool(b) => Ok(serde_json::Value::Bool(*b)),

            Expr::Object(fields) => {
                let mut map = serde_json::Map::new();
                for (k, v_expr) in fields {
                    let val = eval_expr(v_expr, env.clone(), ctx, visited.clone(), frame).await?;
                    map.insert(k.clone(), val);
                }
                Ok(serde_json::Value::Object(map))
            }

            Expr::Var(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| WorkflowError(format!("undefined variable `{name}`"))),

            Expr::Not(inner) => {
                let val = eval_expr(inner, env, ctx, visited.clone(), frame).await?;
                let b = val.as_bool().ok_or_else(|| {
                    WorkflowError(format!("operator `!` requires bool, got {val}"))
                })?;
                Ok(serde_json::Value::Bool(!b))
            }

            Expr::BinOp { op, lhs, rhs } => {
                let l = eval_expr(lhs, env.clone(), ctx, visited.clone(), frame).await?;
                let r = eval_expr(rhs, env, ctx, visited.clone(), frame).await?;
                eval_binop(op, l, r)
            }

            Expr::Access { base, keys } => {
                let mut val = eval_expr(base, env.clone(), ctx, visited.clone(), frame).await?;
                for key in keys {
                    val = eval_access(val, key, env.clone(), ctx, visited.clone(), frame).await?;
                }
                Ok(val)
            }

            Expr::If { branches, default } => {
                for (cond, result) in branches {
                    let cond_val =
                        eval_expr(cond, env.clone(), ctx, visited.clone(), frame).await?;
                    let b = cond_val.as_bool().ok_or_else(|| {
                        WorkflowError(format!("if condition must be bool, got {cond_val}"))
                    })?;
                    if b {
                        return eval_expr(result, env, ctx, visited.clone(), frame).await;
                    }
                }
                eval_expr(default, env, ctx, visited.clone(), frame).await
            }

            Expr::For { item, array, body } => {
                let arr_val = eval_expr(array, env.clone(), ctx, visited.clone(), frame).await?;
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
                    let result = eval_expr(body, inner_env, ctx, visited.clone(), frame).await?;
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
                let max_val = eval_expr(max, env.clone(), ctx, visited.clone(), frame).await?;
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
                    let val = eval_expr(body, env.clone(), ctx, visited.clone(), frame).await?;
                    count += 1.0;
                    if count >= cap {
                        return Ok(val);
                    }
                    let mut cond_env = env.clone();
                    cond_env.insert(var.clone(), val.clone());
                    let again = eval_expr(cond, cond_env, ctx, visited.clone(), frame).await?;
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

                // Collect the evaluated arguments both as the `KEY=value` env
                // vars the script consumes and, for the log, as a typed object
                // (before the string lowering) so `tool.call` shows real values.
                let mut vars = HashMap::new();
                let mut arg_obj = serde_json::Map::new();
                for (key, val_expr) in args {
                    let val = eval_expr(val_expr, env.clone(), ctx, visited.clone(), frame).await?;
                    let s = match &val {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    vars.insert(key.clone(), s);
                    arg_obj.insert(key.clone(), val);
                }

                (ctx.log)(log::tool_call(
                    Some(frame.wf),
                    Some(frame.stmt),
                    None,
                    tool,
                    &serde_json::Value::Object(arg_obj),
                ));
                let started = std::time::Instant::now();
                let run_result = crate::run_script(&tool_def.script, &vars);
                let elapsed_ms = started.elapsed().as_millis() as u64;
                match &run_result {
                    Ok(run) => (ctx.log)(log::tool_done(
                        Some(frame.wf),
                        Some(frame.stmt),
                        None,
                        tool,
                        run.success,
                        run.code,
                        elapsed_ms,
                    )),
                    // A launch failure has no exit code (script not executable,
                    // missing shebang, or path not found) — `exit_code: null`.
                    Err(_) => (ctx.log)(log::tool_done(
                        Some(frame.wf),
                        Some(frame.stmt),
                        None,
                        tool,
                        false,
                        None,
                        elapsed_ms,
                    )),
                }
                let run = run_result
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
                        Some(arm) => eval_expr(arm, env, ctx, visited.clone(), frame).await,
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
                let prompt_val = eval_expr(prompt, env, ctx, visited.clone(), frame).await?;
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

                // Route the specialist's own `specialist.*` / `tool.*` events to
                // this run's sink, tagged with the current workflow/statement.
                let spec_log = SpecialistLog {
                    sink: ctx.log,
                    wf: Some(frame.wf),
                    stmt: Some(frame.stmt),
                };
                let mut collected = Collector::default();
                crate::run::run_specialist(
                    ctx.client,
                    ctx.registry,
                    specialist,
                    prompt_str,
                    &spec_tools,
                    &spec_opts,
                    &mut |event| collected.observe(event),
                    Some(&spec_log),
                )
                .await
                .map_err(|e| WorkflowError(format!("specialist `{spec_name}` failed: {e}")))?;

                Ok(collected.into_envelope(spec_name, &specialist.model))
            }

            Expr::Ask { prompt, fallback } => {
                let prompt_val =
                    eval_expr(prompt, env.clone(), ctx, visited.clone(), frame).await?;
                let prompt_str = prompt_val.as_str().ok_or_else(|| {
                    WorkflowError(format!("ask prompt must be a string, got {prompt_val}"))
                })?;

                (ctx.log)(log::ask_prompt(
                    Some(frame.wf),
                    Some(frame.stmt),
                    prompt_str,
                ));
                let outcome = (ctx.ask)(prompt_str);
                (ctx.log)(log::ask_answer(
                    Some(frame.wf),
                    Some(frame.stmt),
                    matches!(outcome, AskOutcome::Answered(_)),
                ));
                match outcome {
                    // In-band: the reply is ordinary string data (incl. "").
                    AskOutcome::Answered(answer) => Ok(serde_json::Value::String(answer)),
                    // Out-of-band: recover with the `else` fallback, or abort.
                    AskOutcome::Unavailable => match fallback {
                        Some(fallback) => {
                            eval_expr(fallback, env, ctx, visited.clone(), frame).await
                        }
                        None => Err(WorkflowError(format!(
                            "ask couldn't be answered (headless run or cancelled) \
                             and has no `else` fallback: {prompt_str}"
                        ))),
                    },
                }
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
                    let val = eval_expr(val_expr, env.clone(), ctx, visited.clone(), frame).await?;
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
                eval_workflow(wf_name, callee, ctx, &sub_inputs, chain).await
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
    frame: Frame<'ctx>,
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
                let idx = eval_expr(expr, env, ctx, visited.clone(), frame).await?;
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
