//! Tests for [`super`]. Extracted from `eval.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use crate::workflow::parse::parse;

async fn eval_src(src: &str) -> Result<serde_json::Value, WorkflowError> {
    let wf = parse(src).expect("parse failed");
    let registry = Registry::default();
    let client = crate::ai::Client::new();
    let keys = HashMap::new();
    let inputs = HashMap::new();
    let workflows = HashMap::new();
    eval(&wf, &registry, &[], &client, &keys, &inputs, &workflows).await
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
    let workflows = HashMap::new();
    let v = eval(&wf, &registry, &[], &client, &keys, &inputs, &workflows)
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
    let workflows = HashMap::new();
    let ctx = EvalCtx {
        registry: &registry,
        tools: &[],
        client: &client,
        keys: &keys,
        workflows: &workflows,
    };
    let val = eval_access(arr, &AccessKey::Index(1), Env::new(), &ctx, Vec::new())
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
    let workflows = HashMap::new();
    let ctx = EvalCtx {
        registry: &registry,
        tools: &[],
        client: &client,
        keys: &keys,
        workflows: &workflows,
    };

    let mut last = serde_json::Value::Null;
    for stmt in &wf.statements {
        let val = eval_expr(&stmt.expr, env.clone(), &ctx, Vec::new())
            .await
            .unwrap();
        last = val.clone();
        env.insert(stmt.name.clone(), val);
    }
    assert_eq!(last, serde_json::json!([1, 2, 3]));
}

#[tokio::test]
async fn do_runs_once_then_stops_when_while_is_false() {
    // The body runs at least once; the condition reads the bound value (`answer`)
    // and is false here, so the loop stops and yields the body value.
    let v = eval_src("answer = do (false) while (answer) max (5)")
        .await
        .unwrap();
    assert_eq!(v, serde_json::json!(false));
}

#[tokio::test]
async fn do_rejects_max_below_one() {
    let err = eval_src("answer = do (true) while (false) max (0)")
        .await
        .unwrap_err();
    assert!(err.0.contains("at least 1"));
}

/// A `poll` tool whose script bumps a persisted counter on each call and reports
/// `{ "done": <count >= 3>, "count": <count> }`, so a `do` loop's progress comes
/// from the tool's side effect rather than from an accumulator. Returns the tool
/// and the temp paths (script, counter) for cleanup.
fn polling_tool() -> (ToolDef, std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let nonce = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let counter_path = std::env::temp_dir().join(format!("sp_wf_do_cnt_{nonce}"));
    let script_path = std::env::temp_dir().join(format!("sp_wf_do_{nonce}.sh"));
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nn=$(cat {cnt} 2>/dev/null || echo 0)\nn=$((n+1))\necho $n > {cnt}\n\
             if [ $n -ge 3 ]; then d=true; else d=false; fi\n\
             printf '{{\"done\":%s,\"count\":%s}}' $d $n > \"$SP_OUTPUT_PATH\"\n",
            cnt = counter_path.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let tool = ToolDef {
        name: "poll".to_string(),
        script: script_path.clone(),
        description: String::new(),
        params: vec![],
        output: Some(crate::types::Type::Object(vec![
            ("done".to_string(), crate::types::Type::Bool),
            ("count".to_string(), crate::types::Type::Number),
        ])),
        exits: vec![],
    };
    (tool, script_path, counter_path)
}

#[tokio::test]
async fn do_loops_until_while_reads_done_via_assigned_var() {
    let (tool, script_path, counter_path) = polling_tool();
    // `while (!result.done)` inspects the bound value; the cap is high enough that
    // the tool's own `done` flag is what stops the loop, at the 3rd call.
    let val = eval_with_tool(
        "result = do (run tool poll {}) while (!result.done) max (10)",
        tool,
    )
    .await
    .unwrap();
    std::fs::remove_file(&script_path).ok();
    std::fs::remove_file(&counter_path).ok();
    assert_eq!(val, serde_json::json!({ "done": true, "count": 3 }));
}

#[tokio::test]
async fn do_caps_iterations_at_max() {
    let (tool, script_path, counter_path) = polling_tool();
    // The cap of 2 interrupts before the tool would report `done` (at 3), so the
    // loop yields the 2nd call's value.
    let val = eval_with_tool(
        "result = do (run tool poll {}) while (!result.done) max (2)",
        tool,
    )
    .await
    .unwrap();
    std::fs::remove_file(&script_path).ok();
    std::fs::remove_file(&counter_path).ok();
    assert_eq!(val, serde_json::json!({ "done": false, "count": 2 }));
}

#[tokio::test]
async fn run_tool_runs_script_and_reads_output() {
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
        exits: vec![],
    };

    let wf = parse(r#"result = run tool greet { NAME: "world" }"#).unwrap();
    let registry = Registry::default();
    let client = crate::ai::Client::new();
    let keys = HashMap::new();

    let inputs = HashMap::new();
    let workflows = HashMap::new();
    let val = eval(
        &wf,
        &registry,
        &[tool_def],
        &client,
        &keys,
        &inputs,
        &workflows,
    )
    .await
    .unwrap();
    std::fs::remove_file(&script_path).ok();

    assert_eq!(val, serde_json::json!({"greeting": "hello world"}));
}

#[tokio::test]
async fn run_tool_errors_when_sp_output_path_omitted() {
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
        exits: vec![],
    };

    let wf = parse("result = run tool silent {}").unwrap();
    let registry = Registry::default();
    let client = crate::ai::Client::new();
    let keys = HashMap::new();

    let inputs = HashMap::new();
    let workflows = HashMap::new();
    let err = eval(
        &wf,
        &registry,
        &[tool_def],
        &client,
        &keys,
        &inputs,
        &workflows,
    )
    .await
    .unwrap_err();
    std::fs::remove_file(&script_path).ok();

    assert!(err.0.contains("SP_OUTPUT_PATH"));
}

/// A `ping` tool whose script exits with `code` (writing no output), declaring
/// `# exits:` `1 unreachable` / `2 badArgs` and output `{ "ms": number }`.
#[cfg(test)]
fn exiting_tool(code: i32) -> (ToolDef, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let script_path = std::env::temp_dir().join(format!(
        "sp_wf_exit_{}_{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&script_path, format!("#!/bin/sh\nexit {code}\n")).unwrap();
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let tool = ToolDef {
        name: "ping".to_string(),
        script: script_path.clone(),
        description: String::new(),
        params: vec![],
        output: Some(crate::types::Type::Object(vec![(
            "ms".to_string(),
            crate::types::Type::Number,
        )])),
        exits: vec![
            crate::types::ExitCode {
                code: 1,
                name: "unreachable".to_string(),
                desc: None,
            },
            crate::types::ExitCode {
                code: 2,
                name: "badArgs".to_string(),
                desc: None,
            },
        ],
    };
    (tool, script_path)
}

#[cfg(test)]
async fn eval_with_tool(src: &str, tool: ToolDef) -> Result<serde_json::Value, WorkflowError> {
    let wf = parse(src).expect("parse failed");
    let registry = Registry::default();
    let client = crate::ai::Client::new();
    let keys = HashMap::new();
    let inputs = HashMap::new();
    let workflows = HashMap::new();
    eval(&wf, &registry, &[tool], &client, &keys, &inputs, &workflows).await
}

#[tokio::test]
async fn run_tool_recovers_via_named_else_arm() {
    let (tool, script_path) = exiting_tool(1); // -> "unreachable"
    let val = eval_with_tool(
        r#"r = run tool ping {} else { unreachable: { "ms": 0 }, badArgs: { "ms": 1 } }"#,
        tool,
    )
    .await
    .unwrap();
    std::fs::remove_file(&script_path).ok();
    assert_eq!(val, serde_json::json!({ "ms": 0.0 }));
}

#[tokio::test]
async fn run_tool_recovers_undeclared_exit_via_default_arm() {
    let (tool, script_path) = exiting_tool(7); // not in `# exits:`
    let val = eval_with_tool(r#"r = run tool ping {} else { _: { "ms": 99 } }"#, tool)
        .await
        .unwrap();
    std::fs::remove_file(&script_path).ok();
    assert_eq!(val, serde_json::json!({ "ms": 99.0 }));
}

#[tokio::test]
async fn run_tool_aborts_on_unhandled_nonzero_exit() {
    let (tool, script_path) = exiting_tool(1);
    let err = eval_with_tool("r = run tool ping {}", tool)
        .await
        .unwrap_err();
    std::fs::remove_file(&script_path).ok();
    assert!(err.0.contains("no `else` arm handles it"));
}

/// A tiny tool that echoes `NAME` into a structured `{ "v": string }`.
#[cfg(test)]
fn echo_tool() -> (ToolDef, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let script_path = std::env::temp_dir().join(format!(
        "sp_wf_echo_{}_{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(
        &script_path,
        "#!/bin/sh\nprintf '{\"v\":\"%s\"}' \"$NAME\" > \"$SP_OUTPUT_PATH\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let tool = ToolDef {
        name: "echo".to_string(),
        script: script_path.clone(),
        description: String::new(),
        params: vec![crate::types::Param {
            name: "NAME".to_string(),
            ty: crate::types::Type::String,
        }],
        output: Some(crate::types::Type::Object(vec![(
            "v".to_string(),
            crate::types::Type::String,
        )])),
        exits: vec![],
    };
    (tool, script_path)
}

#[tokio::test]
async fn run_nests_a_workflow_passing_inputs() {
    // The outer workflow runs `inner`, which calls a tool with the input it
    // was handed, and reads a field off the nested result.
    let (tool, script_path) = echo_tool();
    let inner = parse("# inputs: WHO:string\n\nout = run tool echo { NAME: WHO }").unwrap();
    let outer = parse(r#"r = run workflow inner { WHO: "world" }"#).unwrap();

    let mut workflows = HashMap::new();
    workflows.insert("inner".to_string(), inner);

    let registry = Registry::default();
    let client = crate::ai::Client::new();
    let keys = HashMap::new();
    let inputs = HashMap::new();
    let val = eval(
        &outer,
        &registry,
        &[tool],
        &client,
        &keys,
        &inputs,
        &workflows,
    )
    .await
    .unwrap();
    std::fs::remove_file(&script_path).ok();

    // `run inner` yields inner's last statement — the tool's structured output.
    assert_eq!(val, serde_json::json!({"v": "world"}));
}

#[tokio::test]
async fn run_detects_a_cycle() {
    // `a` runs `b`, `b` runs `a` — evaluation must stop with a cycle error.
    let a = parse("x = run workflow b {}").unwrap();
    let b = parse("y = run workflow a {}").unwrap();
    let mut workflows = HashMap::new();
    workflows.insert("a".to_string(), a.clone());
    workflows.insert("b".to_string(), b);

    let registry = Registry::default();
    let client = crate::ai::Client::new();
    let keys = HashMap::new();
    let inputs = HashMap::new();
    let err = eval(&a, &registry, &[], &client, &keys, &inputs, &workflows)
        .await
        .unwrap_err();
    assert!(err.0.contains("cycle"), "{}", err.0);
}
