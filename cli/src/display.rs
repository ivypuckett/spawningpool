use std::collections::HashMap;
use std::path::Path;

use spawningpool::ai::Api;
use spawningpool::{ProviderDef, Registry};

/// Print a state-aware onboarding panel: where the user is in the
/// provider → model → specialist → run progression, the exact next command,
/// and any provider whose API-key env var isn't set.
pub(crate) fn status() -> Result<(), String> {
    let registry = spawningpool::store::load()?;
    println!("{}", onboarding_message(&registry));
    for warning in unset_key_warnings(&registry, |env| std::env::var_os(env).is_some()) {
        eprintln!("{warning}");
    }
    Ok(())
}

/// The progression's four rungs, with the completed ones checked, plus a
/// `[current/4]` marker. `done` is how many rungs are already satisfied.
pub(crate) fn progress(done: usize) -> String {
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
pub(crate) fn onboarding_message(registry: &Registry) -> String {
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
pub(crate) fn unset_key_warnings(
    registry: &Registry,
    is_set: impl Fn(&str) -> bool,
) -> Vec<String> {
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

/// Sorted, comma-joined keys of a registry section, for the "Defined X: ..."
/// line in a referential error. Names the empty case explicitly so the message
/// reads sensibly before anything is defined.
pub(crate) fn available_names<V>(map: &HashMap<String, V>) -> String {
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
pub(crate) fn defined_tools(tools_dir: &Path) -> String {
    match spawningpool::tools::list(tools_dir) {
        Ok(names) if !names.is_empty() => names.join(", "),
        _ => "(none defined yet)".to_string(),
    }
}
