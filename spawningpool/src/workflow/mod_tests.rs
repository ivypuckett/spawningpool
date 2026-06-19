//! Tests for [`super`]. Extracted from `mod.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::{referenced, resolve_inputs, source};
use crate::domain::{Registry, Specialist};
use crate::types::{Param, Type};
use crate::workflow::parse;
use std::collections::HashMap;
use std::path::PathBuf;

fn param(name: &str, ty: Type) -> Param {
    Param {
        name: name.to_string(),
        ty,
    }
}

fn provided(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

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
fn referenced_collects_tool_and_specialist_names() {
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
    let wf = parse("a = run tool fetch {}\n\nb = run specialist writer \"hi\"").unwrap();
    let refs = referenced(&wf, &registry);
    // Direct `run tool` plus the specialist's own tool, and the specialist.
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

#[test]
fn referenced_collects_tool_inside_do_body() {
    let wf = parse("a = do [more] (run tool poll {})").unwrap();
    let refs = referenced(&wf, &Registry::default());
    assert_eq!(refs.tools, ["poll".to_string()].into_iter().collect());
}

#[test]
fn referenced_collects_run_workflow_names() {
    let registry = Registry::default();
    let wf =
        parse("a = run workflow deploy { ENV: \"prod\" }\n\nb = run workflow notify {}").unwrap();
    let refs = referenced(&wf, &registry);
    assert_eq!(
        refs.workflows,
        ["deploy".to_string(), "notify".to_string()]
            .into_iter()
            .collect()
    );
}

#[test]
fn resolve_inputs_coerces_each_declared_type() {
    let inputs = [
        param("CITY", Type::String),
        param("COUNT", Type::Number),
        param("OK", Type::Bool),
        param("TAGS", Type::Array(Box::new(Type::String))),
    ];
    let resolved = resolve_inputs(
        &inputs,
        &provided(&[
            ("CITY", "Portland"),
            ("COUNT", "3"),
            ("OK", "true"),
            ("TAGS", r#"["a","b"]"#),
        ]),
    )
    .unwrap();
    assert_eq!(resolved["CITY"], serde_json::json!("Portland"));
    assert_eq!(resolved["COUNT"], serde_json::json!(3.0));
    assert_eq!(resolved["OK"], serde_json::json!(true));
    assert_eq!(resolved["TAGS"], serde_json::json!(["a", "b"]));
}

#[test]
fn resolve_inputs_errors_on_missing_input() {
    let inputs = [param("CITY", Type::String)];
    let err = resolve_inputs(&inputs, &provided(&[])).unwrap_err();
    assert!(err.contains("missing required input `CITY`"), "{err}");
}

#[test]
fn resolve_inputs_errors_on_unknown_provided_key() {
    let inputs = [param("CITY", Type::String)];
    let err = resolve_inputs(&inputs, &provided(&[("CITY", "x"), ("BOGUS", "y")])).unwrap_err();
    assert!(err.contains("no input `BOGUS`"), "{err}");
}

#[test]
fn resolve_inputs_errors_on_uncoercible_value() {
    let inputs = [param("COUNT", Type::Number)];
    let err = resolve_inputs(&inputs, &provided(&[("COUNT", "lots")])).unwrap_err();
    assert!(err.contains("input `COUNT`"), "{err}");
}
