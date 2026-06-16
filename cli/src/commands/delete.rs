use spawningpool::{EntityKind, Referrer, Registry};

use crate::cli::DeleteEntity;

pub(crate) fn delete(entity: DeleteEntity, yes: bool) -> Result<(), String> {
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

pub(crate) fn referrers_of_provider(registry: &Registry, name: &str) -> Vec<String> {
    format_referrers(registry.referrers(EntityKind::Provider, name))
}

pub(crate) fn referrers_of_model(registry: &Registry, name: &str) -> Vec<String> {
    format_referrers(registry.referrers(EntityKind::Model, name))
}

pub(crate) fn referrers_of_tool(registry: &Registry, name: &str) -> Vec<String> {
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
pub(crate) fn affirmative(answer: &str) -> bool {
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}
