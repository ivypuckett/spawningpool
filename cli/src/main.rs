//! `sp` — the spawningpool CLI. Defines providers, models, specialists, and tools
//! into a persisted [`Registry`], and runs a specialist against a prompt.

mod store;

use std::path::{Path, PathBuf};

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
        "specialist '{name}' did not finish within {MAX_TURNS} turns.\n  \
         It kept calling tools without settling on an answer — inspect the tool \
         outputs above, tighten its system prompt, or reduce the tools it can call."
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
            let path = tool.script.display();
            let msg = format!(
                "tool '{tool_name}' could not run its script {path}: {e}\n  \
                 Ensure it exists, is executable (chmod +x {path}), and has a shebang (e.g. #!/bin/sh)."
            );
            eprintln!("[tool {tool_name}] {msg}");
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
            check_model_refs(&registry, &def)?;
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
            check_specialist_refs(&registry, &def)?;
            registry.specialists.insert(name.clone(), def);
            format!("specialist {name}")
        }
        DefineEntity::Tool { name, script } => {
            // Resolve to an absolute, runnable path now so the tool works
            // regardless of the directory `sp run` is later invoked from, and
            // so an un-runnable script fails here with a fix rather than as a
            // cryptic launch error mid-run.
            let script = resolve_script(&script)?;
            let summary = spawningpool::summarize(&script).map_err(|e| e.to_string())?;
            if summary.desc.is_none() {
                eprintln!(
                    "warning: tool '{name}' has no '# desc:' header, so the model will see an \
                     empty description.\n  Add a line like '# desc: <what it does>' to {}.",
                    script.display()
                );
            }
            let def = ToolDef {
                name: name.clone(),
                script,
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
    // Collect any entities that still reference what we're about to remove,
    // before removing it, so we can warn about the references it leaves dangling.
    let (removed, what, kind, name, referrers) = match entity {
        DeleteEntity::Specialist { name } => (
            registry.specialists.remove(&name).is_some(),
            format!("specialist {name}"),
            "specialist",
            name,
            Vec::new(),
        ),
        DeleteEntity::Provider { name } => {
            let referrers = referrers_of_provider(&registry, &name);
            (
                registry.providers.remove(&name).is_some(),
                format!("provider {name}"),
                "provider",
                name,
                referrers,
            )
        }
        DeleteEntity::Model { name } => {
            let referrers = referrers_of_model(&registry, &name);
            (
                registry.models.remove(&name).is_some(),
                format!("model {name}"),
                "model",
                name,
                referrers,
            )
        }
        DeleteEntity::Tool { name } => {
            let referrers = referrers_of_tool(&registry, &name);
            (
                registry.tools.remove(&name).is_some(),
                format!("tool {name}"),
                "tool",
                name,
                referrers,
            )
        }
    };
    if !removed {
        return Err(format!("no such {what}"));
    }
    store::save(&registry)?;
    println!("deleted {what}");
    warn_orphans(kind, &name, &referrers);
    Ok(())
}

/// Specialists matching `pred`, formatted as `specialist 'name'` and sorted, for
/// orphan warnings.
fn specialists_matching(registry: &Registry, pred: impl Fn(&Specialist) -> bool) -> Vec<String> {
    let mut names: Vec<String> = registry
        .specialists
        .values()
        .filter(|s| pred(s))
        .map(|s| format!("specialist '{}'", s.name))
        .collect();
    names.sort();
    names
}

/// Entities that reference a provider: specialists pointing at it, plus models
/// defined under it.
fn referrers_of_provider(registry: &Registry, name: &str) -> Vec<String> {
    let mut referrers = specialists_matching(registry, |s| s.provider == name);
    let mut models: Vec<String> = registry
        .models
        .values()
        .filter(|m| m.provider == name)
        .map(|m| format!("model '{}'", m.id))
        .collect();
    models.sort();
    referrers.extend(models);
    referrers
}

fn referrers_of_model(registry: &Registry, name: &str) -> Vec<String> {
    specialists_matching(registry, |s| s.model == name)
}

fn referrers_of_tool(registry: &Registry, name: &str) -> Vec<String> {
    specialists_matching(registry, |s| s.tool_names().iter().any(|t| t == name))
}

/// Warn that a just-deleted entity left dangling references, naming each
/// referrer and how they'll fail.
fn warn_orphans(kind: &str, name: &str, referrers: &[String]) {
    if referrers.is_empty() {
        return;
    }
    for referrer in referrers {
        eprintln!("warning: {kind} '{name}' is still referenced by {referrer}");
    }
    eprintln!(
        "  Those references will fail at run time until you redefine {kind} '{name}' or repoint them."
    );
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

/// Sorted, comma-joined keys of a registry section, for the "Defined X: ..."
/// line in a referential error. Names the empty case explicitly so the message
/// reads sensibly before anything is defined.
fn available_names<V>(map: &HashMap<String, V>) -> String {
    if map.is_empty() {
        return "(none defined yet)".to_string();
    }
    let mut names: Vec<&str> = map.keys().map(String::as_str).collect();
    names.sort();
    names.join(", ")
}

/// Reject a model that names a provider the registry doesn't hold, naming the
/// provider, what's available, and how to define it.
fn check_model_refs(registry: &Registry, model: &ModelDef) -> Result<(), String> {
    if !registry.providers.contains_key(&model.provider) {
        return Err([
            format!(
                "model '{}' references provider '{}', which isn't defined.",
                model.id, model.provider
            ),
            String::new(),
            format!("  Defined providers: {}", available_names(&registry.providers)),
            String::new(),
            "  Define it first:".to_string(),
            format!(
                "      sp define provider {} --api <anthropic-messages|openai-completions> --base-url <url>",
                model.provider
            ),
        ]
        .join("\n"));
    }
    Ok(())
}

/// Reject a specialist that references a provider, model, or tool the registry
/// doesn't hold. Each error names the missing entity, lists what's defined, and
/// shows the command that would create it.
fn check_specialist_refs(registry: &Registry, specialist: &Specialist) -> Result<(), String> {
    if !registry.providers.contains_key(&specialist.provider) {
        return Err([
            format!(
                "specialist '{}' references provider '{}', which isn't defined.",
                specialist.name, specialist.provider
            ),
            String::new(),
            format!("  Defined providers: {}", available_names(&registry.providers)),
            String::new(),
            "  Define it first:".to_string(),
            format!(
                "      sp define provider {} --api <anthropic-messages|openai-completions> --base-url <url>",
                specialist.provider
            ),
            String::new(),
            "  ...or point the specialist at one that exists with --provider.".to_string(),
        ]
        .join("\n"));
    }
    if !registry.models.contains_key(&specialist.model) {
        return Err([
            format!(
                "specialist '{}' references model '{}', which isn't defined.",
                specialist.name, specialist.model
            ),
            String::new(),
            format!("  Defined models: {}", available_names(&registry.models)),
            String::new(),
            "  Define it first:".to_string(),
            format!(
                "      sp define model {} --provider {} --max-tokens <n> --context-window <n>",
                specialist.model, specialist.provider
            ),
            String::new(),
            "  ...or point the specialist at one that exists with --model.".to_string(),
        ]
        .join("\n"));
    }
    for tool in specialist.tool_names() {
        if !registry.tools.contains_key(tool) {
            return Err([
                format!(
                    "specialist '{}' references tool '{}', which isn't defined.",
                    specialist.name, tool
                ),
                String::new(),
                format!("  Defined tools: {}", available_names(&registry.tools)),
                String::new(),
                "  Back it with a script:".to_string(),
                format!("      sp define tool {tool} --script <path>"),
            ]
            .join("\n"));
        }
    }
    Ok(())
}

/// Resolve a tool script to an absolute path and confirm it can actually run:
/// it must exist and have the executable bit set. Storing it absolute means the
/// tool resolves no matter where `sp run` is invoked from.
fn resolve_script(script: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    let path = std::fs::canonicalize(script).map_err(|e| {
        format!(
            "tool script {} can't be read: {e}\n  Check the path is right and the file exists.",
            script.display()
        )
    })?;
    let mode = std::fs::metadata(&path)
        .map_err(|e| format!("tool script {} can't be read: {e}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o111 == 0 {
        return Err([
            format!("tool script {} isn't executable.", path.display()),
            String::new(),
            "  Make it runnable:".to_string(),
            format!("      chmod +x {}", path.display()),
            String::new(),
            "  (It also needs a shebang line, e.g. #!/bin/sh.)".to_string(),
        ]
        .join("\n"));
    }
    Ok(path)
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

    fn populated_registry() -> Registry {
        let mut registry = Registry::default();
        registry.providers.insert(
            "anthropic".into(),
            ProviderDef {
                name: "anthropic".into(),
                api: spawningpool::ai::Api::AnthropicMessages,
                base_url: "https://api.anthropic.com".into(),
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
            },
        );
        registry.models.insert(
            "claude".into(),
            ModelDef {
                id: "claude".into(),
                name: "Claude".into(),
                provider: "anthropic".into(),
                max_tokens: 1024,
                context_window: 200_000,
            },
        );
        registry.tools.insert(
            "ping".into(),
            ToolDef {
                name: "ping".into(),
                script: PathBuf::from("ping.sh"),
                description: "Ping".into(),
                params: vec![],
            },
        );
        registry
    }

    fn specialist_ref(
        provider: &str,
        model: &str,
        tools: Vec<String>,
        constraint: Option<String>,
    ) -> Specialist {
        Specialist {
            name: "spec".into(),
            provider: provider.into(),
            model: model.into(),
            system_prompt: "s".into(),
            tools,
            constraint,
            reasoning: Reasoning::Off,
            stream: false,
        }
    }

    #[test]
    fn available_names_lists_sorted_or_notes_emptiness() {
        let mut map: HashMap<String, u8> = HashMap::new();
        assert_eq!(available_names(&map), "(none defined yet)");
        map.insert("b".into(), 0);
        map.insert("a".into(), 0);
        assert_eq!(available_names(&map), "a, b");
    }

    #[test]
    fn check_specialist_refs_passes_when_all_present() {
        let registry = populated_registry();
        let spec = specialist_ref("anthropic", "claude", vec!["ping".into()], None);
        assert!(check_specialist_refs(&registry, &spec).is_ok());
    }

    #[test]
    fn check_specialist_refs_reports_missing_provider_model_and_tool() {
        let registry = populated_registry();

        let err =
            check_specialist_refs(&registry, &specialist_ref("ghost", "claude", vec![], None))
                .unwrap_err();
        assert!(err.contains("references provider 'ghost'"));
        assert!(err.contains("sp define provider ghost"));

        let err = check_specialist_refs(
            &registry,
            &specialist_ref("anthropic", "nope", vec![], None),
        )
        .unwrap_err();
        assert!(err.contains("references model 'nope'"));
        assert!(err.contains("sp define model nope"));

        let err = check_specialist_refs(
            &registry,
            &specialist_ref("anthropic", "claude", vec!["absent".into()], None),
        )
        .unwrap_err();
        assert!(err.contains("references tool 'absent'"));
        assert!(err.contains("sp define tool absent"));
    }

    #[test]
    fn check_specialist_refs_validates_the_constrained_tool() {
        let registry = populated_registry();
        // A constraint names a tool too, so an undefined forced tool is caught.
        let spec = specialist_ref("anthropic", "claude", vec![], Some("absent".into()));
        let err = check_specialist_refs(&registry, &spec).unwrap_err();
        assert!(err.contains("references tool 'absent'"));
    }

    #[test]
    fn check_model_refs_requires_a_defined_provider() {
        let registry = populated_registry();
        let ok = ModelDef {
            id: "m".into(),
            name: "m".into(),
            provider: "anthropic".into(),
            max_tokens: 1,
            context_window: 1,
        };
        assert!(check_model_refs(&registry, &ok).is_ok());

        let bad = ModelDef {
            provider: "ghost".into(),
            ..ok
        };
        let err = check_model_refs(&registry, &bad).unwrap_err();
        assert!(err.contains("references provider 'ghost'"));
        assert!(err.contains("sp define provider ghost"));
    }

    #[test]
    fn referrers_find_entities_pointing_at_a_target() {
        let mut registry = populated_registry();
        registry.specialists.insert(
            "spec".into(),
            specialist_ref("anthropic", "claude", vec!["ping".into()], None),
        );

        // A provider is referenced by both the specialist and the model under it.
        assert_eq!(
            referrers_of_provider(&registry, "anthropic"),
            vec![
                "specialist 'spec'".to_string(),
                "model 'claude'".to_string()
            ]
        );
        assert_eq!(
            referrers_of_model(&registry, "claude"),
            vec!["specialist 'spec'".to_string()]
        );
        assert_eq!(
            referrers_of_tool(&registry, "ping"),
            vec!["specialist 'spec'".to_string()]
        );

        // An unreferenced name has no referrers.
        assert!(referrers_of_provider(&registry, "openai").is_empty());
    }

    #[test]
    fn referrers_of_tool_includes_a_constrained_tool() {
        let mut registry = populated_registry();
        registry.specialists.insert(
            "spec".into(),
            specialist_ref("anthropic", "claude", vec![], Some("ping".into())),
        );
        assert_eq!(
            referrers_of_tool(&registry, "ping"),
            vec!["specialist 'spec'".to_string()]
        );
    }

    #[test]
    fn resolve_script_returns_absolute_path_for_executable() {
        let script = write_script("#!/bin/sh\necho hi\n");
        let resolved = resolve_script(&script).unwrap();
        std::fs::remove_file(&script).ok();
        assert!(resolved.is_absolute());
    }

    #[test]
    fn resolve_script_rejects_non_executable_with_chmod_hint() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "sp_noexec_{}_{}.sh",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = resolve_script(&path).unwrap_err();
        std::fs::remove_file(&path).ok();
        assert!(err.contains("isn't executable"));
        assert!(err.contains("chmod +x"));
    }

    #[test]
    fn run_tool_call_enriches_launch_failure_with_remediation() {
        // A tool whose script can't be launched surfaces a fix, not a raw OS error.
        let registry = registry_with_tool(
            "ghost_script",
            PathBuf::from("/nonexistent/sp_tool_does_not_exist.sh"),
        );
        let block = run_tool_call(&registry, "id", "ghost_script", &serde_json::json!({}));
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
