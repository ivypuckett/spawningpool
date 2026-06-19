//! Structured workflow logging (docs/workflow-logging.md).
//!
//! The library builds each event as a JSON object carrying `event`, `wf`, and
//! the event-specific fields, and hands it to a [`LogSink`]. The sink — supplied
//! by the front-end, like [`crate::workflow::AskHandler`] — injects the two
//! universal fields it owns (`ts`, stamped at emit time, and `run`, fixed per
//! invocation) and writes one NDJSON line. A no-op sink disables logging.
//!
//! Key order within an event object is not significant: every line is
//! self-contained and `jq`-parseable regardless, so no `serde_json`
//! `preserve_order` feature is needed.

use serde_json::{json, Value};

use crate::ai::StopReason;

/// A structured-log sink: given an event object (with `event`, `wf`, and
/// event-specific fields), record it. Injected by the caller so the library
/// stays decoupled from any destination or clock.
pub type LogSink<'a> = dyn Fn(Value) + 'a;

/// Per-invocation logging context for a specialist run, threaded into
/// [`crate::run::run_specialist`] so its `specialist.*` events and the tool
/// calls it makes are tied to the right workflow and statement. `wf` and `stmt`
/// are `None` for a bare `run specialist`.
pub struct SpecialistLog<'a> {
    pub sink: &'a LogSink<'a>,
    pub wf: Option<&'a str>,
    pub stmt: Option<&'a str>,
}

impl SpecialistLog<'_> {
    pub(crate) fn emit(&self, event: Value) {
        (self.sink)(event);
    }
}

pub(crate) fn workflow_start(wf: &str, inputs: &Value) -> Value {
    json!({ "event": "workflow.start", "wf": wf, "inputs": inputs })
}

pub(crate) fn workflow_done(wf: &str, elapsed_ms: u64) -> Value {
    json!({ "event": "workflow.done", "wf": wf, "elapsed_ms": elapsed_ms })
}

pub(crate) fn workflow_error(wf: &str, elapsed_ms: u64, error: &str) -> Value {
    json!({ "event": "workflow.error", "wf": wf, "elapsed_ms": elapsed_ms, "error": error })
}

pub(crate) fn specialist_start(
    wf: Option<&str>,
    stmt: Option<&str>,
    specialist: &str,
    model: &str,
    prompt: &str,
) -> Value {
    json!({
        "event": "specialist.start",
        "wf": wf,
        "stmt": stmt,
        "specialist": specialist,
        "model": model,
        "prompt": prompt,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn specialist_done(
    wf: Option<&str>,
    stmt: Option<&str>,
    specialist: &str,
    stop_reason: StopReason,
    turns: u32,
    input_tokens: u32,
    output_tokens: u32,
    elapsed_ms: u64,
) -> Value {
    json!({
        "event": "specialist.done",
        "wf": wf,
        "stmt": stmt,
        "specialist": specialist,
        "stop_reason": stop_reason,
        "turns": turns,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "elapsed_ms": elapsed_ms,
    })
}

pub(crate) fn tool_call(
    wf: Option<&str>,
    stmt: Option<&str>,
    specialist: Option<&str>,
    tool: &str,
    args: &Value,
) -> Value {
    json!({
        "event": "tool.call",
        "wf": wf,
        "stmt": stmt,
        "specialist": specialist,
        "tool": tool,
        "args": args,
    })
}

pub(crate) fn tool_done(
    wf: Option<&str>,
    stmt: Option<&str>,
    specialist: Option<&str>,
    tool: &str,
    success: bool,
    exit_code: Option<i32>,
    elapsed_ms: u64,
) -> Value {
    json!({
        "event": "tool.done",
        "wf": wf,
        "stmt": stmt,
        "specialist": specialist,
        "tool": tool,
        "success": success,
        "exit_code": exit_code,
        "elapsed_ms": elapsed_ms,
    })
}

pub(crate) fn ask_prompt(wf: Option<&str>, stmt: Option<&str>, prompt: &str) -> Value {
    json!({ "event": "ask.prompt", "wf": wf, "stmt": stmt, "prompt": prompt })
}

pub(crate) fn ask_answer(wf: Option<&str>, stmt: Option<&str>, answered: bool) -> Value {
    json!({ "event": "ask.answer", "wf": wf, "stmt": stmt, "answered": answered })
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
