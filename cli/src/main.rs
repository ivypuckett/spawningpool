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
    /// Show a defined entity's full definition.
    Show {
        #[command(subcommand)]
        entity: ShowEntity,
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
    Models {
        /// Discover the models a running LM Studio server currently has loaded
        /// (at `$LMSTUDIO_BASE_URL`) instead of listing the registry.
        #[arg(long)]
        remote: bool,
    },
    Tools,
}

#[derive(Subcommand)]
enum ShowEntity {
    Specialist { name: String },
    Provider { name: String },
    Model { name: String },
    Tool { name: String },
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
    /// Define a tool from an executable script; its `# desc:` and `# params:`
    /// header comments become the description and parameters.
    Tool {
        name: String,
        #[arg(long)]
        script: PathBuf,
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
        Command::List { kind } => list(kind).await,
        Command::Show { entity } => show(entity),
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

/// Execute one tool call by running its backing script, print the outcome, and
/// return the [`ContentBlock::ToolResult`] to feed back to the model. A failed
/// or unknown tool becomes a tool error so the model can react.
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
    match spawningpool::run_script(&tool.script, &vars) {
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

async fn list(kind: ListKind) -> Result<(), String> {
    // Remote model discovery queries a live server rather than the registry.
    if let ListKind::Models { remote: true } = kind {
        return list_remote_models().await;
    }
    let registry = store::load()?;
    let mut names: Vec<&String> = match kind {
        ListKind::Specialists => registry.specialists.keys().collect(),
        ListKind::Providers => registry.providers.keys().collect(),
        ListKind::Models { .. } => registry.models.keys().collect(),
        ListKind::Tools => registry.tools.keys().collect(),
    };
    names.sort();
    for name in names {
        println!("{name}");
    }
    Ok(())
}

/// Discover the model ids a running LM Studio server currently has loaded and
/// print them, sorted. Discovery is only meaningful for an OpenAI-compatible
/// server we can query (`GET {base_url}/v1/models`).
async fn list_remote_models() -> Result<(), String> {
    let models = Client::new()
        .list_models("lmstudio")
        .await
        .map_err(|e| e.to_string())?;
    let mut ids: Vec<String> = models.into_iter().map(|m| m.id).collect();
    ids.sort();
    for id in ids {
        println!("{id}");
    }
    Ok(())
}

/// Print an entity's full definition as pretty JSON, or error if it is absent.
/// Plain serializable definitions never fail to render.
fn show(entity: ShowEntity) -> Result<(), String> {
    let registry = store::load()?;
    let (found, what) = match entity {
        ShowEntity::Specialist { name } => (
            registry
                .specialists
                .get(&name)
                .map(|d| serde_json::to_string_pretty(d).expect("definition serializes")),
            format!("specialist {name}"),
        ),
        ShowEntity::Provider { name } => (
            registry
                .providers
                .get(&name)
                .map(|d| serde_json::to_string_pretty(d).expect("definition serializes")),
            format!("provider {name}"),
        ),
        ShowEntity::Model { name } => (
            registry
                .models
                .get(&name)
                .map(|d| serde_json::to_string_pretty(d).expect("definition serializes")),
            format!("model {name}"),
        ),
        ShowEntity::Tool { name } => (
            registry
                .tools
                .get(&name)
                .map(|d| serde_json::to_string_pretty(d).expect("definition serializes")),
            format!("tool {name}"),
        ),
    };
    match found {
        Some(json) => {
            println!("{json}");
            Ok(())
        }
        None => Err(format!("no such {what}")),
    }
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
        DefineEntity::Tool { name, script } => {
            let summary = spawningpool::summarize(&script).map_err(|e| e.to_string())?;
            let def = ToolDef {
                name: name.clone(),
                script: script.clone(),
                description: summary.desc.unwrap_or_default(),
                params: summary.params,
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

    fn write_script(body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "sp_cli_tool_{}_{}.sh",
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

    fn registry_with_tool(name: &str, script: PathBuf) -> Registry {
        let mut registry = Registry::default();
        registry.tools.insert(
            name.to_string(),
            ToolDef {
                name: name.to_string(),
                script,
                description: String::new(),
                params: vec![],
            },
        );
        registry
    }

    #[test]
    fn run_tool_call_returns_result_on_success() {
        let script = write_script("#!/bin/sh\necho \"hi $NAME\"\n");
        let registry = registry_with_tool("greet", script.clone());
        let block = run_tool_call(
            &registry,
            "id1",
            "greet",
            &serde_json::json!({ "NAME": "world" }),
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
    }

    #[test]
    fn run_tool_call_returns_error_on_nonzero_exit() {
        let script = write_script("#!/bin/sh\necho boom >&2\nexit 1\n");
        let registry = registry_with_tool("fail", script.clone());
        let block = run_tool_call(&registry, "id2", "fail", &serde_json::json!({}));
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
        let registry = Registry::default();
        let block = run_tool_call(&registry, "id3", "ghost", &serde_json::json!({}));
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
    }

    fn restore_registry_env(saved: Option<std::ffi::OsString>) {
        match saved {
            Some(v) => std::env::set_var("SPAWNINGPOOL_REGISTRY", v),
            None => std::env::remove_var("SPAWNINGPOOL_REGISTRY"),
        }
    }

    #[tokio::test]
    async fn define_list_show_and_delete_round_trip_through_the_store() {
        let _guard = store::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var_os("SPAWNINGPOOL_REGISTRY");
        let dir = std::env::temp_dir().join(format!("sp_cli_define_{}", std::process::id()));
        let path = dir.join("registry.json");
        std::env::set_var("SPAWNINGPOOL_REGISTRY", &path);

        define(DefineEntity::Provider {
            name: "anthropic".into(),
            api: "anthropic-messages".into(),
            base_url: "https://api.anthropic.com".into(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
        })
        .unwrap();

        // The provider is persisted and reloads from disk.
        assert!(store::load().unwrap().providers.contains_key("anthropic"));
        // Listing succeeds against the populated registry.
        list(ListKind::Providers).await.unwrap();
        // Showing a defined entity succeeds; an absent one errors.
        show(ShowEntity::Provider {
            name: "anthropic".into(),
        })
        .unwrap();
        let err = show(ShowEntity::Provider {
            name: "ghost".into(),
        })
        .unwrap_err();
        assert!(err.contains("no such"));

        // Deleting it removes it.
        delete(DeleteEntity::Provider {
            name: "anthropic".into(),
        })
        .unwrap();
        assert!(!store::load().unwrap().providers.contains_key("anthropic"));

        // Deleting something absent is an error.
        let err = delete(DeleteEntity::Provider {
            name: "ghost".into(),
        })
        .unwrap_err();
        assert!(err.contains("no such"));

        std::fs::remove_dir_all(&dir).ok();
        restore_registry_env(saved);
    }

    #[test]
    fn define_specialist_rejects_tools_and_constraint_together() {
        let _guard = store::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var_os("SPAWNINGPOOL_REGISTRY");
        let dir = std::env::temp_dir().join(format!("sp_cli_val_{}", std::process::id()));
        let path = dir.join("registry.json");
        std::env::set_var("SPAWNINGPOOL_REGISTRY", &path);

        let err = define(DefineEntity::Specialist {
            name: "bad".into(),
            provider: "p".into(),
            model: "m".into(),
            system_prompt: "s".into(),
            tools: Some("a,b".into()),
            constraint: Some("a".into()),
            reasoning: "off".into(),
            stream: false,
        })
        .unwrap_err();
        assert!(err.contains("tools and a constraint"));

        std::fs::remove_dir_all(&dir).ok();
        restore_registry_env(saved);
    }
}
