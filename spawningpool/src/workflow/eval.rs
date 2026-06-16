//! Async evaluator for the workflow DSL (workflow-dsl.md §5–6).
//!
//! Values are [`serde_json::Value`]s. Tool calls run scripts via
//! [`crate::run_script`]; specialist calls run via [`crate::run::run_specialist`].
//! The evaluator processes statements sequentially and returns the value of the
//! last statement (v1 — workflow output designation is deferred per §8.1).

use std::collections::HashMap;

use futures::future::LocalBoxFuture;

use crate::ai::{Client, StopReason};
use crate::domain::{Registry, ToolDef};
use crate::run::RunEvent;

use super::ast::{AccessKey, BinOp, Expr, Workflow};

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
}

type Env = HashMap<String, serde_json::Value>;

/// Evaluate a workflow, executing its statements in order.
///
/// `tools` must contain every tool referenced by `call` expressions in the
/// workflow, pre-resolved with script paths. `keys` maps a provider name to its
/// API key; each specialist call is authenticated with its own provider's key
/// (a provider absent from the map runs without one), and its constrained-decoding
/// mode comes from that provider's definition in `registry`. `inputs` supplies a
/// value for each declared `# inputs:` entry (workflow-dsl.md §5.1), seeded into
/// scope before the first statement; build it with [`super::resolve_inputs`].
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
) -> Result<serde_json::Value, WorkflowError> {
    let ctx = EvalCtx {
        registry,
        tools,
        client,
        keys,
    };
    let mut env: Env = inputs.clone();
    let mut last = serde_json::Value::Null;

    for stmt in &workflow.statements {
        let val = eval_expr(&stmt.expr, env.clone(), &ctx).await?;
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
) -> LocalBoxFuture<'ctx, Result<serde_json::Value, WorkflowError>> {
    Box::pin(async move {
        match expr {
            Expr::Str(s) => Ok(serde_json::Value::String(s.clone())),

            Expr::Num(n) => Ok(serde_json::json!(*n)),

            Expr::Bool(b) => Ok(serde_json::Value::Bool(*b)),

            Expr::Object(fields) => {
                let mut map = serde_json::Map::new();
                for (k, v_expr) in fields {
                    let val = eval_expr(v_expr, env.clone(), ctx).await?;
                    map.insert(k.clone(), val);
                }
                Ok(serde_json::Value::Object(map))
            }

            Expr::Var(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| WorkflowError(format!("undefined variable `{name}`"))),

            Expr::Not(inner) => {
                let val = eval_expr(inner, env, ctx).await?;
                let b = val.as_bool().ok_or_else(|| {
                    WorkflowError(format!("operator `!` requires bool, got {val}"))
                })?;
                Ok(serde_json::Value::Bool(!b))
            }

            Expr::BinOp { op, lhs, rhs } => {
                let l = eval_expr(lhs, env.clone(), ctx).await?;
                let r = eval_expr(rhs, env, ctx).await?;
                eval_binop(op, l, r)
            }

            Expr::Access { base, keys } => {
                let mut val = eval_expr(base, env.clone(), ctx).await?;
                for key in keys {
                    val = eval_access(val, key, env.clone(), ctx).await?;
                }
                Ok(val)
            }

            Expr::If { branches, default } => {
                for (cond, result) in branches {
                    let cond_val = eval_expr(cond, env.clone(), ctx).await?;
                    let b = cond_val.as_bool().ok_or_else(|| {
                        WorkflowError(format!("if condition must be bool, got {cond_val}"))
                    })?;
                    if b {
                        return eval_expr(result, env, ctx).await;
                    }
                }
                eval_expr(default, env, ctx).await
            }

            Expr::For { item, array, body } => {
                let arr_val = eval_expr(array, env.clone(), ctx).await?;
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
                    let result = eval_expr(body, inner_env, ctx).await?;
                    results.push(result);
                }
                Ok(serde_json::Value::Array(results))
            }

            Expr::Call { tool, args } => {
                let tool_def = ctx
                    .tools
                    .iter()
                    .find(|t| t.name == *tool)
                    .ok_or_else(|| WorkflowError(format!("unknown tool `{tool}`")))?;

                let mut vars = HashMap::new();
                for (key, val_expr) in args {
                    let val = eval_expr(val_expr, env.clone(), ctx).await?;
                    let s = match &val {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    vars.insert(key.clone(), s);
                }

                let run = crate::run_script(&tool_def.script, &vars)
                    .map_err(|e| WorkflowError(format!("failed to run tool `{tool}`: {e}")))?;

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

            Expr::Ask {
                specialist: spec_name,
                prompt,
            } => {
                let prompt_val = eval_expr(prompt, env, ctx).await?;
                let prompt_str = prompt_val.as_str().ok_or_else(|| {
                    WorkflowError(format!("ask prompt must be a string, got {prompt_val}"))
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
        }
    })
}

/// Apply a single access key to a JSON value, returning the accessed element.
fn eval_access<'ctx>(
    val: serde_json::Value,
    key: &'ctx AccessKey,
    env: Env,
    ctx: &'ctx EvalCtx<'ctx>,
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
                let idx = eval_expr(expr, env, ctx).await?;
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
    }
}

fn num_val(v: &serde_json::Value, op: &str) -> Result<f64, WorkflowError> {
    v.as_f64()
        .ok_or_else(|| WorkflowError(format!("operator `{op}` requires a number, got {v}")))
}

/// Accumulates [`RunEvent`]s from a specialist run into the JSON envelope
/// (workflow-dsl.md §4).
#[derive(Default)]
struct Collector {
    output: String,
    thinking: String,
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: Option<StopReason>,
    turns: u32,
    tool_calls: Vec<serde_json::Value>,
}

impl Collector {
    fn observe(&mut self, event: RunEvent<'_>) {
        match event {
            RunEvent::TextDelta(t) | RunEvent::Text(t) => self.output.push_str(t),
            RunEvent::ThinkingDelta(t) | RunEvent::Thinking(t) => self.thinking.push_str(t),
            RunEvent::TurnDone { stop_reason } => {
                self.stop_reason = Some(stop_reason);
                self.turns += 1;
            }
            RunEvent::Usage(u) => {
                self.input_tokens += u.input;
                self.output_tokens += u.output;
            }
            RunEvent::ToolRan {
                name,
                output,
                success,
            } => {
                self.tool_calls.push(serde_json::json!({
                    "name": name,
                    "success": success,
                    "output": output,
                }));
            }
            RunEvent::ToolFailed { name, message } => {
                self.tool_calls.push(serde_json::json!({
                    "name": name,
                    "success": false,
                    "output": message,
                }));
            }
        }
    }

    fn into_envelope(self, specialist: &str, model: &str) -> serde_json::Value {
        serde_json::json!({
            "output": self.output,
            "thinking": self.thinking,
            "inputTokens": self.input_tokens,
            "outputTokens": self.output_tokens,
            "stopReason": self.stop_reason,
            "model": model,
            "specialist": specialist,
            "turns": self.turns,
            "toolCalls": self.tool_calls,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::parse::parse;

    async fn eval_src(src: &str) -> Result<serde_json::Value, WorkflowError> {
        let wf = parse(src).expect("parse failed");
        let registry = Registry::default();
        let client = crate::ai::Client::new();
        let keys = HashMap::new();
        let inputs = HashMap::new();
        eval(&wf, &registry, &[], &client, &keys, &inputs).await
    }

    #[tokio::test]
    async fn evaluates_string_literal() {
        let v = eval_src(r#"x = "hello""#).await.unwrap();
        assert_eq!(v, serde_json::json!("hello"));
    }

    #[tokio::test]
    async fn evaluates_number_literal() {
        let v = eval_src("x = 42").await.unwrap();
        assert_eq!(v, serde_json::json!(42.0));
    }

    #[tokio::test]
    async fn evaluates_bool_literal() {
        let v = eval_src("x = true").await.unwrap();
        assert_eq!(v, serde_json::json!(true));
    }

    #[tokio::test]
    async fn seeds_declared_inputs_into_scope() {
        let wf = parse("# inputs: CITY:string\n\ngreeting = \"hi \" + CITY").unwrap();
        let registry = Registry::default();
        let client = crate::ai::Client::new();
        let keys = HashMap::new();
        let mut inputs = HashMap::new();
        inputs.insert("CITY".to_string(), serde_json::json!("Portland"));
        let v = eval(&wf, &registry, &[], &client, &keys, &inputs)
            .await
            .unwrap();
        assert_eq!(v, serde_json::json!("hi Portland"));
    }

    #[tokio::test]
    async fn evaluates_string_concatenation() {
        let v = eval_src(r#"x = "hello" + " " + "world""#).await.unwrap();
        assert_eq!(v, serde_json::json!("hello world"));
    }

    #[tokio::test]
    async fn evaluates_arithmetic() {
        let v = eval_src("x = 2 + 3 * 4").await.unwrap();
        // Left-to-right: (2+3)*4 = 20
        assert_eq!(v, serde_json::json!(20.0));
    }

    #[tokio::test]
    async fn evaluates_power() {
        let v = eval_src("x = 2 ^ 10").await.unwrap();
        assert_eq!(v, serde_json::json!(1024.0));
    }

    #[tokio::test]
    async fn evaluates_logical_ops() {
        let v = eval_src("x = true && false").await.unwrap();
        assert_eq!(v, serde_json::json!(false));
        let v = eval_src("x = false || true").await.unwrap();
        assert_eq!(v, serde_json::json!(true));
    }

    #[tokio::test]
    async fn evaluates_not() {
        let v = eval_src("x = !true").await.unwrap();
        assert_eq!(v, serde_json::json!(false));
    }

    #[tokio::test]
    async fn evaluates_object_literal() {
        let v = eval_src(r#"x = { "a": 1, "b": "hi" }"#).await.unwrap();
        assert_eq!(v, serde_json::json!({"a": 1.0, "b": "hi"}));
    }

    #[tokio::test]
    async fn evaluates_variable_reference() {
        let v = eval_src("city = \"Portland\"\n\nname = city")
            .await
            .unwrap();
        assert_eq!(v, serde_json::json!("Portland"));
    }

    #[tokio::test]
    async fn last_statement_value_is_returned() {
        let v = eval_src("a = 1\n\nb = 2\n\nc = 3").await.unwrap();
        assert_eq!(v, serde_json::json!(3.0));
    }

    #[tokio::test]
    async fn evaluates_member_access() {
        let v = eval_src("obj = { \"x\": 42 }\n\nv = obj.x").await.unwrap();
        assert_eq!(v, serde_json::json!(42.0));
    }

    #[tokio::test]
    async fn evaluates_array_index_access() {
        // Build an array via a for-loop over a literal... actually we don't
        // have array literals. Use object field containing a computed value.
        // Simpler: just verify indexed access works on a known JSON value by
        // testing eval_access directly.
        let arr = serde_json::json!([10, 20, 30]);
        let registry = Registry::default();
        let client = crate::ai::Client::new();
        let keys = HashMap::new();
        let ctx = EvalCtx {
            registry: &registry,
            tools: &[],
            client: &client,
            keys: &keys,
        };
        let val = eval_access(arr, &AccessKey::Index(1), Env::new(), &ctx)
            .await
            .unwrap();
        assert_eq!(val, serde_json::json!(20));
    }

    #[tokio::test]
    async fn evaluates_if_true_branch() {
        let v = eval_src(r#"v = if (true) "yes", (_) "no""#).await.unwrap();
        assert_eq!(v, serde_json::json!("yes"));
    }

    #[tokio::test]
    async fn evaluates_if_default_branch() {
        let v = eval_src(r#"v = if (false) "yes", (_) "no""#).await.unwrap();
        assert_eq!(v, serde_json::json!("no"));
    }

    #[tokio::test]
    async fn evaluates_for_as_map() {
        // We can't create arrays via literals, so build from an Object field
        // holding a pre-known array-typed variable. Actually, the simplest
        // test: for over an empty array.
        // Use a script-backed tool to produce an array, or test via direct
        // eval with a seeded env.
        let wf = parse("doubled = for [x: nums] (x)").unwrap();
        let mut env = Env::new();
        env.insert("nums".to_string(), serde_json::json!([1, 2, 3]));
        let registry = Registry::default();
        let client = crate::ai::Client::new();
        let keys = HashMap::new();
        let ctx = EvalCtx {
            registry: &registry,
            tools: &[],
            client: &client,
            keys: &keys,
        };

        let mut last = serde_json::Value::Null;
        for stmt in &wf.statements {
            let val = eval_expr(&stmt.expr, env.clone(), &ctx).await.unwrap();
            last = val.clone();
            env.insert(stmt.name.clone(), val);
        }
        assert_eq!(last, serde_json::json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn call_runs_tool_script_and_reads_output() {
        use std::os::unix::fs::PermissionsExt;

        let script_path = std::env::temp_dir().join(format!(
            "sp_wf_tool_{}_{}.sh",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &script_path,
            "#!/bin/sh\nprintf '{\"greeting\":\"hello %s\"}' \"$NAME\" > \"$SP_OUTPUT_PATH\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let tool_def = ToolDef {
            name: "greet".to_string(),
            script: script_path.clone(),
            description: String::new(),
            params: vec![crate::types::Param {
                name: "NAME".to_string(),
                ty: crate::types::Type::String,
            }],
            output: Some(crate::types::Type::Object(vec![(
                "greeting".to_string(),
                crate::types::Type::String,
            )])),
        };

        let wf = parse(r#"result = call greet { NAME: "world" }"#).unwrap();
        let registry = Registry::default();
        let client = crate::ai::Client::new();
        let keys = HashMap::new();

        let inputs = HashMap::new();
        let val = eval(&wf, &registry, &[tool_def], &client, &keys, &inputs)
            .await
            .unwrap();
        std::fs::remove_file(&script_path).ok();

        assert_eq!(val, serde_json::json!({"greeting": "hello world"}));
    }

    #[tokio::test]
    async fn call_errors_when_tool_omits_sp_output_path() {
        use std::os::unix::fs::PermissionsExt;

        let script_path = std::env::temp_dir().join(format!(
            "sp_wf_noout_{}_{}.sh",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&script_path, "#!/bin/sh\necho ok\n").unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let tool_def = ToolDef {
            name: "silent".to_string(),
            script: script_path.clone(),
            description: String::new(),
            params: vec![],
            output: Some(crate::types::Type::String),
        };

        let wf = parse("result = call silent {}").unwrap();
        let registry = Registry::default();
        let client = crate::ai::Client::new();
        let keys = HashMap::new();

        let inputs = HashMap::new();
        let err = eval(&wf, &registry, &[tool_def], &client, &keys, &inputs)
            .await
            .unwrap_err();
        std::fs::remove_file(&script_path).ok();

        assert!(err.0.contains("SP_OUTPUT_PATH"));
    }
}
