//! Tests for [`super`]. Extracted from `render.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

use super::super::app::tests::sample;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Render the buffer to newline-separated text — the ascii "screenshot"
/// used as proof of work. Trailing spaces are trimmed so captures stay
/// compact and diff cleanly.
fn buffer_to_string(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut row = String::new();
        for x in area.left()..area.right() {
            row.push_str(buf[(x, y)].symbol());
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

/// Draw `app` to an 64×16 test terminal and return the ascii screenshot.
fn screenshot(app: &App) -> String {
    let backend = TestBackend::new(64, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| render(app, f)).unwrap();
    buffer_to_string(terminal.backend().buffer())
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

#[test]
fn specialists_view_is_the_default() {
    let app = sample();
    let shot = screenshot(&app);
    // Active tab bracketed; the three specialists listed; selection gutter.
    assert!(shot.contains("[Specialists]"), "{shot}");
    assert!(shot.contains("> classifier"), "{shot}");
    assert!(shot.contains("router"));
    assert!(shot.contains("summarizer"));
    // Footer hints are always on (the row starts with the tab-switch keys).
    assert!(shot.contains("p/s/t"), "{shot}");
}

#[test]
fn drilling_into_a_provider_shows_its_models() {
    let mut app = sample();
    app.on_key(key('p'));
    app.on_key(key('l'));
    let shot = screenshot(&app);
    assert!(shot.contains("Providers \u{203a} anthropic"), "{shot}");
    assert!(shot.contains("claude-haiku"), "{shot}");
    assert!(shot.contains("claude-opus"), "{shot}");
}

#[test]
fn search_prompt_renders_filter() {
    let mut app = sample();
    app.on_key(key('/'));
    for c in "rou".chars() {
        app.on_key(key(c));
    }
    let shot = screenshot(&app);
    assert!(shot.contains("/rou"), "{shot}");
    assert!(shot.contains("> router"), "{shot}");
    assert!(!shot.contains("classifier"), "{shot}");
}

#[test]
fn delete_confirm_prompt_renders() {
    let mut app = sample();
    app.on_key(key('d'));
    let shot = screenshot(&app);
    assert!(shot.contains("delete 'classifier'? (y/n)"), "{shot}");
}

#[test]
fn help_overlay_renders_bindings() {
    let mut app = sample();
    app.on_key(key('?'));
    let shot = screenshot(&app);
    assert!(shot.contains("help"), "{shot}");
    assert!(shot.contains("edit in $EDITOR"), "{shot}");
}

#[test]
fn empty_level_shows_onboarding_hint() {
    use spawningpool::Registry;
    use std::path::PathBuf;
    let app = App::new(
        Registry::default(),
        Vec::new(),
        PathBuf::from("/dev/null/r.json"),
        PathBuf::from("/dev/null/tools"),
    );
    let shot = screenshot(&app);
    assert!(shot.contains("No specialists yet"), "{shot}");
}
