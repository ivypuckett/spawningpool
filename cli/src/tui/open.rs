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

/// The command to open `url` in the platform's default handler. macOS uses
/// `open`; elsewhere `xdg-open`.
pub fn open_url_command(url: &str) -> Vec<String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    vec![opener.to_string(), url.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn plain_terminal_runs_editor_inline() {
        let launch = editor_launch("/tmp/x.json", env_of(&[("EDITOR", "nvim")]));
        assert_eq!(launch.argv, vec!["nvim", "/tmp/x.json"]);
        assert!(launch.inline);
    }

    #[test]
    fn visual_wins_over_editor_and_default() {
        let launch = editor_launch("/tmp/x", env_of(&[("VISUAL", "code"), ("EDITOR", "vi")]));
        assert_eq!(launch.argv[0], "code");
        // Nothing set at all falls back to vi.
        let bare = editor_launch("/tmp/x", env_of(&[]));
        assert_eq!(bare.argv[0], "vi");
    }

    #[test]
    fn zellij_opens_a_new_pane_not_inline() {
        let launch = editor_launch("/tmp/x", env_of(&[("ZELLIJ", "0"), ("EDITOR", "hx")]));
        assert!(!launch.inline);
        assert_eq!(launch.argv[0], "zellij");
        assert!(launch.argv.contains(&"new-pane".to_string()));
        assert_eq!(launch.argv.last().unwrap(), "/tmp/x");
    }

    #[test]
    fn tmux_preferred_after_zellij() {
        // With both set, Zellij wins.
        let both = editor_launch("/f", env_of(&[("ZELLIJ", "0"), ("TMUX", "/tmp/t")]));
        assert_eq!(both.argv[0], "zellij");
        // tmux alone splits a window.
        let launch = editor_launch("/f", env_of(&[("TMUX", "/tmp/t"), ("EDITOR", "vim")]));
        assert_eq!(launch.argv, vec!["tmux", "split-window", "vim", "/f"]);
    }

    #[test]
    fn kitty_launches_a_window() {
        let launch = editor_launch("/f", env_of(&[("KITTY_WINDOW_ID", "3"), ("EDITOR", "vim")]));
        assert_eq!(launch.argv[0], "kitty");
        assert!(launch.argv.contains(&"launch".to_string()));
        assert!(!launch.inline);
    }

    #[test]
    fn inline_editor_ignores_multiplexers() {
        // Even inside a multiplexer, an inline edit must run the editor directly
        // (the caller blocks on it), never a detached pane.
        let argv = inline_editor(
            "/tmp/sp-edit.json",
            env_of(&[("ZELLIJ", "0"), ("TMUX", "/tmp/t"), ("EDITOR", "hx")]),
        );
        assert_eq!(argv, vec!["hx", "/tmp/sp-edit.json"]);
        // Falls back through VISUAL -> EDITOR -> vi just like editor_launch.
        let bare = inline_editor("/f", env_of(&[]));
        assert_eq!(bare, vec!["vi", "/f"]);
    }
}
