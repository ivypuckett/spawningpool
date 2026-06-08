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
    Client, CompleteOptions, ContentBlock, Context, Message, Model, Role, StreamEvent, Usage,
};
use crate::domain::{Registry, Specialist, ToolDef};

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
pub async fn run_specialist(
    client: &Client,
    registry: &Registry,
    specialist: &Specialist,
    prompt: &str,
    tools: &[ToolDef],
    opts: &CompleteOptions,
    observer: &mut dyn FnMut(RunEvent<'_>),
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

    for _ in 0..MAX_TURNS {
        let (message, usage) = one_turn(client, &model, &ctx, opts, stream, observer).await?;
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
            return Ok(());
        }

        let mut results = Vec::with_capacity(calls.len());
        for (id, tool_name, arguments) in &calls {
            results.push(run_tool_call(tools, id, tool_name, arguments, observer));
        }

        // The constraint guaranteed exactly one call; once executed, we're done.
        if !agentic {
            return Ok(());
        }

        // Feed the assistant's turn and the tool results back, then loop.
        ctx.messages.push(message);
        ctx.messages.push(Message {
            role: Role::User,
            content: results,
        });
    }

    Err(format!(
        "specialist '{}' did not finish within {MAX_TURNS} turns.\n  \
         It kept calling tools without settling on an answer — inspect the tool \
         outputs above, tighten its system prompt, or reduce the tools it can call.",
        specialist.name
    ))
}

/// Run one model turn, reporting any assistant text (streamed live when the
/// specialist streams), and return the fully assembled message plus usage.
async fn one_turn(
    client: &Client,
    model: &Model,
    ctx: &Context,
    opts: &CompleteOptions,
    stream: bool,
    observer: &mut dyn FnMut(RunEvent<'_>),
) -> Result<(Message, Usage), String> {
    if stream {
        let mut events = client
            .stream(model, ctx, opts)
            .await
            .map_err(|e| e.to_string())?;
        while let Some(event) = events.next().await {
            match event.map_err(|e| e.to_string())? {
                StreamEvent::TextDelta { delta, .. } => observer(RunEvent::TextDelta(&delta)),
                StreamEvent::Done { usage, message, .. } => return Ok((message, usage)),
                StreamEvent::ThinkingDelta { .. } | StreamEvent::ToolCallDelta { .. } => {}
            }
        }
        Err("stream ended without a final event".to_string())
    } else {
        let completion = client
            .complete(model, ctx, opts)
            .await
            .map_err(|e| e.to_string())?;
        for block in &completion.message.content {
            if let ContentBlock::Text { text } = block {
                observer(RunEvent::Text(text));
            }
        }
        Ok((completion.message, completion.usage))
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
/// react.
fn run_tool_call(
    tools: &[ToolDef],
    id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    observer: &mut dyn FnMut(RunEvent<'_>),
) -> ContentBlock {
    let tool = match tools.iter().find(|t| t.name == tool_name) {
        Some(tool) => tool,
        None => {
            let message = format!("unknown tool: {tool_name}");
            observer(RunEvent::ToolFailed {
                name: tool_name,
                message: &message,
            });
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
mod tests {
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
            params: vec!["host".to_string()],
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
}
