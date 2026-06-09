//! Watches the registry directory and notifies on change, so the UI refreshes
//! when the registry file or a tool script is edited out-of-band (e.g. by `sp`).

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// Start watching `dir` (recursively) and call `on_change` after a burst of
/// filesystem events settles (debounced). The returned guard must be kept alive
/// for watching to continue; dropping it stops the watch.
pub fn watch_dir(
    dir: &Path,
    on_change: impl Fn() + Send + 'static,
) -> Result<Box<dyn std::any::Any + Send>, String> {
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();

    let mut watcher =
        RecommendedWatcher::new(tx, notify::Config::default()).map_err(|e| e.to_string())?;

    watcher
        .watch(dir, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    // Draining thread: coalesce bursts of events into a single on_change call
    // using the simple debounce: block on first event, then drain with a 200ms
    // timeout until quiet, then fire.
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Block waiting for the first event in a burst, then drain the rest.
            loop {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(_) => {}                                    // more events in the burst; keep draining
                    Err(mpsc::RecvTimeoutError::Timeout) => break, // burst over
                    Err(mpsc::RecvTimeoutError::Disconnected) => return, // watcher dropped
                }
            }
            on_change();
        }
    });

    Ok(Box::new(watcher))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn fires_on_change_after_a_write() {
        let dir = std::env::temp_dir().join(format!(
            "sp_watch_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let _guard = watch_dir(&dir, move || {
            c.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        // Give the watcher a moment to attach, then write.
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::write(dir.join("registry.json"), "{}").unwrap();

        // Poll up to ~3s for the debounced callback to fire (avoid flakiness).
        let mut fired = false;
        for _ in 0..30 {
            if count.load(Ordering::SeqCst) >= 1 {
                fired = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        std::fs::remove_dir_all(&dir).ok();
        assert!(fired, "watcher did not fire on a file write");
    }
}
