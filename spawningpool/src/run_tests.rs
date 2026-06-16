//! Tests for [`super`]. Extracted from `run.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use std::path::PathBuf;

fn write_script(body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!(
        "sp_run_tool_{}_{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn tool(name: &str, script: PathBuf) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        script,
        description: String::new(),
        params: vec![],
        output: None,
        exits: vec![],
    }
}

#[test]
fn build_context_carries_system_prompt_user_turn_and_tools() {
    let specialist = Specialist {
        name: "netop".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-opus-4-8".to_string(),
        system_prompt: "You ping hosts.".to_string(),
        tools: vec!["ping".to_string()],
        constraint: None,
        reasoning: crate::ai::Reasoning::Off,
        stream: false,
    };
    let tools = vec![ToolDef {
        name: "ping".to_string(),
        script: PathBuf::from("ping.sh"),
        description: "Ping a host".to_string(),
        params: vec![crate::types::Param {
            name: "host".to_string(),
            ty: crate::types::Type::String,
        }],
        output: None,
        exits: vec![],
    }];

    let ctx = build_context(&specialist, "ping example.com", &tools);
    assert_eq!(ctx.system.as_deref(), Some("You ping hosts."));
    assert_eq!(ctx.tools.len(), 1);
    assert_eq!(ctx.tools[0].name, "ping");
    assert_eq!(
        ctx.messages[0].content,
        vec![ContentBlock::text("ping example.com")]
    );
}

#[test]
fn args_to_vars_stringifies_values_and_ignores_non_objects() {
    let vars = args_to_vars(&serde_json::json!({ "env": "prod", "count": 3 }));
    assert_eq!(vars.get("env"), Some(&"prod".to_string()));
    // Non-string values fall back to their JSON form.
    assert_eq!(vars.get("count"), Some(&"3".to_string()));

    // A non-object (e.g. malformed args) yields no variables.
    assert!(args_to_vars(&serde_json::json!("oops")).is_empty());
}

#[test]
fn args_to_vars_handles_varied_json_value_types() {
    let vars = args_to_vars(&serde_json::json!({
        "s": "txt",
        "n": 1.5,
        "b": true,
        "nil": null,
        "arr": [1, 2],
        "obj": { "k": "v" },
    }));
    assert_eq!(vars.get("s"), Some(&"txt".to_string()));
    assert_eq!(vars.get("n"), Some(&"1.5".to_string()));
    assert_eq!(vars.get("b"), Some(&"true".to_string()));
    assert_eq!(vars.get("nil"), Some(&"null".to_string()));
    assert_eq!(vars.get("arr"), Some(&"[1,2]".to_string()));
    assert_eq!(vars.get("obj"), Some(&r#"{"k":"v"}"#.to_string()));
}

#[test]
fn run_tool_call_returns_result_on_success_and_reports_output() {
    let script = write_script("#!/bin/sh\necho \"hi $NAME\"\n");
    let tools = vec![tool("greet", script.clone())];
    let mut events = Vec::new();
    let block = run_tool_call(
        &tools,
        "id1",
        "greet",
        &serde_json::json!({ "NAME": "world" }),
        &mut |e| events.push(format!("{e:?}")),
    );
    std::fs::remove_file(&script).ok();
    match block {
        ContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_call_id, "id1");
            assert!(!is_error);
            assert!(content.contains("hi world"));
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
    // The successful run is reported to the observer.
    assert!(events
        .iter()
        .any(|e| e.contains("ToolRan") && e.contains("hi world")));
}

#[test]
fn run_tool_call_returns_error_on_nonzero_exit() {
    let script = write_script("#!/bin/sh\necho boom >&2\nexit 1\n");
    let tools = vec![tool("fail", script.clone())];
    let block = run_tool_call(&tools, "id2", "fail", &serde_json::json!({}), &mut |_| {});
    std::fs::remove_file(&script).ok();
    match block {
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            assert!(is_error);
            assert!(content.contains("boom"));
        }
        other => panic!("expected ToolResult error, got {other:?}"),
    }
}

#[test]
fn run_tool_call_reports_unknown_tool_as_error() {
    let mut failed = false;
    let block = run_tool_call(&[], "id3", "ghost", &serde_json::json!({}), &mut |e| {
        if matches!(e, RunEvent::ToolFailed { .. }) {
            failed = true;
        }
    });
    match block {
        ContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => {
            assert_eq!(tool_call_id, "id3");
            assert!(is_error);
            assert!(content.contains("unknown tool"));
        }
        other => panic!("expected ToolResult error, got {other:?}"),
    }
    assert!(failed);
}

#[test]
fn run_tool_call_enriches_launch_failure_with_remediation() {
    // A tool whose script can't be launched surfaces a fix, not a raw OS error.
    let tools = vec![tool(
        "ghost_script",
        PathBuf::from("/nonexistent/sp_tool_does_not_exist.sh"),
    )];
    let block = run_tool_call(
        &tools,
        "id",
        "ghost_script",
        &serde_json::json!({}),
        &mut |_| {},
    );
    match block {
        ContentBlock::ToolResult {
            content, is_error, ..
        } => {
            assert!(is_error);
            assert!(content.contains("could not run its script"));
            assert!(content.contains("chmod +x"));
        }
        other => panic!("expected ToolResult error, got {other:?}"),
    }
}
