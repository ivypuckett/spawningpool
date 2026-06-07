//! `sp` — the spawningpool CLI. Defines providers, models, experts, and tools
//! into a persisted [`Registry`], and runs an expert against a prompt.

mod store;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use spawningpool::ai::{Api, Client, ContentBlock, Reasoning};
use spawningpool::{Expert, ModelDef, ProviderDef, ToolDef};

#[derive(Parser)]
#[command(name = "sp", bin_name = "spawningpool", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run an expert against a prompt.
    #[command(alias = "spawn")]
    Run {
        #[arg(long)]
        expert: String,
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
    Experts,
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
    /// Define an expert template.
    Expert {
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
        /// A tool the expert is forced to call (constrained decoding).
        #[arg(long)]
        constraint: Option<String>,
        #[arg(long, default_value = "off")]
        reasoning: String,
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
    Expert { name: String },
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
        Command::Run { expert, prompt } => run_expert(&expert, &prompt).await,
        Command::List { kind } => list(kind),
        Command::Define { entity } => define(entity),
        Command::Delete { entity } => delete(entity),
    }
}

async fn run_expert(name: &str, prompt: &str) -> Result<(), String> {
    let registry = store::load()?;
    let expert = registry
        .experts
        .get(name)
        .ok_or_else(|| format!("unknown expert: {name}"))?;

    let model = registry.resolve_model(expert)?;
    let ctx = registry.build_context(expert, prompt)?;
    let mut opts = expert.complete_options();
    // Source the API key from the provider's configured env var, if any.
    if let Some(env) = registry
        .providers
        .get(&expert.provider)
        .and_then(|p| p.api_key_env.as_ref())
    {
        if let Ok(key) = std::env::var(env) {
            opts.api_key = Some(key);
        }
    }

    let completion = Client::new()
        .complete(&model, &ctx, &opts)
        .await
        .map_err(|e| e.to_string())?;

    for block in &completion.message.content {
        match block {
            ContentBlock::Text { text } => println!("{text}"),
            ContentBlock::ToolCall {
                name, arguments, ..
            } => println!("[tool-call] {name} {arguments}"),
            ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } => {}
        }
    }
    eprintln!(
        "[usage] {} in / {} out",
        completion.usage.input, completion.usage.output
    );
    Ok(())
}

fn list(kind: ListKind) -> Result<(), String> {
    let registry = store::load()?;
    let mut names: Vec<&String> = match kind {
        ListKind::Experts => registry.experts.keys().collect(),
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
        DefineEntity::Expert {
            name,
            provider,
            model,
            system_prompt,
            tools,
            constraint,
            reasoning,
        } => {
            let def = Expert {
                name: name.clone(),
                provider,
                model,
                system_prompt,
                tools: parse_list(tools),
                constraint,
                reasoning: parse_reasoning(&reasoning)?,
            };
            registry.experts.insert(name.clone(), def);
            format!("expert {name}")
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
        DeleteEntity::Expert { name } => (
            registry.experts.remove(&name).is_some(),
            format!("expert {name}"),
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
}
