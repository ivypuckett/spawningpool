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

/// Type-check a workflow against the registry and the available tools,
/// returning the inferred type of every variable on success.
///
/// `tools` is the set of tool definitions accessible from this workflow —
/// every `call` expression must name a tool present here. All called tools
/// must declare an `# output:` type (workflow-dsl.md §3).
pub fn check(
    workflow: &Workflow,
    registry: &Registry,
    tools: &[ToolDef],
) -> Result<TypeEnv, TypeError> {
    let mut env = TypeEnv::new();
    for stmt in &workflow.statements {
        let ty = infer(&stmt.expr, &env, registry, tools)?;
        env.insert(stmt.name.clone(), ty);
    }
    Ok(env)
}

fn infer(
    expr: &Expr,
    env: &TypeEnv,
    registry: &Registry,
    tools: &[ToolDef],
) -> Result<Type, TypeError> {
    match expr {
        Expr::Str(_) => Ok(Type::String),
        Expr::Num(_) => Ok(Type::Number),
        Expr::Bool(_) => Ok(Type::Bool),

        Expr::Object(fields) => {
            let typed: Result<Vec<(String, Type)>, TypeError> = fields
                .iter()
                .map(|(k, e)| Ok((k.clone(), infer(e, env, registry, tools)?)))
                .collect();
            Ok(Type::Object(typed?))
        }

        Expr::Var(name) => env
            .get(name)
            .cloned()
            .ok_or_else(|| TypeError(format!("undefined variable `{name}`"))),

        Expr::Not(inner) => {
            let ty = infer(inner, env, registry, tools)?;
            if ty != Type::Bool {
                return Err(TypeError(format!(
                    "operator `!` requires bool, found `{ty}`"
                )));
            }
            Ok(Type::Bool)
        }

        Expr::BinOp { op, lhs, rhs } => {
            let l = infer(lhs, env, registry, tools)?;
            let r = infer(rhs, env, registry, tools)?;
            infer_binop(op, l, r)
        }

        Expr::Access { base, keys } => {
            let mut ty = infer(base, env, registry, tools)?;
            for key in keys {
                ty = apply_key(ty, key, env, registry, tools)?;
            }
            Ok(ty)
        }

        Expr::If { branches, default } => {
            let result_ty = infer(default, env, registry, tools)?;
            for (cond, result) in branches {
                let cond_ty = infer(cond, env, registry, tools)?;
                if cond_ty != Type::Bool {
                    return Err(TypeError(format!(
                        "if condition must be bool, found `{cond_ty}`"
                    )));
                }
                let branch_ty = infer(result, env, registry, tools)?;
                if branch_ty != result_ty {
                    return Err(TypeError(format!(
                        "if branches have mismatched types: `{branch_ty}` vs `{result_ty}`"
                    )));
                }
            }
            Ok(result_ty)
        }

        Expr::For { item, array, body } => {
            let arr_ty = infer(array, env, registry, tools)?;
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
            let body_ty = infer(body, &inner_env, registry, tools)?;
            Ok(Type::Array(Box::new(body_ty)))
        }

        Expr::Call { tool, args } => {
            let tool_def = tools
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
                let val_ty = infer(val_expr, env, registry, tools)?;
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

            tool_def.output.clone().ok_or_else(|| {
                TypeError(format!(
                    "tool `{tool}` doesn't declare an `# output:` type; \
                     all tools called in a workflow must declare their output type"
                ))
            })
        }

        Expr::Ask {
            specialist: name,
            prompt,
        } => {
            let prompt_ty = infer(prompt, env, registry, tools)?;
            if prompt_ty != Type::String {
                return Err(TypeError(format!(
                    "ask prompt must be a string, found `{prompt_ty}`"
                )));
            }
            if !registry.specialists.contains_key(name.as_str()) {
                return Err(TypeError(format!("unknown specialist `{name}`")));
            }
            Ok(specialist_return_type())
        }
    }
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
    registry: &Registry,
    tools: &[ToolDef],
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
                infer(expr, env, registry, tools)?;
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
mod tests {
    use super::*;
    use crate::ai::Reasoning;
    use crate::domain::Specialist;
    use crate::types::Param;
    use crate::workflow::parse::parse;
    use std::path::PathBuf;

    fn empty_registry() -> Registry {
        Registry::default()
    }

    fn registry_with_specialist(name: &str) -> Registry {
        let mut r = Registry::default();
        r.specialists.insert(
            name.to_string(),
            Specialist {
                name: name.to_string(),
                provider: "p".to_string(),
                model: "m".to_string(),
                system_prompt: String::new(),
                tools: vec![],
                constraint: None,
                reasoning: Reasoning::Off,
                stream: false,
            },
        );
        r
    }

    fn tool(name: &str, params: Vec<(&str, Type)>, output: Option<Type>) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            script: PathBuf::from(format!("{name}.sh")),
            description: String::new(),
            params: params
                .into_iter()
                .map(|(n, ty)| Param {
                    name: n.to_string(),
                    ty,
                })
                .collect(),
            output,
        }
    }

    #[test]
    fn infers_literal_types() {
        let wf = parse("s = \"hi\"\n\nn = 1\n\nb = true").unwrap();
        let env = check(&wf, &empty_registry(), &[]).unwrap();
        assert_eq!(env["s"], Type::String);
        assert_eq!(env["n"], Type::Number);
        assert_eq!(env["b"], Type::Bool);
    }

    #[test]
    fn infers_string_concatenation() {
        let wf = parse(r#"s = "a" + "b""#).unwrap();
        let env = check(&wf, &empty_registry(), &[]).unwrap();
        assert_eq!(env["s"], Type::String);
    }

    #[test]
    fn rejects_type_mismatch_in_add() {
        let wf = parse(r#"s = "a" + 1"#).unwrap();
        assert!(check(&wf, &empty_registry(), &[]).is_err());
    }

    #[test]
    fn infers_object_type_from_literal() {
        let wf = parse(r#"obj = { "x": 1, "ok": true }"#).unwrap();
        let env = check(&wf, &empty_registry(), &[]).unwrap();
        assert_eq!(
            env["obj"],
            Type::Object(vec![
                ("x".to_string(), Type::Number),
                ("ok".to_string(), Type::Bool),
            ])
        );
    }

    #[test]
    fn infers_access_into_object() {
        let wf = parse("obj = { \"x\": 1 }\n\nv = obj.x").unwrap();
        let env = check(&wf, &empty_registry(), &[]).unwrap();
        assert_eq!(env["v"], Type::Number);
    }

    #[test]
    fn rejects_access_into_non_object() {
        let wf = parse("n = 1\n\nv = n.x").unwrap();
        assert!(check(&wf, &empty_registry(), &[]).is_err());
    }

    #[test]
    fn infers_for_as_array_map() {
        let _wf = parse(r#"ns = for [n: items] (n)"#).unwrap();
        let _tools = [tool("t", vec![], Some(Type::Array(Box::new(Type::Number))))];
        let mut env = TypeEnv::new();
        env.insert("items".to_string(), Type::Array(Box::new(Type::Number)));
        let wf2 = parse("result = for [n: items] (n)").unwrap();
        let env2 = check(&wf2, &empty_registry(), &[]);
        // `items` is not in scope, so this should fail.
        assert!(env2.is_err());

        // With items defined via a preceding statement:
        let wf3 = parse(r#"items = for [n: items2] (n)"#).unwrap();
        // items2 still undefined — still fails.
        assert!(check(&wf3, &empty_registry(), &[]).is_err());
    }

    #[test]
    fn infers_call_output_type() {
        let t = tool(
            "ping",
            vec![("HOST", Type::String)],
            Some(Type::Object(vec![
                ("reachable".to_string(), Type::Bool),
                ("ms".to_string(), Type::Number),
            ])),
        );
        let wf = parse(r#"r = call ping { HOST: "example.com" }"#).unwrap();
        let env = check(&wf, &empty_registry(), &[t]).unwrap();
        assert_eq!(
            env["r"],
            Type::Object(vec![
                ("reachable".to_string(), Type::Bool),
                ("ms".to_string(), Type::Number),
            ])
        );
    }

    #[test]
    fn rejects_call_with_missing_param() {
        let t = tool("ping", vec![("HOST", Type::String)], Some(Type::String));
        let wf = parse("r = call ping {}").unwrap();
        assert!(check(&wf, &empty_registry(), &[t]).is_err());
    }

    #[test]
    fn rejects_call_with_wrong_param_type() {
        let t = tool("ping", vec![("COUNT", Type::Number)], Some(Type::String));
        let wf = parse(r#"r = call ping { COUNT: "five" }"#).unwrap();
        assert!(check(&wf, &empty_registry(), &[t]).is_err());
    }

    #[test]
    fn rejects_call_to_tool_without_output_type() {
        let t = tool("ping", vec![], None);
        let wf = parse("r = call ping {}").unwrap();
        assert!(check(&wf, &empty_registry(), &[t]).is_err());
    }

    #[test]
    fn infers_ask_as_specialist_envelope() {
        let wf = parse(r#"s = ask reporter "hello""#).unwrap();
        let registry = registry_with_specialist("reporter");
        let env = check(&wf, &registry, &[]).unwrap();
        assert_eq!(env["s"], specialist_return_type());
    }

    #[test]
    fn rejects_ask_with_non_string_prompt() {
        let wf = parse("s = ask reporter 42").unwrap();
        let registry = registry_with_specialist("reporter");
        assert!(check(&wf, &registry, &[]).is_err());
    }

    #[test]
    fn rejects_ask_for_unknown_specialist() {
        let wf = parse(r#"s = ask ghost "hi""#).unwrap();
        assert!(check(&wf, &empty_registry(), &[]).is_err());
    }

    #[test]
    fn infers_if_expression_type() {
        let wf = parse(r#"v = if (true) "yes", (_) "no""#).unwrap();
        let env = check(&wf, &empty_registry(), &[]).unwrap();
        assert_eq!(env["v"], Type::String);
    }

    #[test]
    fn rejects_if_with_non_bool_condition() {
        let wf = parse(r#"v = if (1) "yes", (_) "no""#).unwrap();
        assert!(check(&wf, &empty_registry(), &[]).is_err());
    }

    #[test]
    fn rejects_if_with_mismatched_branch_types() {
        let wf = parse(r#"v = if (true) "yes", (_) 42"#).unwrap();
        assert!(check(&wf, &empty_registry(), &[]).is_err());
    }

    #[test]
    fn rejects_undefined_variable() {
        let wf = parse("v = x").unwrap();
        assert!(check(&wf, &empty_registry(), &[]).is_err());
    }
}
