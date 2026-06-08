//! `sp` — the spawningpool CLI. Defines providers, models, specialists, and tools
//! into a persisted [`Registry`], and runs a specialist against a prompt.

mod store;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use futures::StreamExt;
use spawningpool::ai::{
    Api, Client, CompleteOptions, ContentBlock, Context, Message, Model, Reasoning, Role,
    StreamEvent, Usage,
};
use spawningpool::{ModelDef, ProviderDef, Registry, Specialist, ToolDef};
use std::collections::HashMap;
use std::io::Write;

#[derive(Parser)]
#[command(name = "sp", bin_name = "spawningpool", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a specialist against a prompt.
    #[command(alias = "spawn")]
    Run {
        #[arg(long)]
        specialist: String,
        #[arg(long)]
        prompt: String,
    },
    /// List defined entities.
    List {
        #[command(subcommand)]
        kind: ListKind,
    },
    /// Define an entity.
    Define {
        #[command(subcommand)]
        entity: DefineEntity,
    },
    /// Delete an entity.
    Delete {
        #[command(subcommand)]
        entity: DeleteEntity,
    },
}

#[derive(Subcommand)]
enum ListKind {
    Specialists,
    Providers,
    Models,
    Tools,
}

#[derive(Subcommand)]
enum DefineEntity {
    /// Define a provider (wire protocol + endpoint + key env var).
    Provider {
        name: String,
        #[arg(long)]
        api: String,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        api_key_env: Option<String>,
    },
    /// Define a model, keyed by its API id, against a provider.
    Model {
        id: String,
        #[arg(long)]
        provider: String,
        /// Display name; defaults to the id.
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        max_tokens: u32,
        #[arg(long)]
        context_window: u32,
    },
    /// Define a specialist template.
    Specialist {
        name: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        system_prompt: String,
        /// Comma-separated tool names.
        #[arg(long)]
        tools: Option<String>,
        /// A tool the specialist is forced to call (constrained decoding).
        #[arg(long)]
        constraint: Option<String>,
        #[arg(long, default_value = "off")]
        reasoning: String,
        /// Stream the response incrementally when this specialist runs.
        #[arg(long)]
        stream: bool,
    },
    /// Define a tool from a Taskfile task; its desc and `{{.VARS}}` become the
    /// description and parameters.
    Tool {
        name: String,
        #[arg(long)]
        taskfile: PathBuf,
        #[arg(long)]
        task: String,
    },
}

#[derive(Subcommand)]
enum DeleteEntity {
    Specialist { name: String },
    Provider { name: String },
    Model { name: String },
    Tool { name: String },
}

#[tokio::main]
async fn main() {
    if let Err(e) = run(Cli::parse()).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Run { specialist, prompt } => run_specialist(&specialist, &prompt).await,
        Command::List { kind } => list(kind),
        Command::Define { entity } => define(entity),
        Command::Delete { entity } => delete(entity),
    }
}

/// Cap on agentic turns, so a specialist that keeps calling tools without ever
/// settling on an answer terminates instead of looping forever.
const MAX_TURNS: usize = 16;

async fn run_specialist(name: &str, prompt: &str) -> Result<(), String> {
    let registry = store::load()?;
    let specialist = registry
        .specialists
        .get(name)
        .ok_or_else(|| format!("unknown specialist: {name}"))?;

    let model = registry.resolve_model(specialist)?;
    let mut ctx = registry.build_context(specialist, prompt)?;
    let mut opts = specialist.complete_options();
    // Source the API key from the provider's configured env var, if any.
    if let Some(env) = registry
        .providers
        .get(&specialist.provider)
        .and_then(|p| p.api_key_env.as_ref())
    {
        if let Ok(key) = std::env::var(env) {
            opts.api_key = Some(key);
        }
    }

    let client = Client::new();
    // A constrained specialist makes a single forced call; a tools specialist
    // runs agentically until it stops calling tools.
    let agentic = specialist.constraint.is_none();

    for _ in 0..MAX_TURNS {
        let (message, usage) = one_turn(&client, &model, &ctx, &opts, specialist.stream).await?;
        eprintln!("[usage] {} in / {} out", usage.input, usage.output);

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
            results.push(run_tool_call(&registry, id, tool_name, arguments));
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
        "specialist did not finish within {MAX_TURNS} turns"
    ))
}

/// Run one model turn, printing any assistant text (streamed live when the
/// specialist streams), and return the fully assembled message plus usage.
async fn one_turn(
    client: &Client,
    model: &Model,
    ctx: &Context,
    opts: &CompleteOptions,
    stream: bool,
) -> Result<(Message, Usage), String> {
    if stream {
        let mut events = client
            .stream(model, ctx, opts)
            .await
            .map_err(|e| e.to_string())?;
        let mut stdout = std::io::stdout();
        let mut printed_text = false;
        while let Some(event) = events.next().await {
            match event.map_err(|e| e.to_string())? {
                StreamEvent::TextDelta { delta, .. } => {
                    print!("{delta}");
                    stdout.flush().ok();
                    printed_text = true;
                }
                StreamEvent::Done { usage, message, .. } => {
                    if printed_text {
                        println!();
                    }
                    return Ok((message, usage));
                }
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
                println!("{text}");
            }
        }
        Ok((completion.message, completion.usage))
    }
}

/// Execute one tool call by running its backing Taskfile task, print the
/// outcome, and return the [`ContentBlock::ToolResult`] to feed back to the
/// model. A failed or unknown task becomes a tool error so the model can react.
fn run_tool_call(
    registry: &Registry,
    id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> ContentBlock {
    let tool = match registry.tools.get(tool_name) {
        Some(tool) => tool,
        None => {
            let msg = format!("unknown tool: {tool_name}");
            eprintln!("[tool {tool_name}] {msg}");
            return ContentBlock::tool_error(id, msg);
        }
    };

    let vars = args_to_vars(arguments);
    match spawningpool::run_task(&tool.taskfile, &tool.task, &vars) {
        Ok(run) => {
            println!("[tool {tool_name}]\n{}", run.output);
            if run.success {
                ContentBlock::tool_result(id, run.output)
            } else {
                ContentBlock::tool_error(id, run.output)
            }
        }
        Err(e) => {
            let msg = e.to_string();
            eprintln!("[tool {tool_name}] failed to run: {msg}");
            ContentBlock::tool_error(id, msg)
        }
    }
}

/// Lower a tool call's JSON arguments into the `KEY=value` variables a task
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

fn list(kind: ListKind) -> Result<(), String> {
    let registry = store::load()?;
    let mut names: Vec<&String> = match kind {
        ListKind::Specialists => registry.specialists.keys().collect(),
        ListKind::Providers => registry.providers.keys().collect(),
        ListKind::Models => registry.models.keys().collect(),
        ListKind::Tools => registry.tools.keys().collect(),
    };
    names.sort();
    for name in names {
        println!("{name}");
    }
    Ok(())
}

fn define(entity: DefineEntity) -> Result<(), String> {
    let mut registry = store::load()?;
    let what = match entity {
        DefineEntity::Provider {
            name,
            api,
            base_url,
            api_key_env,
        } => {
            let def = ProviderDef {
                name: name.clone(),
                api: api.parse::<Api>()?,
                base_url,
                api_key_env,
            };
            registry.providers.insert(name.clone(), def);
            format!("provider {name}")
        }
        DefineEntity::Model {
            id,
            provider,
            name,
            max_tokens,
            context_window,
        } => {
            let def = ModelDef {
                id: id.clone(),
                name: name.unwrap_or_else(|| id.clone()),
                provider,
                max_tokens,
                context_window,
            };
            registry.models.insert(id.clone(), def);
            format!("model {id}")
        }
        DefineEntity::Specialist {
            name,
            provider,
            model,
            system_prompt,
            tools,
            constraint,
            reasoning,
            stream,
        } => {
            let def = Specialist {
                name: name.clone(),
                provider,
                model,
                system_prompt,
                tools: parse_list(tools),
                constraint,
                reasoning: parse_reasoning(&reasoning)?,
                stream,
            };
            def.validate()?;
            registry.specialists.insert(name.clone(), def);
            format!("specialist {name}")
        }
        DefineEntity::Tool {
            name,
            taskfile,
            task,
        } => {
            let summary = spawningpool::summarize(&taskfile).map_err(|e| e.to_string())?;
            let entry = summary
                .get(&task)
                .ok_or_else(|| format!("task '{task}' not found in {}", taskfile.display()))?;
            let def = ToolDef {
                name: name.clone(),
                taskfile: taskfile.clone(),
                task,
                description: entry.desc.clone().unwrap_or_default(),
                params: entry.vars.clone(),
            };
            registry.tools.insert(name.clone(), def);
            format!("tool {name}")
        }
    };
    store::save(&registry)?;
    println!("defined {what}");
    Ok(())
}

fn delete(entity: DeleteEntity) -> Result<(), String> {
    let mut registry = store::load()?;
    let (removed, what) = match entity {
        DeleteEntity::Specialist { name } => (
            registry.specialists.remove(&name).is_some(),
            format!("specialist {name}"),
        ),
        DeleteEntity::Provider { name } => (
            registry.providers.remove(&name).is_some(),
            format!("provider {name}"),
        ),
        DeleteEntity::Model { name } => (
            registry.models.remove(&name).is_some(),
            format!("model {name}"),
        ),
        DeleteEntity::Tool { name } => (
            registry.tools.remove(&name).is_some(),
            format!("tool {name}"),
        ),
    };
    if !removed {
        return Err(format!("no such {what}"));
    }
    store::save(&registry)?;
    println!("deleted {what}");
    Ok(())
}

/// Split a comma-separated list flag into trimmed, non-empty names.
fn parse_list(raw: Option<String>) -> Vec<String> {
    raw.into_iter()
        .flat_map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn parse_reasoning(raw: &str) -> Result<Reasoning, String> {
    match raw {
        "off" => Ok(Reasoning::Off),
        "low" => Ok(Reasoning::Low),
        "medium" => Ok(Reasoning::Medium),
        "high" => Ok(Reasoning::High),
        other => Err(format!("unknown reasoning '{other}' (off|low|medium|high)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_splits_and_trims() {
        assert_eq!(
            parse_list(Some("a, b ,c".into())),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        assert!(parse_list(None).is_empty());
        assert!(parse_list(Some("  ,  ".into())).is_empty());
    }

    #[test]
    fn parse_reasoning_maps_levels_and_rejects_unknown() {
        assert_eq!(parse_reasoning("high"), Ok(Reasoning::High));
        assert_eq!(parse_reasoning("off"), Ok(Reasoning::Off));
        assert!(parse_reasoning("ultra").is_err());
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
}
