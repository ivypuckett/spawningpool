use crate::cli::{ShowEntity, WorkflowFormat};

/// Print an entity's full definition as pretty JSON, or error if it is absent.
/// Plain serializable definitions never fail to render.
pub(crate) fn show(entity: ShowEntity) -> Result<(), String> {
    // Workflows are source files (not registry entries) and take a --format
    // flag, so they're handled apart from the registry-backed entities.
    if let ShowEntity::Workflow { name, format } = &entity {
        let dir = spawningpool::store::workflows_dir();
        let src = spawningpool::workflow::source(&dir, name)?;
        let out = match format {
            WorkflowFormat::Source => src,
            WorkflowFormat::Mermaid => {
                let workflow = spawningpool::workflow::parse(&src)
                    .map_err(|e| format!("workflow '{name}' failed to parse: {e}"))?;
                spawningpool::workflow::mermaid(&workflow)
            }
        };
        println!("{out}");
        return Ok(());
    }

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
        ShowEntity::Workflow { .. } => unreachable!("workflows are handled above"),
    };
    match found {
        Some(json) => {
            println!("{json}");
            Ok(())
        }
        None => Err(format!("no such {what}")),
    }
}
