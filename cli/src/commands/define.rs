use std::path::{Path, PathBuf};

use spawningpool::ai::{Api, Reasoning};
use spawningpool::{EntityKind, ModelDef, ProviderDef, Registry, ScriptError, Specialist};

use crate::cli::DefineEntity;
use crate::display::{available_names, defined_tools};

pub(crate) fn define(entity: DefineEntity) -> Result<(), String> {
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

/// Split a comma-separated list flag into trimmed, non-empty names.
pub(crate) fn parse_list(raw: Option<String>) -> Vec<String> {
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

pub(crate) fn parse_reasoning(raw: &str) -> Result<Reasoning, String> {
    raw.parse()
}

/// Reject a model that names a provider the registry doesn't hold, naming the
/// provider, what's available, and how to define it.
pub(crate) fn check_model_refs(registry: &Registry, model: &ModelDef) -> Result<(), String> {
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
pub(crate) fn check_specialist_refs(
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
pub(crate) fn resolve_script(script: &Path) -> Result<PathBuf, String> {
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
