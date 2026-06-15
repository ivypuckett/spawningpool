//! The `spawningpool` CLI. Defines providers, models, specialists, and tools
//! into a persisted [`Registry`], and runs a specialist against a prompt.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use spawningpool::ai::{Api, Client, Reasoning, StopReason};
use spawningpool::{
    EntityKind, ModelDef, ProviderDef, Referrer, Registry, RunEvent, ScriptError, Specialist,
};
use std::collections::HashMap;
use std::io::Write;

mod tui;

#[derive(Parser)]
#[command(name = "spawningpool", bin_name = "spawningpool", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::ValueEnum, Clone)]
enum OutputFormat {
    Json,
    Plaintext,
}

#[derive(Subcommand)]
enum Command {
    /// Run a specialist, workflow, or tool.
    #[command(alias = "spawn")]
    Run {
        #[command(subcommand)]
        target: RunTarget,
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
    /// Browse and manage everything in an interactive terminal UI.
    Tui,
}

#[derive(Subcommand)]
enum RunTarget {
    /// Run a specialist against a prompt.
    #[command(aliases = ["lenny", "ling"])]
    Specialist {
        name: String,
        #[arg(long)]
        prompt: String,
        /// Output format. Defaults to `json` (machine-readable envelope with
        /// output, thinking, token counts, stopReason, model, specialist,
        /// turns, and toolCalls). Use `plaintext` for streaming terminal output.
        #[arg(long, value_name = "FORMAT")]
        output: Option<OutputFormat>,
    },
    /// Execute a workflow from the `workflows/` folder, by name.
    Workflow { name: String },
    /// Run a single tool script directly, by name.
    Tool {
        name: String,
        /// A tool parameter, as `KEY=VALUE`. Repeatable.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ListKind {
    #[command(aliases = ["specialist", "lenny", "ling", "lennys", "lings"])]
    Specialists,
    #[command(alias = "provider")]
    Providers,
    #[command(alias = "model")]
    Models {
        /// Discover the models a running LM Studio server currently has loaded
        /// (at `$LMSTUDIO_BASE_URL`) instead of listing the registry.
        #[arg(long)]
        remote: bool,
    },
    #[command(alias = "tool")]
    Tools,
}

#[derive(Subcommand)]
enum ShowEntity {
    #[command(aliases = ["lenny", "ling"])]
    Specialist {
        name: String,
    },
    Provider {
        name: String,
    },
    Model {
        name: String,
    },
    Tool {
        name: String,
    },
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
        /// Declare that this provider's endpoint supports constrained decoding,
        /// so constrained specialists force their tool call via grammar-constrained
        /// `response_format` instead of `tool_choice`. OpenAI-compatible only.
        #[arg(long)]
        constrained_decoding: bool,
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
    #[command(aliases = ["lenny", "ling"])]
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
        /// A tool the specialist is forced to call. Realized via the portable
        /// tool-call trick (forced `tool_choice`), or grammar-constrained decoding
        /// if the provider was defined `--constrained-decoding`.
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
    #[command(aliases = ["lenny", "ling"])]
    Specialist {
        name: String,
    },
    Provider {
        name: String,
    },
    Model {
        name: String,
    },
    Tool {
        name: String,
    },
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
        // Bare `spawningpool` reads where you are in the provider → model → specialist →
        // run progression and shows the next step.
        None => status(),
        Some(Command::Run { target }) => match target {
            RunTarget::Specialist {
                name,
                prompt,
                output,
            } => run_specialist(&name, &prompt, output).await,
            RunTarget::Workflow { name } => run_workflow(&name).await,
            RunTarget::Tool { name, args } => run_tool(&name, &args),
        },
        Some(Command::List { kind }) => list(kind).await,
        Some(Command::Show { entity }) => show(entity),
        Some(Command::Define { entity }) => define(entity),
        Some(Command::Delete { entity }) => delete(entity),
        Some(Command::Tui) => tui::launch().await,
    }
}

/// Print a state-aware onboarding panel: where the user is in the
/// provider → model → specialist → run progression, the exact next command,
/// and any provider whose API-key env var isn't set.
fn status() -> Result<(), String> {
    let registry = spawningpool::store::load()?;
    println!("{}", onboarding_message(&registry));
    for warning in unset_key_warnings(&registry, |env| std::env::var_os(env).is_some()) {
        eprintln!("{warning}");
    }
    Ok(())
}

/// The progression's four rungs, with the completed ones checked, plus a
/// `[current/4]` marker. `done` is how many rungs are already satisfied.
fn progress(done: usize) -> String {
    let labels = ["provider", "model", "specialist", "run"];
    let parts: Vec<String> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            if i < done {
                format!("{label} \u{2713}")
            } else {
                label.to_string()
            }
        })
        .collect();
    let current = (done + 1).min(labels.len());
    format!("  [{current}/4] {}", parts.join(" \u{b7} "))
}

/// Pick where the user is by what the registry holds — providers, then models,
/// then specialists — and render the panel for that rung. Examples use the
/// user's real entity names so they're copy-pasteable.
fn onboarding_message(registry: &Registry) -> String {
    if registry.providers.is_empty() {
        empty_state()
    } else if registry.models.is_empty() {
        no_models_state(registry)
    } else if registry.specialists.is_empty() {
        no_specialists_state(registry)
    } else {
        ready_state(registry)
    }
}

fn empty_state() -> String {
    [
        "spawningpool — nothing defined yet. Let's set up your first specialist.",
        "",
        &progress(0),
        "",
        "Step 1: define a provider — the API your specialists talk to. Pick one:",
        "",
        "  Anthropic (Claude, hosted):",
        "      spawningpool define provider anthropic --api anthropic-messages \\",
        "        --base-url https://api.anthropic.com --api-key-env ANTHROPIC_API_KEY",
        "",
        "  LM Studio (local, OpenAI-compatible):",
        "      spawningpool define provider lmstudio --api openai-completions \\",
        "        --base-url http://localhost:1234/v1",
        "      (add --constrained-decoding if the server supports it, for a hard",
        "       guarantee on a specialist's forced tool call.)",
        "",
        "Then run `spawningpool` again for the next step.",
    ]
    .join("\n")
}

fn no_models_state(registry: &Registry) -> String {
    let mut lines = vec![
        "spawningpool — provider defined. Next: add a model.".to_string(),
        String::new(),
        progress(1),
        String::new(),
        format!("Your providers: {}.", available_names(&registry.providers)),
        String::new(),
        "Step 2: define a model under one of them.".to_string(),
        String::new(),
        "  Manually:".to_string(),
        "      spawningpool define model <id> --provider <provider> --max-tokens <n> --context-window <n>"
            .to_string(),
    ];
    // Discovery only works against an OpenAI-compatible server we can query.
    if registry
        .providers
        .values()
        .any(|p| matches!(p.api, Api::OpenAiCompletions))
    {
        lines.extend([
            String::new(),
            "  Or discover what a running LM Studio server has loaded:".to_string(),
            "      spawningpool list models --remote".to_string(),
            "  (then define the one you want with `spawningpool define model`).".to_string(),
        ]);
    }
    lines.join("\n")
}

fn no_specialists_state(registry: &Registry) -> String {
    // Use a real model + its provider so the example is copy-pasteable.
    let model = registry
        .models
        .values()
        .min_by(|a, b| a.id.cmp(&b.id))
        .expect("registry has models in this state");
    [
        "spawningpool — model ready. Next: define a specialist.".to_string(),
        String::new(),
        progress(2),
        String::new(),
        "A specialist is a hyper-specific agent: one model, one system prompt,".to_string(),
        "and an optional set of tools (scripts) it may call.".to_string(),
        String::new(),
        "Step 3: define one.".to_string(),
        String::new(),
        format!(
            "      spawningpool define specialist <name> --provider {} --model {} \\",
            model.provider, model.id
        ),
        "        --system-prompt '<what this specialist does>'".to_string(),
        String::new(),
        "  Optional: to let it call a tool, add one first. A tool is just an".to_string(),
        "  executable script in ~/.spawningpool/tools/. Drop one in (or run the".to_string(),
        "  command below), then pass --tools <name> above:".to_string(),
        "      spawningpool define tool <name> --script <path>".to_string(),
    ]
    .join("\n")
}

fn ready_state(registry: &Registry) -> String {
    // Name a real specialist so the run command works as shown.
    let specialist = registry
        .specialists
        .values()
        .min_by(|a, b| a.name.cmp(&b.name))
        .expect("registry has specialists in this state");
    [
        "spawningpool — you're all set.".to_string(),
        String::new(),
        progress(3),
        String::new(),
        "Run a specialist against a prompt:".to_string(),
        String::new(),
        format!(
            "      spawningpool run specialist {} --prompt '<your prompt>'",
            specialist.name
        ),
        String::new(),
        format!("  Specialists: {}", available_names(&registry.specialists)),
        String::new(),
        "  To give a specialist a tool to call, put an executable script in".to_string(),
        "  ~/.spawningpool/tools/ (or `spawningpool define tool <name> --script <path>`),"
            .to_string(),
        "  then add --tools <name> when you define the specialist. `spawningpool list tools`"
            .to_string(),
        "  shows what's there.".to_string(),
    ]
    .join("\n")
}

/// Warn about every provider that sources its API key from an env var that
/// isn't set — the most common silent failure, surfaced before a run hits it.
/// `is_set` answers whether a given variable is present, injected for testing.
fn unset_key_warnings(registry: &Registry, is_set: impl Fn(&str) -> bool) -> Vec<String> {
    let mut providers: Vec<&ProviderDef> = registry.providers.values().collect();
    providers.sort_by(|a, b| a.name.cmp(&b.name));
    providers
        .into_iter()
        .filter_map(|p| {
            let env = p.api_key_env.as_ref()?;
            if is_set(env) {
                return None;
            }
            Some(format!(
                "warning: provider '{}' reads its API key from ${env}, which isn't set.\n  \
                 export {env}=<your key> before running a specialist that uses it.",
                p.name
            ))
        })
        .collect()
}

async fn run_specialist(
    name: &str,
    prompt: &str,
    output: Option<OutputFormat>,
) -> Result<(), String> {
    let registry = spawningpool::store::load()?;
    let specialist = registry
        .specialists
        .get(name)
        .ok_or_else(|| format!("unknown specialist: {name}"))?;

    // Resolve the specialist's tools from the folder up front, so a missing or
    // unreadable tool fails before the model starts rather than mid-run.
    let tools = spawningpool::tools::resolve_all(
        &spawningpool::store::tools_dir(),
        specialist.tool_names(),
    )?;

    let mut opts = specialist.complete_options();
    if let Some(provider) = registry.providers.get(&specialist.provider) {
        // Source the API key from the provider's configured env var, if any.
        if let Some(env) = provider.api_key_env.as_ref() {
            if let Ok(key) = std::env::var(env) {
                opts.api_key = Some(key);
            }
        }
        // Honor the provider's declared constrained-decoding capability.
        opts.constrained_decoding = provider.constrained_decoding;
    }

    let client = Client::new();

    match output {
        None | Some(OutputFormat::Json) => {
            let mut output = String::new();
            let mut thinking = String::new();
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut stop_reason: Option<StopReason> = None;
            let mut turns: u32 = 0;
            let mut tool_calls: Vec<serde_json::Value> = Vec::new();
            let mut render = |event: RunEvent<'_>| match event {
                RunEvent::TextDelta(delta) => output.push_str(delta),
                RunEvent::Text(t) => output.push_str(t),
                RunEvent::ThinkingDelta(delta) => thinking.push_str(delta),
                RunEvent::Thinking(t) => thinking.push_str(t),
                RunEvent::TurnDone { stop_reason: sr } => {
                    stop_reason = Some(sr);
                    turns += 1;
                }
                RunEvent::Usage(usage) => {
                    input_tokens += usage.input;
                    output_tokens += usage.output;
                }
                RunEvent::ToolRan {
                    name,
                    output: out,
                    success,
                } => tool_calls.push(serde_json::json!({
                    "name": name,
                    "success": success,
                    "output": out,
                })),
                RunEvent::ToolFailed { name, message } => tool_calls.push(serde_json::json!({
                    "name": name,
                    "success": false,
                    "output": message,
                })),
            };
            spawningpool::run::run_specialist(
                &client,
                &registry,
                specialist,
                prompt,
                &tools,
                &opts,
                &mut render,
            )
            .await?;
            println!(
                "{}",
                serde_json::json!({
                    "output": output,
                    "thinking": thinking,
                    "inputTokens": input_tokens,
                    "outputTokens": output_tokens,
                    "stopReason": stop_reason,
                    "model": specialist.model,
                    "specialist": name,
                    "turns": turns,
                    "toolCalls": tool_calls,
                })
            );
            Ok(())
        }
        Some(OutputFormat::Plaintext) => {
            // Render the run to the terminal: assistant text on stdout (streamed
            // live), usage and tool failures on stderr, tool output on stdout.
            // `printed_text` tracks streamed deltas so a trailing newline lands
            // before the usage line.
            let mut printed_text = false;
            let mut render = |event: RunEvent<'_>| match event {
                RunEvent::TextDelta(delta) => {
                    print!("{delta}");
                    std::io::stdout().flush().ok();
                    printed_text = true;
                }
                RunEvent::Text(text) => println!("{text}"),
                RunEvent::ThinkingDelta(_) | RunEvent::Thinking(_) | RunEvent::TurnDone { .. } => {}
                RunEvent::Usage(usage) => {
                    if std::mem::take(&mut printed_text) {
                        println!();
                    }
                    eprintln!("[usage] {} in / {} out", usage.input, usage.output);
                }
                RunEvent::ToolRan { name, output, .. } => println!("[tool {name}]\n{output}"),
                RunEvent::ToolFailed { name, message } => eprintln!("[tool {name}] {message}"),
            };
            spawningpool::run::run_specialist(
                &client,
                &registry,
                specialist,
                prompt,
                &tools,
                &opts,
                &mut render,
            )
            .await
        }
    }
}

/// Execute a workflow from the `workflows/` folder by name: parse it, resolve
/// the tool catalog, type-check, then evaluate. Prints the workflow's result
/// value as JSON.
async fn run_workflow(name: &str) -> Result<(), String> {
    let registry = spawningpool::store::load()?;
    let source = spawningpool::workflow::source(&spawningpool::store::workflows_dir(), name)?;
    let workflow = spawningpool::workflow::parse(&source)
        .map_err(|e| format!("workflow '{name}' is invalid: {e}"))?;

    // Resolve exactly the tools the workflow references — its `call` tools plus
    // the tools its specialists need — so an unrelated broken or ambiguous tool
    // elsewhere in the catalog can't block a workflow that doesn't use it.
    let refs = spawningpool::workflow::referenced(&workflow, &registry);
    let tools_dir = spawningpool::store::tools_dir();
    let tool_names: Vec<String> = refs.tools.iter().cloned().collect();
    let tools = spawningpool::tools::resolve_all(&tools_dir, &tool_names)?;

    spawningpool::workflow::check(&workflow, &registry, &tools)
        .map_err(|e| format!("workflow '{name}' failed type-checking: {e}"))?;

    let keys = provider_keys(&registry);
    warn_unset_keys(&refs.specialists, &registry, &keys);
    let client = Client::new();
    let result = spawningpool::workflow::eval(&workflow, &registry, &tools, &client, &keys)
        .await
        .map_err(|e| format!("workflow '{name}' failed: {e}"))?;
    println!("{result}");
    Ok(())
}

/// Map each provider to its API key, read from the provider's configured
/// `api_key_env`. A provider with no key env, or whose env isn't set, is simply
/// omitted; specialists on it run without a key (matching `run specialist`).
fn provider_keys(registry: &Registry) -> HashMap<String, String> {
    let mut keys = HashMap::new();
    for provider in registry.providers.values() {
        if let Some(env) = provider.api_key_env.as_ref() {
            if let Ok(key) = std::env::var(env) {
                keys.insert(provider.name.clone(), key);
            }
        }
    }
    keys
}

/// Warn on stderr about each provider a workflow's specialists need but whose API
/// key isn't set, so a missing key surfaces before the run rather than as an auth
/// failure mid-workflow. Mirrors the bare-`spawningpool` key warning.
fn warn_unset_keys(
    specialists: &std::collections::BTreeSet<String>,
    registry: &Registry,
    keys: &HashMap<String, String>,
) {
    let mut warned = std::collections::BTreeSet::new();
    for spec_name in specialists {
        if let Some(spec) = registry.specialists.get(spec_name) {
            if let Some(provider) = registry.providers.get(&spec.provider) {
                if provider.api_key_env.is_some()
                    && !keys.contains_key(&provider.name)
                    && warned.insert(provider.name.clone())
                {
                    eprintln!(
                        "warning: API key for provider '{}' is unset; specialists using it will fail",
                        provider.name
                    );
                }
            }
        }
    }
}

/// Run a single tool script directly, with `KEY=VALUE` params, and print the
/// structured JSON the tool writes to `$SP_OUTPUT_PATH`.
fn run_tool(name: &str, args: &[String]) -> Result<(), String> {
    let tool = spawningpool::tools::resolve(&spawningpool::store::tools_dir(), name)?;

    let mut vars = HashMap::new();
    for arg in args {
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("invalid --arg '{arg}': expected KEY=VALUE"))?;
        vars.insert(key.to_string(), value.to_string());
    }

    // Validate args against the tool's declared params (all are required), so a
    // typo'd or missing param fails here instead of silently passing an empty or
    // absent env var to the script — matching the checks a workflow `call` gets.
    for key in vars.keys() {
        if !tool.params.iter().any(|p| &p.name == key) {
            return Err(format!("tool '{name}' has no parameter '{key}'"));
        }
    }
    for param in &tool.params {
        if !vars.contains_key(&param.name) {
            return Err(format!(
                "tool '{name}' is missing required parameter '{}'",
                param.name
            ));
        }
    }

    let run = spawningpool::run_script(&tool.script, &vars)
        .map_err(|e| format!("failed to run tool '{name}': {e}"))?;

    let output = run.structured_output.ok_or_else(|| {
        format!(
            "tool '{name}' didn't write to $SP_OUTPUT_PATH; it has no structured output to show"
        )
    })?;
    println!("{output}");
    Ok(())
}

async fn list(kind: ListKind) -> Result<(), String> {
    // Remote model discovery queries a live server rather than the registry.
    if let ListKind::Models { remote: true } = kind {
        return list_remote_models().await;
    }
    // Tools live in a folder, not the registry, so list them from disk.
    if let ListKind::Tools = kind {
        let dir = spawningpool::store::tools_dir();
        let names = spawningpool::tools::list(&dir)?;
        // An empty folder is the most confusing case ("where do tools come
        // from?"), so point the way — on stderr, leaving stdout pipe-clean.
        if names.is_empty() {
            eprintln!(
                "no tools yet — drop an executable script in {} or run \
                 `spawningpool define tool <name> --script <path>` (see `spawningpool show tool`).",
                dir.display()
            );
        }
        for name in names {
            println!("{name}");
        }
        return Ok(());
    }
    let registry = spawningpool::store::load()?;
    let mut names: Vec<&String> = match kind {
        ListKind::Specialists => registry.specialists.keys().collect(),
        ListKind::Providers => registry.providers.keys().collect(),
        ListKind::Models { .. } => registry.models.keys().collect(),
        ListKind::Tools => unreachable!("tools listed from the folder above"),
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
    let registry = spawningpool::store::load()?;
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
            // Tools live in the folder; resolve reads the script's header.
            spawningpool::tools::resolve(&spawningpool::store::tools_dir(), &name)
                .ok()
                .map(|d| serde_json::to_string_pretty(&d).expect("definition serializes")),
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
    // Tools aren't part of the registry, so install them straight into the
    // folder without touching it.
    if let DefineEntity::Tool { name, script } = &entity {
        return define_tool(name, script);
    }
    let mut registry = spawningpool::store::load()?;
    let what = match entity {
        DefineEntity::Provider {
            name,
            api,
            base_url,
            api_key_env,
            constrained_decoding,
        } => {
            let def = ProviderDef {
                name: name.clone(),
                api: api.parse::<Api>()?,
                base_url,
                api_key_env,
                constrained_decoding,
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
            check_specialist_refs(&registry, &def, &spawningpool::store::tools_dir())?;
            registry.specialists.insert(name.clone(), def);
            format!("specialist {name}")
        }
        // Tools live as scripts in the folder, not the registry, so they're
        // handled separately and never reach the registry save below.
        DefineEntity::Tool { .. } => unreachable!("handled by define_tool before load"),
    };
    spawningpool::store::save(&registry)?;
    println!("defined {what}");
    Ok(())
}

/// Install a tool into the [`spawningpool::store::tools_dir`] folder as a symlink
/// to its script, so the tool's `# desc:`/`# params:` header stays live (editing
/// the script updates the tool) and the script can keep living in its own repo.
/// The script is validated as runnable now, and any existing entry for this tool
/// name is replaced.
fn define_tool(name: &str, script: &Path) -> Result<(), String> {
    if !spawningpool::tools::is_valid_tool_name(name) {
        return Err(format!(
            "'{name}' isn't a valid tool name; use letters, digits, '_' or '-' (max 64 chars)."
        ));
    }
    // Resolve to an absolute, runnable path now so the tool works regardless of
    // the directory `spawningpool run` is later invoked from, and so an un-runnable script
    // fails here with a fix rather than as a cryptic launch error mid-run.
    let script = resolve_script(script)?;
    let summary = spawningpool::summarize(&script).map_err(|e| e.to_string())?;
    if summary.desc.is_none() {
        eprintln!(
            "warning: tool '{name}' has no '# desc:' header, so the model will see an empty \
             description.\n  Add a line like '# desc: <what it does>' to {}.",
            script.display()
        );
    }

    let dir = spawningpool::store::tools_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    // Replace any existing entry for this name (a symlink or a stray `name.ext`)
    // so a redefine is idempotent and can't leave an ambiguous pair behind.
    spawningpool::tools::remove(&dir, name)?;
    let link = dir.join(name);
    std::os::unix::fs::symlink(&script, &link).map_err(|e| {
        format!(
            "failed to link {} -> {}: {e}",
            link.display(),
            script.display()
        )
    })?;
    println!("defined tool {name}");
    Ok(())
}

fn delete(entity: DeleteEntity) -> Result<(), String> {
    // Tools are files in the folder, not registry entries.
    if let DeleteEntity::Tool { name } = &entity {
        return delete_tool(name);
    }
    let mut registry = spawningpool::store::load()?;
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
        DeleteEntity::Tool { .. } => unreachable!("handled by delete_tool before load"),
    };
    if !removed {
        return Err(format!("no such {what}"));
    }
    spawningpool::store::save(&registry)?;
    println!("deleted {what}");
    warn_orphans(kind, &name, &referrers);
    Ok(())
}

/// Delete a tool by removing its script(s) from the folder, warning about any
/// specialists that still reference it (those references come from the registry).
fn delete_tool(name: &str) -> Result<(), String> {
    let registry = spawningpool::store::load()?;
    let referrers = referrers_of_tool(&registry, name);
    if !spawningpool::tools::remove(&spawningpool::store::tools_dir(), name)? {
        return Err(format!("no such tool {name}"));
    }
    println!("deleted tool {name}");
    warn_orphans("tool", name, &referrers);
    Ok(())
}

/// Render the registry's [`Referrer`]s as `kind 'name'` lines for orphan warnings.
fn format_referrers(referrers: Vec<Referrer>) -> Vec<String> {
    referrers
        .into_iter()
        .map(|r| format!("{} '{}'", r.kind, r.name))
        .collect()
}

fn referrers_of_provider(registry: &Registry, name: &str) -> Vec<String> {
    format_referrers(registry.referrers(EntityKind::Provider, name))
}

fn referrers_of_model(registry: &Registry, name: &str) -> Vec<String> {
    format_referrers(registry.referrers(EntityKind::Model, name))
}

fn referrers_of_tool(registry: &Registry, name: &str) -> Vec<String> {
    format_referrers(registry.referrers(EntityKind::Tool, name))
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
    raw.parse()
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

/// The folder's tool names, comma-joined, for the "Defined tools: ..." line in a
/// referential error. Mirrors [`available_names`] but reads the tools folder; an
/// unreadable folder is reported as none rather than failing the error message.
fn defined_tools(tools_dir: &Path) -> String {
    match spawningpool::tools::list(tools_dir) {
        Ok(names) if !names.is_empty() => names.join(", "),
        _ => "(none defined yet)".to_string(),
    }
}

/// Reject a model that names a provider the registry doesn't hold, naming the
/// provider, what's available, and how to define it.
fn check_model_refs(registry: &Registry, model: &ModelDef) -> Result<(), String> {
    if registry.missing_model_ref(model).is_some() {
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
                "      spawningpool define provider {} --api <anthropic-messages|openai-completions> --base-url <url>",
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
fn check_specialist_refs(
    registry: &Registry,
    specialist: &Specialist,
    tools_dir: &Path,
) -> Result<(), String> {
    let Some(missing) = registry.missing_specialist_ref(specialist, |name| {
        spawningpool::tools::exists(tools_dir, name)
    }) else {
        return Ok(());
    };
    let message = match missing.kind {
        EntityKind::Provider => [
            format!(
                "specialist '{}' references provider '{}', which isn't defined.",
                specialist.name, missing.name
            ),
            String::new(),
            format!("  Defined providers: {}", available_names(&registry.providers)),
            String::new(),
            "  Define it first:".to_string(),
            format!(
                "      spawningpool define provider {} --api <anthropic-messages|openai-completions> --base-url <url>",
                missing.name
            ),
            String::new(),
            "  ...or point the specialist at one that exists with --provider.".to_string(),
        ]
        .join("\n"),
        EntityKind::Model => [
            format!(
                "specialist '{}' references model '{}', which isn't defined.",
                specialist.name, missing.name
            ),
            String::new(),
            format!("  Defined models: {}", available_names(&registry.models)),
            String::new(),
            "  Define it first:".to_string(),
            format!(
                "      spawningpool define model {} --provider {} --max-tokens <n> --context-window <n>",
                missing.name, specialist.provider
            ),
            String::new(),
            "  ...or point the specialist at one that exists with --model.".to_string(),
        ]
        .join("\n"),
        EntityKind::Tool => [
            format!(
                "specialist '{}' references tool '{}', which isn't defined.",
                specialist.name, missing.name
            ),
            String::new(),
            format!("  Defined tools: {}", defined_tools(tools_dir)),
            String::new(),
            "  Back it with a script:".to_string(),
            format!("      spawningpool define tool {} --script <path>", missing.name),
        ]
        .join("\n"),
        EntityKind::Specialist => {
            unreachable!("a specialist does not reference other specialists")
        }
    };
    Err(message)
}

/// Resolve a tool script to an absolute path and confirm it can actually run:
/// it must exist and have the executable bit set. Storing it absolute means the
/// tool resolves no matter where `spawningpool run` is invoked from.
fn resolve_script(script: &Path) -> Result<PathBuf, String> {
    spawningpool::prepare_script(script).map_err(|e| match e {
        ScriptError::Unreadable { path, source } => format!(
            "tool script {} can't be read: {source}\n  Check the path is right and the file exists.",
            path.display()
        ),
        ScriptError::NotExecutable { path } => [
            format!("tool script {} isn't executable.", path.display()),
            String::new(),
            "  Make it runnable:".to_string(),
            format!("      chmod +x {}", path.display()),
            String::new(),
            "  (It also needs a shebang line, e.g. #!/bin/sh.)".to_string(),
        ]
        .join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests below that point `$SPAWNINGPOOL_REGISTRY` at a temp
    /// file, since that env var is process-wide and tests otherwise run parallel.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    fn restore_registry_env(saved: Option<std::ffi::OsString>) {
        match saved {
            Some(v) => std::env::set_var("SPAWNINGPOOL_REGISTRY", v),
            None => std::env::remove_var("SPAWNINGPOOL_REGISTRY"),
        }
    }

    #[test]
    fn define_list_show_and_delete_round_trip_through_the_store() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var_os("SPAWNINGPOOL_REGISTRY");
        let dir = std::env::temp_dir().join(format!("sp_cli_define_{}", std::process::id()));
        let path = dir.join("registry.json");
        std::env::set_var("SPAWNINGPOOL_REGISTRY", &path);

        define(DefineEntity::Provider {
            name: "anthropic".into(),
            api: "anthropic-messages".into(),
            base_url: "https://api.anthropic.com".into(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
            constrained_decoding: false,
        })
        .unwrap();

        // The provider is persisted and reloads from disk.
        assert!(spawningpool::store::load()
            .unwrap()
            .providers
            .contains_key("anthropic"));
        // Listing succeeds against the populated registry. Driven on a local
        // runtime rather than `#[tokio::test]` so the env-serializing guard is
        // never held across an await point.
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(list(ListKind::Providers))
            .unwrap();
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
        assert!(!spawningpool::store::load()
            .unwrap()
            .providers
            .contains_key("anthropic"));

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
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
                constrained_decoding: false,
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
        registry
    }

    /// A temp tools folder containing a single executable `ping` script, plus the
    /// folder path. The caller removes the folder when done.
    fn tools_dir_with_ping() -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "sp_cli_tools_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("ping");
        std::fs::write(&script, "#!/bin/sh\n# desc: Ping\necho hi\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        dir
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
        let dir = tools_dir_with_ping();
        let spec = specialist_ref("anthropic", "claude", vec!["ping".into()], None);
        let result = check_specialist_refs(&registry, &spec, &dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_ok());
    }

    #[test]
    fn check_specialist_refs_reports_missing_provider_model_and_tool() {
        let registry = populated_registry();
        let dir = tools_dir_with_ping();

        let err = check_specialist_refs(
            &registry,
            &specialist_ref("ghost", "claude", vec![], None),
            &dir,
        )
        .unwrap_err();
        assert!(err.contains("references provider 'ghost'"));
        assert!(err.contains("spawningpool define provider ghost"));

        let err = check_specialist_refs(
            &registry,
            &specialist_ref("anthropic", "nope", vec![], None),
            &dir,
        )
        .unwrap_err();
        assert!(err.contains("references model 'nope'"));
        assert!(err.contains("spawningpool define model nope"));

        let err = check_specialist_refs(
            &registry,
            &specialist_ref("anthropic", "claude", vec!["absent".into()], None),
            &dir,
        )
        .unwrap_err();
        std::fs::remove_dir_all(&dir).ok();
        assert!(err.contains("references tool 'absent'"));
        assert!(err.contains("spawningpool define tool absent"));
    }

    #[test]
    fn check_specialist_refs_validates_the_constrained_tool() {
        let registry = populated_registry();
        let dir = tools_dir_with_ping();
        // A constraint names a tool too, so an undefined forced tool is caught.
        let spec = specialist_ref("anthropic", "claude", vec![], Some("absent".into()));
        let err = check_specialist_refs(&registry, &spec, &dir).unwrap_err();
        std::fs::remove_dir_all(&dir).ok();
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
        assert!(err.contains("spawningpool define provider ghost"));
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
    fn onboarding_message_walks_the_progression() {
        // Empty registry: step 1, both provider examples.
        let empty = Registry::default();
        let msg = onboarding_message(&empty);
        assert!(msg.contains("[1/4]"));
        assert!(msg.contains("spawningpool define provider anthropic"));
        assert!(msg.contains("spawningpool define provider lmstudio"));

        // Provider only: step 2, points at the real provider.
        let mut reg = Registry::default();
        reg.providers.insert(
            "anthropic".into(),
            ProviderDef {
                name: "anthropic".into(),
                api: Api::AnthropicMessages,
                base_url: "https://api.anthropic.com".into(),
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
                constrained_decoding: false,
            },
        );
        let msg = onboarding_message(&reg);
        assert!(msg.contains("[2/4]"));
        assert!(msg.contains("spawningpool define model"));
        // Anthropic has no discovery endpoint, so don't offer --remote.
        assert!(!msg.contains("--remote"));

        // Model present: step 3, example uses the real provider/model.
        reg.models.insert(
            "claude".into(),
            ModelDef {
                id: "claude".into(),
                name: "Claude".into(),
                provider: "anthropic".into(),
                max_tokens: 1024,
                context_window: 200_000,
            },
        );
        let msg = onboarding_message(&reg);
        assert!(msg.contains("[3/4]"));
        assert!(msg.contains("--provider anthropic --model claude"));

        // Specialist present: step 4, run command names the real specialist.
        reg.specialists.insert(
            "summarizer".into(),
            specialist_ref("anthropic", "claude", vec![], None),
        );
        let msg = onboarding_message(&reg);
        assert!(msg.contains("[4/4]"));
        assert!(msg.contains("spawningpool run specialist spec"));
    }

    #[test]
    fn no_models_state_offers_discovery_for_openai_providers() {
        let mut reg = Registry::default();
        reg.providers.insert(
            "lmstudio".into(),
            ProviderDef {
                name: "lmstudio".into(),
                api: Api::OpenAiCompletions,
                base_url: "http://localhost:1234/v1".into(),
                api_key_env: None,
                constrained_decoding: false,
            },
        );
        let msg = onboarding_message(&reg);
        assert!(msg.contains("spawningpool list models --remote"));
    }

    #[test]
    fn unset_key_warnings_flags_only_missing_env_vars() {
        let reg = populated_registry();
        // The anthropic provider wants ANTHROPIC_API_KEY.
        let warnings = unset_key_warnings(&reg, |_| false);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ANTHROPIC_API_KEY"));
        // When it's set, nothing to warn about.
        assert!(unset_key_warnings(&reg, |_| true).is_empty());
    }

    #[test]
    fn progress_checks_completed_rungs() {
        assert!(progress(0).starts_with("  [1/4]"));
        assert!(progress(3).contains("specialist \u{2713}"));
        assert!(progress(3).starts_with("  [4/4]"));
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
}
