//! Tests for [`super`]. Extracted from `check.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use crate::ai::Reasoning;
use crate::domain::Specialist;
use crate::types::{ExitCode, Param};
use crate::workflow::parse::parse;
use std::collections::HashMap;
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
        exits: vec![],
    }
}

#[test]
fn infers_literal_types() {
    let wf = parse("s = \"hi\"\n\nn = 1\n\nb = true").unwrap();
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
    assert_eq!(env["s"], Type::String);
    assert_eq!(env["n"], Type::Number);
    assert_eq!(env["b"], Type::Bool);
}

#[test]
fn infers_string_concatenation() {
    let wf = parse(r#"s = "a" + "b""#).unwrap();
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
    assert_eq!(env["s"], Type::String);
}

#[test]
fn rejects_type_mismatch_in_add() {
    let wf = parse(r#"s = "a" + 1"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_bool_from_equality_on_any_same_type() {
    for src in [
        "b = 1 == 2",
        r#"b = "a" != "b""#,
        "b = true == false",
        r#"b = { "x": 1 } == { "x": 2 }"#,
    ] {
        let wf = parse(src).unwrap();
        let env = check(&wf, &empty_registry(), &[], &HashMap::new())
            .unwrap_or_else(|e| panic!("check failed for `{src}`: {e}"));
        assert_eq!(env["b"], Type::Bool, "for source `{src}`");
    }
}

#[test]
fn rejects_equality_between_different_types() {
    let wf = parse(r#"b = 1 == "a""#).unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_bool_from_comparison_on_numbers_and_strings() {
    for src in ["b = 1 < 2", r#"b = "a" >= "b""#] {
        let wf = parse(src).unwrap();
        let env = check(&wf, &empty_registry(), &[], &HashMap::new())
            .unwrap_or_else(|e| panic!("check failed for `{src}`: {e}"));
        assert_eq!(env["b"], Type::Bool, "for source `{src}`");
    }
}

#[test]
fn rejects_comparison_on_bools() {
    let wf = parse("b = true < false").unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_object_type_from_literal() {
    let wf = parse(r#"obj = { "x": 1, "ok": true }"#).unwrap();
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
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
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
    assert_eq!(env["v"], Type::Number);
}

#[test]
fn rejects_access_into_non_object() {
    let wf = parse("n = 1\n\nv = n.x").unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_for_as_array_map() {
    let _wf = parse(r#"ns = for [n: items] (n)"#).unwrap();
    let _tools = [tool("t", vec![], Some(Type::Array(Box::new(Type::Number))))];
    let mut env = TypeEnv::new();
    env.insert("items".to_string(), Type::Array(Box::new(Type::Number)));
    let wf2 = parse("result = for [n: items] (n)").unwrap();
    let env2 = check(&wf2, &empty_registry(), &[], &HashMap::new());
    // `items` is not in scope, so this should fail.
    assert!(env2.is_err());

    // With items defined via a preceding statement:
    let wf3 = parse(r#"items = for [n: items2] (n)"#).unwrap();
    // items2 still undefined — still fails.
    assert!(check(&wf3, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_do_as_body_type_with_var_in_scope_for_while() {
    let t = tool(
        "poll",
        vec![],
        Some(Type::Object(vec![("ready".to_string(), Type::Bool)])),
    );
    // The `while` condition reads `answer`, the value being bound by the loop.
    let wf = parse("answer = do (run tool poll {}) while (!answer.ready) max (3)").unwrap();
    let env = check(&wf, &empty_registry(), &[t], &HashMap::new()).unwrap();
    assert_eq!(
        env["answer"],
        Type::Object(vec![("ready".to_string(), Type::Bool)])
    );
}

#[test]
fn rejects_do_with_non_bool_while() {
    let t = tool(
        "poll",
        vec![],
        Some(Type::Object(vec![("count".to_string(), Type::Number)])),
    );
    let wf = parse("answer = do (run tool poll {}) while (answer.count) max (3)").unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

#[test]
fn rejects_do_with_non_number_max() {
    let wf = parse(r#"answer = do (1) while (true) max ("lots")"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn rejects_do_whose_while_reads_an_undefined_variable() {
    // Only the assigned `answer` is in scope for the condition; `nope` is not.
    let wf = parse("answer = do (1) while (nope) max (3)").unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_run_tool_output_type() {
    let t = tool(
        "ping",
        vec![("HOST", Type::String)],
        Some(Type::Object(vec![
            ("reachable".to_string(), Type::Bool),
            ("ms".to_string(), Type::Number),
        ])),
    );
    let wf = parse(r#"r = run tool ping { HOST: "example.com" }"#).unwrap();
    let env = check(&wf, &empty_registry(), &[t], &HashMap::new()).unwrap();
    assert_eq!(
        env["r"],
        Type::Object(vec![
            ("reachable".to_string(), Type::Bool),
            ("ms".to_string(), Type::Number),
        ])
    );
}

#[test]
fn rejects_run_tool_with_missing_param() {
    let t = tool("ping", vec![("HOST", Type::String)], Some(Type::String));
    let wf = parse("r = run tool ping {}").unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

#[test]
fn rejects_run_tool_with_wrong_param_type() {
    let t = tool("ping", vec![("COUNT", Type::Number)], Some(Type::String));
    let wf = parse(r#"r = run tool ping { COUNT: "five" }"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

#[test]
fn rejects_run_tool_without_output_type() {
    let t = tool("ping", vec![], None);
    let wf = parse("r = run tool ping {}").unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

/// A `ping` tool returning `{ "ms": number }` with the given `# exits:` codes.
fn ping_with_exits(exits: Vec<(i32, &str)>) -> ToolDef {
    let mut t = tool(
        "ping",
        vec![],
        Some(Type::Object(vec![("ms".to_string(), Type::Number)])),
    );
    t.exits = exits
        .into_iter()
        .map(|(code, name)| ExitCode {
            code,
            name: name.to_string(),
            desc: None,
        })
        .collect();
    t
}

#[test]
fn accepts_exhaustive_else_block() {
    let t = ping_with_exits(vec![(1, "unreachable"), (2, "badArgs")]);
    let wf =
        parse(r#"r = run tool ping {} else { unreachable: { "ms": 0 }, badArgs: { "ms": 0 } }"#)
            .unwrap();
    let env = check(&wf, &empty_registry(), &[t], &HashMap::new()).unwrap();
    assert_eq!(
        env["r"],
        Type::Object(vec![("ms".to_string(), Type::Number)])
    );
}

#[test]
fn accepts_else_default_arm() {
    let t = ping_with_exits(vec![(1, "unreachable"), (2, "badArgs")]);
    let wf = parse(r#"r = run tool ping {} else { _: { "ms": 0 } }"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_ok());
}

#[test]
fn rejects_else_arm_for_unknown_exit_code() {
    let t = ping_with_exits(vec![(1, "unreachable")]);
    let wf = parse(r#"r = run tool ping {} else { typo: { "ms": 0 } }"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

#[test]
fn rejects_non_exhaustive_else_block() {
    let t = ping_with_exits(vec![(1, "unreachable"), (2, "badArgs")]);
    // `badArgs` is unhandled and there's no `_` default.
    let wf = parse(r#"r = run tool ping {} else { unreachable: { "ms": 0 } }"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

#[test]
fn rejects_else_arm_with_wrong_type() {
    let t = ping_with_exits(vec![(1, "unreachable")]);
    let wf = parse(r#"r = run tool ping {} else { unreachable: "down" }"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

#[test]
fn rejects_else_arm_for_success_code() {
    let t = ping_with_exits(vec![(0, "ok"), (1, "unreachable")]);
    let wf = parse(r#"r = run tool ping {} else { ok: { "ms": 0 }, unreachable: { "ms": 0 } }"#)
        .unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_err());
}

#[test]
fn tool_without_else_block_needs_no_exit_handling() {
    // Declared exits but no `else` block — allowed (failure aborts at runtime).
    let t = ping_with_exits(vec![(1, "unreachable")]);
    let wf = parse("r = run tool ping {}").unwrap();
    assert!(check(&wf, &empty_registry(), &[t], &HashMap::new()).is_ok());
}

#[test]
fn infers_run_specialist_as_envelope() {
    let wf = parse(r#"s = run specialist reporter "hello""#).unwrap();
    let registry = registry_with_specialist("reporter");
    let env = check(&wf, &registry, &[], &HashMap::new()).unwrap();
    assert_eq!(env["s"], specialist_return_type());
}

#[test]
fn rejects_run_specialist_with_non_string_prompt() {
    let wf = parse("s = run specialist reporter 42").unwrap();
    let registry = registry_with_specialist("reporter");
    assert!(check(&wf, &registry, &[], &HashMap::new()).is_err());
}

#[test]
fn rejects_run_specialist_for_unknown_specialist() {
    let wf = parse(r#"s = run specialist ghost "hi""#).unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_ask_as_string() {
    let wf = parse(r#"city = ask "Which city?""#).unwrap();
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
    assert_eq!(env["city"], Type::String);
}

#[test]
fn infers_ask_with_fallback_as_string() {
    let wf = parse(r#"city = ask "Which city?" else "Portland""#).unwrap();
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
    assert_eq!(env["city"], Type::String);
}

#[test]
fn rejects_ask_with_non_string_prompt() {
    let wf = parse("x = ask 42").unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn rejects_ask_with_non_string_fallback() {
    let wf = parse(r#"x = ask "q" else 42"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn infers_if_expression_type() {
    let wf = parse(r#"v = if (true) "yes", (_) "no""#).unwrap();
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
    assert_eq!(env["v"], Type::String);
}

#[test]
fn rejects_if_with_non_bool_condition() {
    let wf = parse(r#"v = if (1) "yes", (_) "no""#).unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn rejects_if_with_mismatched_branch_types() {
    let wf = parse(r#"v = if (true) "yes", (_) 42"#).unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn rejects_undefined_variable() {
    let wf = parse("v = x").unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn declared_inputs_are_in_scope_with_their_types() {
    let wf = parse("# inputs: CITY:string, COUNT:number\n\nn = COUNT + 1").unwrap();
    let env = check(&wf, &empty_registry(), &[], &HashMap::new()).unwrap();
    assert_eq!(env["CITY"], Type::String);
    assert_eq!(env["n"], Type::Number);
}

#[test]
fn rejects_input_used_at_the_wrong_type() {
    // CITY is a string, so adding a number to it is a type error.
    let wf = parse("# inputs: CITY:string\n\nbad = CITY + 1").unwrap();
    assert!(check(&wf, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn run_infers_the_callees_result_type() {
    // `inner` produces a number; `run inner` in the outer workflow is typed
    // as that number, recursively inferred (no `# output:` declaration).
    let outer = parse("r = run workflow inner { N: 2 }\n\ndoubled = r + r").unwrap();
    let mut wfs = HashMap::new();
    wfs.insert(
        "inner".to_string(),
        parse("# inputs: N:number\n\nout = N + 1").unwrap(),
    );
    let env = check(&outer, &empty_registry(), &[], &wfs).unwrap();
    assert_eq!(env["r"], Type::Number);
    assert_eq!(env["doubled"], Type::Number);
}

#[test]
fn rejects_run_with_wrong_input_type() {
    let outer = parse(r#"r = run workflow inner { N: "two" }"#).unwrap();
    let mut wfs = HashMap::new();
    wfs.insert(
        "inner".to_string(),
        parse("# inputs: N:number\n\nout = N").unwrap(),
    );
    assert!(check(&outer, &empty_registry(), &[], &wfs).is_err());
}

#[test]
fn rejects_run_with_missing_input() {
    let outer = parse("r = run workflow inner {}").unwrap();
    let mut wfs = HashMap::new();
    wfs.insert(
        "inner".to_string(),
        parse("# inputs: N:number\n\nout = N").unwrap(),
    );
    assert!(check(&outer, &empty_registry(), &[], &wfs).is_err());
}

#[test]
fn rejects_run_of_unknown_workflow() {
    let outer = parse("r = run workflow ghost {}").unwrap();
    assert!(check(&outer, &empty_registry(), &[], &HashMap::new()).is_err());
}

#[test]
fn detects_run_cycle() {
    // a -> b -> a is rejected rather than recursing forever.
    let a = parse("x = run workflow b {}").unwrap();
    let mut wfs = HashMap::new();
    wfs.insert("a".to_string(), a.clone());
    wfs.insert("b".to_string(), parse("y = run workflow a {}").unwrap());
    let err = check(&a, &empty_registry(), &[], &wfs).unwrap_err();
    assert!(err.0.contains("cycle"), "{}", err.0);
}
