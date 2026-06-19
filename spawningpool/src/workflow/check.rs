//! Static type-checker for the workflow DSL (workflow-dsl.md §2, §6).
//!
//! Types are inferred from literal values, from tool/specialist declarations,
//! and propagated through operators and access. Type errors are caught here;
//! runtime values are trusted to match the declared types (v1 — no runtime
//! verification per the spec).

use std::collections::HashMap;

use crate::domain::{Registry, ToolDef};
use crate::types::Type;

use super::ast::{AccessKey, BinOp, Expr, Workflow};

/// The type environment produced by a successful check: maps each workflow
/// variable to its inferred type.
pub type TypeEnv = HashMap<String, Type>;

/// A type error: describes what went wrong and where.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeError(pub String);

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for TypeError {}

/// The fixed return type of every specialist call (workflow-dsl.md §4).
pub fn specialist_return_type() -> Type {
    Type::Object(vec![
        ("output".to_string(), Type::String),
        ("thinking".to_string(), Type::String),
        ("inputTokens".to_string(), Type::Number),
        ("outputTokens".to_string(), Type::Number),
        ("stopReason".to_string(), Type::String),
        ("model".to_string(), Type::String),
        ("specialist".to_string(), Type::String),
        ("turns".to_string(), Type::Number),
        (
            "toolCalls".to_string(),
            Type::Array(Box::new(Type::Object(vec![
                ("name".to_string(), Type::String),
                ("success".to_string(), Type::Bool),
                ("output".to_string(), Type::String),
            ]))),
        ),
    ])
}

/// The immutable context a check runs against: the registry, the resolvable
/// tools, and the set of workflows a `run` can resolve (workflow-dsl.md §6.6).
struct Ctx<'a> {
    registry: &'a Registry,
    tools: &'a [ToolDef],
    workflows: &'a HashMap<String, Workflow>,
}

/// Type-check a workflow against the registry, the available tools, and the
/// workflows a `run` may invoke, returning the inferred type of every variable
/// on success.
///
/// `tools` is the set of tool definitions accessible from this workflow —
/// every `run tool` expression must name a tool present here, and all such tools
/// must declare an `# output:` type (workflow-dsl.md §3). `workflows` maps each
/// name a `run` may reference (the transitive closure) to its parsed AST; an
/// empty map is fine for a workflow that never uses `run`.
pub fn check(
    workflow: &Workflow,
    registry: &Registry,
    tools: &[ToolDef],
    workflows: &HashMap<String, Workflow>,
) -> Result<TypeEnv, TypeError> {
    let ctx = Ctx {
        registry,
        tools,
        workflows,
    };
    let (env, _) = check_workflow(workflow, &ctx, &[])?;
    Ok(env)
}

/// Type-check one workflow, returning its variable environment and the type of
/// its result — the last statement's value, or `None` when it has no statements
/// (so a `run` of it has no value to yield). `chain` is the stack of
/// `run`-nested workflow names currently being checked, used to reject cycles.
fn check_workflow(
    workflow: &Workflow,
    ctx: &Ctx,
    chain: &[String],
) -> Result<(TypeEnv, Option<Type>), TypeError> {
    let mut env = TypeEnv::new();
    // Declared inputs are in scope as typed variables from the first statement
    // (workflow-dsl.md §5.1).
    for input in &workflow.inputs {
        env.insert(input.name.clone(), input.ty.clone());
    }
    let mut output = None;
    for stmt in &workflow.statements {
        let ty = infer(&stmt.expr, &env, ctx, chain)?;
        output = Some(ty.clone());
        env.insert(stmt.name.clone(), ty);
    }
    Ok((env, output))
}

fn infer(expr: &Expr, env: &TypeEnv, ctx: &Ctx, chain: &[String]) -> Result<Type, TypeError> {
    match expr {
        Expr::Str(_) => Ok(Type::String),
        Expr::Num(_) => Ok(Type::Number),
        Expr::Bool(_) => Ok(Type::Bool),

        Expr::Object(fields) => {
            let typed: Result<Vec<(String, Type)>, TypeError> = fields
                .iter()
                .map(|(k, e)| Ok((k.clone(), infer(e, env, ctx, chain)?)))
                .collect();
            Ok(Type::Object(typed?))
        }

        Expr::Var(name) => env
            .get(name)
            .cloned()
            .ok_or_else(|| TypeError(format!("undefined variable `{name}`"))),

        Expr::Not(inner) => {
            let ty = infer(inner, env, ctx, chain)?;
            if ty != Type::Bool {
                return Err(TypeError(format!(
                    "operator `!` requires bool, found `{ty}`"
                )));
            }
            Ok(Type::Bool)
        }

        Expr::BinOp { op, lhs, rhs } => {
            let l = infer(lhs, env, ctx, chain)?;
            let r = infer(rhs, env, ctx, chain)?;
            infer_binop(op, l, r)
        }

        Expr::Access { base, keys } => {
            let mut ty = infer(base, env, ctx, chain)?;
            for key in keys {
                ty = apply_key(ty, key, env, ctx, chain)?;
            }
            Ok(ty)
        }

        Expr::If { branches, default } => {
            let result_ty = infer(default, env, ctx, chain)?;
            for (cond, result) in branches {
                let cond_ty = infer(cond, env, ctx, chain)?;
                if cond_ty != Type::Bool {
                    return Err(TypeError(format!(
                        "if condition must be bool, found `{cond_ty}`"
                    )));
                }
                let branch_ty = infer(result, env, ctx, chain)?;
                if branch_ty != result_ty {
                    return Err(TypeError(format!(
                        "if branches have mismatched types: `{branch_ty}` vs `{result_ty}`"
                    )));
                }
            }
            Ok(result_ty)
        }

        Expr::For { item, array, body } => {
            let arr_ty = infer(array, env, ctx, chain)?;
            let elem_ty = match arr_ty {
                Type::Array(inner) => *inner,
                other => {
                    return Err(TypeError(format!(
                        "for loop requires array, found `{other}`"
                    )))
                }
            };
            let mut inner_env = env.clone();
            inner_env.insert(item.clone(), elem_ty);
            let body_ty = infer(body, &inner_env, ctx, chain)?;
            Ok(Type::Array(Box::new(body_ty)))
        }

        Expr::Do {
            var,
            body,
            cond,
            max,
        } => {
            let body_ty = infer(body, env, ctx, chain)?;
            // The `while` condition sees the loop's running value bound to `var`.
            let mut cond_env = env.clone();
            cond_env.insert(var.clone(), body_ty.clone());
            let cond_ty = infer(cond, &cond_env, ctx, chain)?;
            if cond_ty != Type::Bool {
                return Err(TypeError(format!(
                    "do loop `while` condition must be bool, found `{cond_ty}`"
                )));
            }
            // The cap is a plain count, evaluated in the outer scope (no `var`).
            let max_ty = infer(max, env, ctx, chain)?;
            if max_ty != Type::Number {
                return Err(TypeError(format!(
                    "do loop `max` must be a number, found `{max_ty}`"
                )));
            }
            Ok(body_ty)
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
                .ok_or_else(|| TypeError(format!("unknown tool `{tool}`")))?;

            // Every supplied arg must be a declared param of matching type.
            for (key, val_expr) in args {
                let param = tool_def
                    .params
                    .iter()
                    .find(|p| p.name == *key)
                    .ok_or_else(|| TypeError(format!("tool `{tool}` has no param `{key}`")))?;
                let val_ty = infer(val_expr, env, ctx, chain)?;
                if val_ty != param.ty {
                    return Err(TypeError(format!(
                        "tool `{tool}` param `{key}` expects `{}`, found `{val_ty}`",
                        param.ty
                    )));
                }
            }

            // Every declared param must be supplied.
            for param in &tool_def.params {
                if !args.iter().any(|(k, _)| k == &param.name) {
                    return Err(TypeError(format!(
                        "tool `{tool}` requires param `{}` but it wasn't supplied",
                        param.name
                    )));
                }
            }

            let output_ty = tool_def.output.clone().ok_or_else(|| {
                TypeError(format!(
                    "tool `{tool}` doesn't declare an `# output:` type; \
                     all tools called in a workflow must declare their output type"
                ))
            })?;

            check_recover(
                tool_def,
                recover,
                recover_default,
                &output_ty,
                env,
                ctx,
                chain,
            )?;

            Ok(output_ty)
        }

        Expr::RunSpecialist {
            specialist: name,
            prompt,
        } => {
            let prompt_ty = infer(prompt, env, ctx, chain)?;
            if prompt_ty != Type::String {
                return Err(TypeError(format!(
                    "specialist prompt must be a string, found `{prompt_ty}`"
                )));
            }
            if !ctx.registry.specialists.contains_key(name.as_str()) {
                return Err(TypeError(format!("unknown specialist `{name}`")));
            }
            Ok(specialist_return_type())
        }

        Expr::RunWorkflow {
            workflow: name,
            args,
        } => {
            if chain.iter().any(|n| n == name) {
                return Err(TypeError(format!(
                    "workflow cycle detected: {} -> {name}",
                    chain.join(" -> ")
                )));
            }
            let callee = ctx
                .workflows
                .get(name)
                .ok_or_else(|| TypeError(format!("unknown workflow `{name}`")))?;

            // Every supplied arg must be a declared input of matching type, and
            // every declared input must be supplied — mirroring `run tool`.
            for (key, val_expr) in args {
                let input = callee
                    .inputs
                    .iter()
                    .find(|p| p.name == *key)
                    .ok_or_else(|| TypeError(format!("workflow `{name}` has no input `{key}`")))?;
                let val_ty = infer(val_expr, env, ctx, chain)?;
                if val_ty != input.ty {
                    return Err(TypeError(format!(
                        "workflow `{name}` input `{key}` expects `{}`, found `{val_ty}`",
                        input.ty
                    )));
                }
            }
            for input in &callee.inputs {
                if !args.iter().any(|(k, _)| k == &input.name) {
                    return Err(TypeError(format!(
                        "workflow `{name}` requires input `{}` but it wasn't supplied",
                        input.name
                    )));
                }
            }

            // The call's type is the callee's result type, inferred recursively.
            let mut nested = chain.to_vec();
            nested.push(name.clone());
            let (_, output) = check_workflow(callee, ctx, &nested)?;
            output.ok_or_else(|| {
                TypeError(format!(
                    "workflow `{name}` has no statements, so `run {name}` produces no value"
                ))
            })
        }
    }
}

/// Type-check a `run tool` `else` recovery block (workflow-dsl.md §6.6/§7).
///
/// When a block is present, every named arm must match a non-success `# exits:`
/// code the tool declares, no arm may repeat, every arm's value (including the
/// `_` default) must have the tool's `# output:` type, and the block must be
/// exhaustive — every declared non-zero exit is covered by name, or a `_`
/// default catches the rest. A tool with no `else` block simply aborts on
/// failure, so nothing is checked there.
fn check_recover(
    tool_def: &ToolDef,
    recover: &[(String, Expr)],
    recover_default: &Option<Box<Expr>>,
    output_ty: &Type,
    env: &TypeEnv,
    ctx: &Ctx,
    chain: &[String],
) -> Result<(), TypeError> {
    let tool = &tool_def.name;
    if recover.is_empty() && recover_default.is_none() {
        return Ok(());
    }

    let mut seen: Vec<&str> = Vec::new();
    for (name, arm) in recover {
        if seen.contains(&name.as_str()) {
            return Err(TypeError(format!(
                "tool `{tool}` has a duplicate `else` arm `{name}`"
            )));
        }
        seen.push(name);

        match tool_def.exits.iter().find(|e| e.name == *name) {
            None => {
                return Err(TypeError(format!(
                    "tool `{tool}` declares no exit code named `{name}`"
                )))
            }
            Some(exit) if exit.code == 0 => {
                return Err(TypeError(format!(
                    "exit code `{name}` of tool `{tool}` is a success code (0); \
                     `else` arms handle failures"
                )))
            }
            Some(_) => {}
        }

        check_recover_arm_type(
            tool,
            &format!("arm `{name}`"),
            arm,
            output_ty,
            env,
            ctx,
            chain,
        )?;
    }

    if let Some(default) = recover_default {
        check_recover_arm_type(tool, "`_` default", default, output_ty, env, ctx, chain)?;
    }

    // Exhaustiveness: without a `_` default, every non-zero declared exit must
    // have its own arm.
    if recover_default.is_none() {
        let missing: Vec<&str> = tool_def
            .exits
            .iter()
            .filter(|e| e.code != 0 && !seen.contains(&e.name.as_str()))
            .map(|e| e.name.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(TypeError(format!(
                "tool `{tool}` `else` block doesn't handle exit code(s) {}; \
                 add an arm for each or a `_` default",
                missing.join(", ")
            )));
        }
    }

    Ok(())
}

/// One `else` arm's value must have the tool's `# output:` type.
fn check_recover_arm_type(
    tool: &str,
    label: &str,
    arm: &Expr,
    output_ty: &Type,
    env: &TypeEnv,
    ctx: &Ctx,
    chain: &[String],
) -> Result<(), TypeError> {
    let arm_ty = infer(arm, env, ctx, chain)?;
    if arm_ty != *output_ty {
        return Err(TypeError(format!(
            "tool `{tool}` `else` {label} has type `{arm_ty}`, \
             but the tool's output type is `{output_ty}`"
        )));
    }
    Ok(())
}

fn infer_binop(op: &BinOp, l: Type, r: Type) -> Result<Type, TypeError> {
    match op {
        BinOp::Or | BinOp::And => {
            if l != Type::Bool || r != Type::Bool {
                return Err(TypeError(format!(
                    "operator `{op:?}` requires bool operands, found `{l}` and `{r}`"
                )));
            }
            Ok(Type::Bool)
        }
        BinOp::Add => {
            if l == Type::String && r == Type::String {
                Ok(Type::String)
            } else if l == Type::Number && r == Type::Number {
                Ok(Type::Number)
            } else {
                Err(TypeError(format!(
                    "operator `+` requires two strings or two numbers, found `{l}` and `{r}`"
                )))
            }
        }
        BinOp::Eq | BinOp::Ne => {
            if l != r {
                return Err(TypeError(format!(
                    "operator `{op:?}` requires operands of the same type, found `{l}` and `{r}`"
                )));
            }
            Ok(Type::Bool)
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let both = |t| l == t && r == t;
            if both(Type::Number) || both(Type::String) {
                Ok(Type::Bool)
            } else {
                Err(TypeError(format!(
                    "comparison operator `{op:?}` requires two numbers or two strings, \
                     found `{l}` and `{r}`"
                )))
            }
        }
        _ => {
            if l != Type::Number || r != Type::Number {
                return Err(TypeError(format!(
                    "arithmetic operator `{op:?}` requires number operands, \
                     found `{l}` and `{r}`"
                )));
            }
            Ok(Type::Number)
        }
    }
}

fn apply_key(
    ty: Type,
    key: &AccessKey,
    env: &TypeEnv,
    ctx: &Ctx,
    chain: &[String],
) -> Result<Type, TypeError> {
    match key {
        AccessKey::Ident(k) | AccessKey::Quoted(k) => match ty {
            Type::Object(fields) => fields
                .into_iter()
                .find(|(fk, _)| fk == k)
                .map(|(_, ft)| ft)
                .ok_or_else(|| TypeError(format!("object type has no field `{k}`"))),
            other => Err(TypeError(format!(
                "cannot access field `{k}` on type `{other}`"
            ))),
        },
        AccessKey::Index(_) => match ty {
            Type::Array(inner) => Ok(*inner),
            other => Err(TypeError(format!(
                "integer index requires array, found `{other}`"
            ))),
        },
        AccessKey::Computed(expr) => match ty {
            Type::Array(inner) => {
                infer(expr, env, ctx, chain)?;
                Ok(*inner)
            }
            other => Err(TypeError(format!(
                "computed access `.(expr)` requires array, found `{other}` \
                 (computed access into objects is deferred to v2)"
            ))),
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
