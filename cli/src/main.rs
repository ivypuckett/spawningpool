//! The `spawningpool` CLI. Defines providers, models, specialists, and tools
//! into a persisted [`Registry`], and runs a specialist against a prompt.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use spawningpool::ai::{Api, Client, Reasoning, StopReason};
use spawningpool::{
    EntityKind, ModelDef, ProviderDef, Referrer, Registry, RunEvent, ScriptError, Specialist,
};
use std::collections::{BTreeSet, HashMap, VecDeque};
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
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
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
    #[command(alias = "overseer")]
    Workflow {
        name: String,
        /// A workflow input, as `KEY=VALUE`, matching a `# inputs:` entry.
        /// Repeatable.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        args: Vec<String>,
    },
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
            RunTarget::Workflow { name, args } => run_workflow(&name, &args).await,
            RunTarget::Tool { name, args } => run_tool(&name, &args),
        },
        Some(Command::List { kind }) => list(kind).await,
        Some(Command::Show { entity }) => show(entity),
        Some(Command::Define { entity }) => define(entity),
        Some(Command::Delete { entity, yes }) => delete(entity, yes),
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
/// the tool catalog, type-check, then evaluate. `args` are `KEY=VALUE` values
/// for the workflow's declared `# inputs:`. Prints the workflow's result value
/// as JSON, and — like a tool — also writes it to `$SP_OUTPUT_PATH` when that is
/// set, so a workflow obeys the same I/O contract and is composable as a tool.
async fn run_workflow(name: &str, args: &[String]) -> Result<(), String> {
    let registry = spawningpool::store::load()?;

    // Load the root and the transitive closure of workflows it can `run`, plus
    // the union of every tool and specialist referenced across that closure.
    let closure = load_workflow_closure(name, &registry)?;
    let workflows = &closure.workflows;
    let root = workflows
        .get(name)
        .expect("the closure always contains the root workflow");

    // Coerce the supplied --arg values to the root's declared input types before
    // anything runs, so a missing or mistyped input fails up front.
    let provided = parse_kv_args(args)?;
    let inputs = spawningpool::workflow::resolve_inputs(&root.inputs, &provided)
        .map_err(|e| format!("workflow '{name}': {e}"))?;

    // Resolve exactly the tools the closure references — `call` tools plus the
    // tools its specialists need — so an unrelated broken or ambiguous tool
    // elsewhere in the catalog can't block a workflow that doesn't use it.
    let tools_dir = spawningpool::store::tools_dir();
    let tool_names: Vec<String> = closure.tools.iter().cloned().collect();
    let tools = spawningpool::tools::resolve_all(&tools_dir, &tool_names)?;

    spawningpool::workflow::check(root, &registry, &tools, workflows)
        .map_err(|e| format!("workflow '{name}' failed type-checking: {e}"))?;

    let keys = provider_keys(&registry);
    warn_unset_keys(&closure.specialists, &registry, &keys);
    let client = Client::new();
    let result =
        spawningpool::workflow::eval(root, &registry, &tools, &client, &keys, &inputs, workflows)
            .await
            .map_err(|e| format!("workflow '{name}' failed: {e}"))?;

    // GHA-style output: when invoked with $SP_OUTPUT_PATH set (e.g. as another
    // runner's tool), write the result there so it composes like a tool.
    if let Some(path) = std::env::var_os("SP_OUTPUT_PATH") {
        std::fs::write(&path, result.to_string())
            .map_err(|e| format!("workflow '{name}': can't write $SP_OUTPUT_PATH: {e}"))?;
    }
    println!("{result}");
    Ok(())
}

/// A root workflow and everything reachable from it through `run`: the name→AST
/// map (always including the root), and the union of tool and specialist names
/// referenced anywhere in the closure.
struct WorkflowClosure {
    workflows: HashMap<String, spawningpool::workflow::Workflow>,
    tools: BTreeSet<String>,
    specialists: BTreeSet<String>,
}

/// Load `name` and the transitive closure of workflows it can `run`. A `run` to
/// a missing workflow surfaces here (as an unknown workflow) rather than
/// mid-evaluation.
fn load_workflow_closure(name: &str, registry: &Registry) -> Result<WorkflowClosure, String> {
    let dir = spawningpool::store::workflows_dir();
    let mut closure = WorkflowClosure {
        workflows: HashMap::new(),
        tools: BTreeSet::new(),
        specialists: BTreeSet::new(),
    };
    let mut queue: VecDeque<String> = VecDeque::from([name.to_string()]);

    while let Some(wf_name) = queue.pop_front() {
        if closure.workflows.contains_key(&wf_name) {
            continue;
        }
        let source = spawningpool::workflow::source(&dir, &wf_name)?;
        let workflow = spawningpool::workflow::parse(&source)
            .map_err(|e| format!("workflow '{wf_name}' is invalid: {e}"))?;
        let refs = spawningpool::workflow::referenced(&workflow, registry);
        closure.tools.extend(refs.tools);
        closure.specialists.extend(refs.specialists);
        for nested in refs.workflows {
            if !closure.workflows.contains_key(&nested) {
                queue.push_back(nested);
            }
        }
        closure.workflows.insert(wf_name, workflow);
    }

    Ok(closure)
}

/// Parse repeated `KEY=VALUE` flags into a map, erroring on a token without `=`.
fn parse_kv_args(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    for arg in args {
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("invalid --arg '{arg}': expected KEY=VALUE"))?;
        map.insert(key.to_string(), value.to_string());
    }
    Ok(map)
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

fn delete(entity: DeleteEntity, yes: bool) -> Result<(), String> {
    // Tools are files in the folder, not registry entries.
    if let DeleteEntity::Tool { name } = &entity {
        return delete_tool(name, yes);
    }
    let mut registry = spawningpool::store::load()?;
    // Look up the target and any entities that reference it *before* touching
    // anything, so we can preview the references this delete would orphan and
    // confirm before committing.
    let (exists, what, kind, name, referrers) = match &entity {
        DeleteEntity::Specialist { name } => (
            registry.specialists.contains_key(name),
            format!("specialist {name}"),
            "specialist",
            name.clone(),
            Vec::new(),
        ),
        DeleteEntity::Provider { name } => (
            registry.providers.contains_key(name),
            format!("provider {name}"),
            "provider",
            name.clone(),
            referrers_of_provider(&registry, name),
        ),
        DeleteEntity::Model { name } => (
            registry.models.contains_key(name),
            format!("model {name}"),
            "model",
            name.clone(),
            referrers_of_model(&registry, name),
        ),
        DeleteEntity::Tool { .. } => unreachable!("handled by delete_tool before load"),
    };
    if !exists {
        return Err(format!("no such {what}"));
    }
    if !confirm_delete(&what, kind, &name, &referrers, yes)? {
        return Ok(());
    }
    match entity {
        DeleteEntity::Specialist { name } => {
            registry.specialists.remove(&name);
        }
        DeleteEntity::Provider { name } => {
            registry.providers.remove(&name);
        }
        DeleteEntity::Model { name } => {
            registry.models.remove(&name);
        }
        DeleteEntity::Tool { .. } => unreachable!("handled by delete_tool before load"),
    }
    spawningpool::store::save(&registry)?;
    println!("deleted {what}");
    Ok(())
}

/// Delete a tool by removing its script(s) from the folder. Warns about — and
/// confirms past — any specialists that still reference it (those references
/// come from the registry).
fn delete_tool(name: &str, yes: bool) -> Result<(), String> {
    let registry = spawningpool::store::load()?;
    let dir = spawningpool::store::tools_dir();
    if !spawningpool::tools::exists(&dir, name) {
        return Err(format!("no such tool {name}"));
    }
    let referrers = referrers_of_tool(&registry, name);
    if !confirm_delete(&format!("tool {name}"), "tool", name, &referrers, yes)? {
        return Ok(());
    }
    spawningpool::tools::remove(&dir, name)?;
    println!("deleted tool {name}");
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

/// Preview the references a delete would orphan, then — unless `yes` — ask for
/// confirmation on stdin. Returns whether the delete should proceed; a declined
/// prompt prints `cancelled` and returns `false`.
fn confirm_delete(
    what: &str,
    kind: &str,
    name: &str,
    referrers: &[String],
    yes: bool,
) -> Result<bool, String> {
    if !referrers.is_empty() {
        for referrer in referrers {
            eprintln!("warning: deleting {kind} '{name}' will orphan {referrer}");
        }
        eprintln!(
            "  Those references will fail at run time until you redefine {kind} '{name}' or repoint them."
        );
    }
    if yes {
        return Ok(true);
    }
    if prompt_yes_no(&format!("delete {what}?"))? {
        Ok(true)
    } else {
        println!("cancelled");
        Ok(false)
    }
}

/// Ask a yes/no question on stdin, defaulting to no: only an explicit `y`/`yes`
/// proceeds. EOF (a non-interactive stdin) reads as no.
fn prompt_yes_no(question: &str) -> Result<bool, String> {
    use std::io::Write;
    print!("{question} [y/N] ");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    Ok(affirmative(&answer))
}

/// Whether a prompt answer is an explicit yes. Anything else — including the
/// empty string EOF yields on a non-interactive stdin — is a no.
fn affirmative(answer: &str) -> bool {
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
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
#[path = "main_tests.rs"]
mod tests;
