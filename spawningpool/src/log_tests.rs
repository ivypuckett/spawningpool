//! Tests for [`super`]: each builder produces the documented event shape
//! (docs/workflow-logging.md). The universal `ts` and `run` fields are injected
//! by the sink, not these builders, so they are absent here.

use super::*;
use crate::ai::StopReason;

#[test]
fn workflow_events_carry_wf_and_timing() {
    let start = workflow_start("summarise", &serde_json::json!({ "url": "x" }));
    assert_eq!(start["event"], "workflow.start");
    assert_eq!(start["wf"], "summarise");
    assert_eq!(start["inputs"]["url"], "x");

    let done = workflow_done("summarise", 5045);
    assert_eq!(done["event"], "workflow.done");
    assert_eq!(done["elapsed_ms"], 5045);

    let err = workflow_error("summarise", 12, "boom");
    assert_eq!(err["event"], "workflow.error");
    assert_eq!(err["error"], "boom");
    assert_eq!(err["elapsed_ms"], 12);
}

#[test]
fn specialist_events_carry_context_and_aggregates() {
    let start = specialist_start(
        Some("summarise"),
        Some("summary"),
        "summariser",
        "claude-sonnet-4-6",
        "Summarise this",
    );
    assert_eq!(start["event"], "specialist.start");
    assert_eq!(start["wf"], "summarise");
    assert_eq!(start["stmt"], "summary");
    assert_eq!(start["specialist"], "summariser");
    assert_eq!(start["model"], "claude-sonnet-4-6");
    assert_eq!(start["prompt"], "Summarise this");

    let done = specialist_done(
        Some("summarise"),
        Some("summary"),
        "summariser",
        StopReason::Stop,
        2,
        2104,
        418,
        4190,
    );
    assert_eq!(done["event"], "specialist.done");
    assert_eq!(done["stop_reason"], "stop");
    assert_eq!(done["turns"], 2);
    assert_eq!(done["input_tokens"], 2104);
    assert_eq!(done["output_tokens"], 418);
    assert_eq!(done["elapsed_ms"], 4190);
}

#[test]
fn bare_specialist_events_null_their_workflow_context() {
    let start = specialist_start(None, None, "classifier", "m", "p");
    assert!(start["wf"].is_null());
    assert!(start["stmt"].is_null());
}

#[test]
fn tool_events_distinguish_workflow_and_specialist_calls() {
    // A workflow `call tool`: no specialist.
    let call = tool_call(
        Some("summarise"),
        Some("raw"),
        None,
        "fetch-url",
        &serde_json::json!({ "url": "x" }),
    );
    assert_eq!(call["event"], "tool.call");
    assert!(call["specialist"].is_null());
    assert_eq!(call["tool"], "fetch-url");
    assert_eq!(call["args"]["url"], "x");

    // A tool the specialist chose to call.
    let spec_call = tool_call(
        Some("summarise"),
        Some("summary"),
        Some("summariser"),
        "search",
        &serde_json::json!({ "query": "q" }),
    );
    assert_eq!(spec_call["specialist"], "summariser");

    let done = tool_done(
        Some("summarise"),
        Some("raw"),
        None,
        "fetch-url",
        true,
        Some(0),
        837,
    );
    assert_eq!(done["event"], "tool.done");
    assert_eq!(done["success"], true);
    assert_eq!(done["exit_code"], 0);
    assert_eq!(done["elapsed_ms"], 837);

    // A launch failure (or unknown tool) reports a null exit code.
    let failed = tool_done(None, None, Some("s"), "ghost", false, None, 1);
    assert!(failed["exit_code"].is_null());
    assert_eq!(failed["success"], false);
}

#[test]
fn ask_events_record_the_prompt_and_whether_it_was_answered() {
    let prompt = ask_prompt(Some("wf"), Some("c"), "Which city?");
    assert_eq!(prompt["event"], "ask.prompt");
    assert_eq!(prompt["prompt"], "Which city?");

    assert_eq!(ask_answer(Some("wf"), Some("c"), true)["answered"], true);
    assert_eq!(ask_answer(Some("wf"), Some("c"), false)["answered"], false);
}
