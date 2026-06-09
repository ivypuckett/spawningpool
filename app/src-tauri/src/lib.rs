mod commands;
#[cfg(test)]
mod test_support;
mod watch;

use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::registry::list_entities,
            commands::registry::show_entity
        ])
        .setup(|app| {
            let dir = match spawningpool::store::registry_path().parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => {
                    eprintln!(
                        "registry watcher: could not determine the registry directory from {}; watching '.'",
                        spawningpool::store::registry_path().display()
                    );
                    std::path::PathBuf::from(".")
                }
            };
            // Create the dir so the watcher can attach on a fresh install.
            std::fs::create_dir_all(&dir).ok();
            let handle = app.handle().clone();
            match watch::watch_dir(&dir, move || {
                let _ = handle.emit("registry-changed", ());
            }) {
                Ok(guard) => {
                    // Wrap in Mutex so the type is Send + Sync, required by manage().
                    app.manage(Mutex::new(guard));
                }
                Err(e) => eprintln!("registry watcher disabled: {e}"),
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
