use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::Write;

use spawningpool::ai::{Client, StopReason};
use spawningpool::{Registry, RunEvent};

use crate::cli::OutputFormat;

pub(crate) async fn run_specialist(
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
pub(crate) async fn run_workflow(name: &str, args: &[String]) -> Result<(), String> {
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
pub(crate) struct WorkflowClosure {
    pub(crate) workflows: HashMap<String, spawningpool::workflow::Workflow>,
    pub(crate) tools: BTreeSet<String>,
    pub(crate) specialists: BTreeSet<String>,
}

/// Load `name` and the transitive closure of workflows it can `run`. A `run` to
/// a missing workflow surfaces here (as an unknown workflow) rather than
/// mid-evaluation.
pub(crate) fn load_workflow_closure(
    name: &str,
    registry: &Registry,
) -> Result<WorkflowClosure, String> {
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
pub(crate) fn provider_keys(registry: &Registry) -> HashMap<String, String> {
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
pub(crate) fn warn_unset_keys(
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
pub(crate) fn run_tool(name: &str, args: &[String]) -> Result<(), String> {
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
