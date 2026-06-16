use spawningpool::ai::Client;

use crate::cli::ListKind;

pub(crate) async fn list(kind: ListKind) -> Result<(), String> {
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
