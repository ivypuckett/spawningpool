//! Tests for [`super`]. Extracted from `open.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

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
