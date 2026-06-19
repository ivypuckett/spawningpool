//! Running a specialist: the agentic loop that drives a [`Specialist`] against a
//! prompt, executing its tools and feeding results back until it settles.
//!
//! The loop is front-end agnostic. It reports progress — assistant text, token
//! usage, tool outcomes — through a [`RunEvent`] observer the caller supplies,
//! instead of writing to stdout itself, so the CLI and any other front-end can
//! render the same run however they like.

use std::collections::HashMap;

use futures::StreamExt;

use crate::ai::{
    Client, CompleteOptions, ContentBlock, Context, Message, Model, Role, StopReason, StreamEvent,
    Usage,
};
use crate::domain::{Registry, Specialist, ToolDef};
use crate::log::{self, SpecialistLog};

/// Cap on agentic turns, so a specialist that keeps calling tools without ever
/// settling on an answer terminates instead of looping forever.
const MAX_TURNS: usize = 16;

/// Progress reported by [`run_specialist`] as a run unfolds. Borrowed data is
/// only valid for the duration of the observer call.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent<'a> {
    /// A chunk of assistant text as it streams (streaming specialists only).
    TextDelta(&'a str),
    /// A complete assistant text block (non-streaming specialists).
    Text(&'a str),
    /// A chunk of thinking text as it streams (streaming + reasoning enabled).
    ThinkingDelta(&'a str),
    /// A complete thinking block (non-streaming + reasoning enabled).
    Thinking(&'a str),
    /// The stop reason for the turn that just completed, emitted before [`Usage`].
    TurnDone { stop_reason: StopReason },
    /// Token usage reported for the turn that just completed.
    Usage(Usage),
    /// A tool ran (possibly exiting non-zero); carries its combined output.
    ToolRan {
        name: &'a str,
        output: &'a str,
        success: bool,
    },
    /// A tool couldn't be executed — it's unknown, or its script failed to
    /// launch. `message` is the error fed back to the model.
    ToolFailed { name: &'a str, message: &'a str },
}

/// Run `specialist` against `prompt`, executing its tools and looping until it
/// stops calling them (or, for a constrained specialist, after its single forced
/// call). `tools` is the specialist's resolved tools (the caller reads them from
/// the [`crate::tools`] folder); only these are exposed to the model, so they're
/// also the only tools its calls can name. Progress is reported through
/// `observer`; `opts` carries the request options, including any API key the
/// caller has sourced.
///
/// `log`, when `Some`, brackets the invocation with `specialist.start` /
/// `specialist.done` structured events and records each tool the specialist
/// calls (docs/workflow-logging.md); `None` disables that layer.
#[allow(clippy::too_many_arguments)]
pub async fn run_specialist(
    client: &Client,
    registry: &Registry,
    specialist: &Specialist,
    prompt: &str,
    tools: &[ToolDef],
    opts: &CompleteOptions,
    observer: &mut dyn FnMut(RunEvent<'_>),
    log: Option<&SpecialistLog<'_>>,
) -> Result<(), String> {
    let model = registry.resolve_model(specialist)?;
    let mut ctx = build_context(specialist, prompt, tools);
    // A constrained specialist makes a single forced call; a tools specialist
    // runs agentically until it stops calling tools.
    let agentic = specialist.constraint.is_none();
    // Constrained decoding returns the tool's arguments as a single block of
    // JSON text that the adapter rewrites into a tool call; there's nothing
    // meaningful to stream, and the stream path can't do that rewrite, so force
    // a non-streaming turn when it's in play.
    let stream = specialist.stream && !opts.constrained_decoding;

    if let Some(log) = log {
        log.emit(log::specialist_start(
            log.wf,
            log.stmt,
            &specialist.name,
            &specialist.model,
            prompt,
        ));
    }
    // Aggregate metadata for `specialist.done`: an invocation is atomic from the
    // workflow's view, so per-turn token and stop-reason data is summed here.
    let started = std::time::Instant::now();
    let mut turns = 0u32;
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut last_stop = StopReason::Stop;

    let mut settled = false;
    for _ in 0..MAX_TURNS {
        let (message, usage, stop_reason) =
            one_turn(client, &model, &ctx, opts, stream, observer).await?;
        turns += 1;
        input_tokens += usage.input;
        output_tokens += usage.output;
        last_stop = stop_reason;
        observer(RunEvent::TurnDone { stop_reason });
        observer(RunEvent::Usage(usage));

        let calls: Vec<(String, String, serde_json::Value)> = message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => Some((id.clone(), name.clone(), arguments.clone())),
                _ => None,
            })
            .collect();

        // No tool calls means the model produced its final answer.
        if calls.is_empty() {
            settled = true;
            break;
        }

        let mut results = Vec::with_capacity(calls.len());
        for (id, tool_name, arguments) in &calls {
            results.push(run_tool_call(
                tools,
                id,
                tool_name,
                arguments,
                observer,
                log,
                &specialist.name,
            ));
        }

        // The constraint guaranteed exactly one call; once executed, we're done.
        if !agentic {
            settled = true;
            break;
        }

        // Feed the assistant's turn and the tool results back, then loop.
        ctx.messages.push(message);
        ctx.messages.push(Message {
            role: Role::User,
            content: results,
        });
    }

    if settled {
        if let Some(log) = log {
            log.emit(log::specialist_done(
                log.wf,
                log.stmt,
                &specialist.name,
                last_stop,
                turns,
                input_tokens,
                output_tokens,
                started.elapsed().as_millis() as u64,
            ));
        }
        return Ok(());
    }

    Err(format!(
        "specialist '{}' did not finish within {MAX_TURNS} turns.\n  \
         It kept calling tools without settling on an answer — inspect the tool \
         outputs above, tighten its system prompt, or reduce the tools it can call.",
        specialist.name
    ))
}

/// Run one model turn, reporting any assistant text and thinking (streamed live
/// when the specialist streams), and return the assembled message, usage, and
/// stop reason.
async fn one_turn(
    client: &Client,
    model: &Model,
    ctx: &Context,
    opts: &CompleteOptions,
    stream: bool,
    observer: &mut dyn FnMut(RunEvent<'_>),
) -> Result<(Message, Usage, StopReason), String> {
    if stream {
        let mut events = client
            .stream(model, ctx, opts)
            .await
            .map_err(|e| e.to_string())?;
        while let Some(event) = events.next().await {
            match event.map_err(|e| e.to_string())? {
                StreamEvent::TextDelta { delta, .. } => observer(RunEvent::TextDelta(&delta)),
                StreamEvent::ThinkingDelta { delta, .. } => {
                    observer(RunEvent::ThinkingDelta(&delta))
                }
                StreamEvent::Done {
                    usage,
                    message,
                    stop_reason,
                } => return Ok((message, usage, stop_reason)),
                StreamEvent::ToolCallDelta { .. } => {}
            }
        }
        Err("stream ended without a final event".to_string())
    } else {
        let completion = client
            .complete(model, ctx, opts)
            .await
            .map_err(|e| e.to_string())?;
        for block in &completion.message.content {
            match block {
                ContentBlock::Text { text } => observer(RunEvent::Text(text)),
                ContentBlock::Thinking { thinking } => observer(RunEvent::Thinking(thinking)),
                _ => {}
            }
        }
        Ok((completion.message, completion.usage, completion.stop_reason))
    }
}

/// Assemble the runtime [`Context`] for a turn: the specialist's system prompt,
/// the user's prompt, and its resolved tools lowered into the wire [`Tool`] type.
fn build_context(specialist: &Specialist, prompt: &str, tools: &[ToolDef]) -> Context {
    let mut ctx = Context::new(
        Some(specialist.system_prompt.clone()),
        vec![Message::user(prompt)],
    );
    ctx.tools = tools.iter().map(ToolDef::to_tool).collect();
    ctx
}

/// Execute one tool call by running its backing script, report the outcome
/// through `observer`, and return the [`ContentBlock::ToolResult`] to feed back
/// to the model. A failed or unknown tool becomes a tool error so the model can
/// react. When `log` is `Some`, the attempt is bracketed with `tool.call` /
/// `tool.done` events scoped to the specialist (docs/workflow-logging.md): a
/// launched script reports its exit code, while an unknown tool or a launch
/// failure reports `exit_code: null`.
fn run_tool_call(
    tools: &[ToolDef],
    id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    observer: &mut dyn FnMut(RunEvent<'_>),
    log: Option<&SpecialistLog<'_>>,
    specialist: &str,
) -> ContentBlock {
    if let Some(log) = log {
        log.emit(log::tool_call(
            log.wf,
            log.stmt,
            Some(specialist),
            tool_name,
            arguments,
        ));
    }
    let started = std::time::Instant::now();
    let done = |log: Option<&SpecialistLog<'_>>, success: bool, exit_code: Option<i32>| {
        if let Some(log) = log {
            log.emit(log::tool_done(
                log.wf,
                log.stmt,
                Some(specialist),
                tool_name,
                success,
                exit_code,
                started.elapsed().as_millis() as u64,
            ));
        }
    };

    let tool = match tools.iter().find(|t| t.name == tool_name) {
        Some(tool) => tool,
        None => {
            let message = format!("unknown tool: {tool_name}");
            observer(RunEvent::ToolFailed {
                name: tool_name,
                message: &message,
            });
            done(log, false, None);
            return ContentBlock::tool_error(id, message);
        }
    };

    let vars = args_to_vars(arguments);
    match crate::run_script(&tool.script, &vars) {
        Ok(run) => {
            observer(RunEvent::ToolRan {
                name: tool_name,
                output: &run.output,
                success: run.success,
            });
            done(log, run.success, run.code);
            if run.success {
                ContentBlock::tool_result(id, run.output)
            } else {
                ContentBlock::tool_error(id, run.output)
            }
        }
        Err(e) => {
            let path = tool.script.display();
            let message = format!(
                "tool '{tool_name}' could not run its script {path}: {e}\n  \
                 Ensure it exists, is executable (chmod +x {path}), and has a shebang (e.g. #!/bin/sh)."
            );
            observer(RunEvent::ToolFailed {
                name: tool_name,
                message: &message,
            });
            done(log, false, None);
            ContentBlock::tool_error(id, message)
        }
    }
}

/// Lower a tool call's JSON arguments into the `KEY=value` variables a script
/// expects. Non-string values are stringified via their JSON form.
fn args_to_vars(arguments: &serde_json::Value) -> HashMap<String, String> {
    arguments
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| {
            let value = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (key.clone(), value)
        })
        .collect()
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
