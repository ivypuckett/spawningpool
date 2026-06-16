//! Deciding *how* to open a file in the user's editor.
//!
//! Per the spec: open with `$EDITOR`, but prefer a new pane in whatever
//! multiplexer/terminal we're inside — Zellij, then tmux, then Kitty — falling
//! back to running the editor inline (suspending the TUI) when we're in a plain
//! terminal. The decision is a pure function of the environment so it can be
//! tested without spawning anything.

/// How to launch the editor: the argv to run, and whether it must run *inline*
/// (taking over our terminal, so the TUI has to be suspended around it). A pane
/// launched in a multiplexer is not inline — it opens beside us.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EditorLaunch {
    pub argv: Vec<String>,
    pub inline: bool,
}

/// Resolve the editor command for `file`, choosing a multiplexer pane when we're
/// inside one. `env` looks up an environment variable (injected for testing).
pub fn editor_launch(file: &str, env: impl Fn(&str) -> Option<String>) -> EditorLaunch {
    let editor = resolve_editor(&env);

    // Zellij first, then tmux, then Kitty — each opens a fresh pane so the TUI
    // keeps running. A plain terminal runs the editor inline.
    if env("ZELLIJ").is_some() {
        EditorLaunch {
            argv: vec![
                "zellij".into(),
                "action".into(),
                "new-pane".into(),
                "--close-on-exit".into(),
                "--".into(),
                editor,
                file.to_string(),
            ],
            inline: false,
        }
    } else if env("TMUX").is_some() {
        EditorLaunch {
            argv: vec![
                "tmux".into(),
                "split-window".into(),
                editor,
                file.to_string(),
            ],
            inline: false,
        }
    } else if env("KITTY_WINDOW_ID").is_some() {
        EditorLaunch {
            argv: vec![
                "kitty".into(),
                "@".into(),
                "launch".into(),
                "--type=window".into(),
                editor,
                file.to_string(),
            ],
            inline: false,
        }
    } else {
        EditorLaunch {
            argv: vec![editor, file.to_string()],
            inline: true,
        }
    }
}

/// The editor command name: `$VISUAL`, then `$EDITOR`, then `vi`.
fn resolve_editor(env: &impl Fn(&str) -> Option<String>) -> String {
    env("VISUAL")
        .or_else(|| env("EDITOR"))
        .unwrap_or_else(|| "vi".to_string())
}

/// The argv to run the user's editor *inline* — taking over this terminal,
/// never a multiplexer pane. Round-trip JSON edits need this: the caller must
/// block until the editor exits before re-reading the file, which a detached
/// pane (the command returns immediately) would defeat — the temp file gets
/// read back and deleted out from under the still-opening editor, so the edit
/// is lost and the editor shows a blank buffer.
pub fn inline_editor(file: &str, env: impl Fn(&str) -> Option<String>) -> Vec<String> {
    vec![resolve_editor(&env), file.to_string()]
}

#[cfg(test)]
#[path = "open_tests.rs"]
mod tests;
