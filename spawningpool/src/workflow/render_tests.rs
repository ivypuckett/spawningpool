//! Tests for [`super`]. Extracted via `#[path]` so they stay a child module
//! with access to private helpers like `free_vars`.

use super::{free_vars, mermaid};
use crate::workflow::parse;
use std::collections::BTreeSet;

fn vars(src: &str) -> BTreeSet<String> {
    let wf = parse(src).unwrap();
    let mut out = BTreeSet::new();
    free_vars(&wf.statements[0].expr, &mut out);
    out
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn free_vars_collects_references() {
    assert_eq!(vars("y = a + b"), set(&["a", "b"]));
}

#[test]
fn free_vars_excludes_for_binding() {
    // `x` is bound by the loop; only the outer `xs` and `z` are free.
    assert_eq!(vars("y = for [x: xs] (x + z)"), set(&["xs", "z"]));
}

#[test]
fn mermaid_starts_with_flowchart() {
    let wf = parse("a = 1").unwrap();
    assert!(mermaid(&wf).starts_with("flowchart TD\n"));
}

#[test]
fn mermaid_draws_data_flow_edge() {
    // a -> n0, b -> n1, with an edge from a's node to b's node.
    let wf = parse("a = 1\n\nb = a + 1").unwrap();
    let out = mermaid(&wf);
    assert!(out.contains("n0 --> n1"), "{out}");
}

#[test]
fn mermaid_self_reference_points_at_previous_def() {
    // The second `x` reassigns; its edge must come from the first `x` (n0),
    // not from itself (n1).
    let wf = parse("x = 1\n\nx = x + 1").unwrap();
    let out = mermaid(&wf);
    assert!(out.contains("n0 --> n1"), "{out}");
    assert!(!out.contains("n1 --> n1"), "{out}");
}

#[test]
fn mermaid_labels_input_with_type() {
    let wf = parse("# inputs: CITY:string\n\nx = CITY").unwrap();
    let out = mermaid(&wf);
    assert!(out.contains("CITY: string"), "{out}");
    // The input (n0) feeds the statement that reads it (n1).
    assert!(out.contains("n0 --> n1"), "{out}");
}

#[test]
fn mermaid_shapes_a_tool_run() {
    let wf = parse("city = \"x\"\n\nw = run tool t { K: city }").unwrap();
    let out = mermaid(&wf);
    assert!(out.contains("[\"w = run tool t\"]"), "{out}");
}

#[test]
fn mermaid_shapes_a_workflow_run() {
    let wf = parse("env = \"prod\"\n\nr = run workflow deploy { ENV: env }").unwrap();
    let out = mermaid(&wf);
    // A `run workflow` node uses the subroutine shape `[["..."]]`.
    assert!(out.contains("[[\"r = run workflow deploy\"]]"), "{out}");
    // The arg passes `env` in, so its node feeds the workflow run.
    assert!(out.contains("n0 --> n1"), "{out}");
}
