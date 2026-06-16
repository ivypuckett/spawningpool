use crate::cli::ShowEntity;

/// Print an entity's full definition as pretty JSON, or error if it is absent.
/// Plain serializable definitions never fail to render.
pub(crate) fn show(entity: ShowEntity) -> Result<(), String> {
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
