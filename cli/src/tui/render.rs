//! Rendering the [`App`] to a Ratatui frame. Pure: it reads app state and draws,
//! nothing more. The same [`layout`] split is shared with mouse hit-testing so a
//! click lands on the row the renderer drew.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::app::{App, Level, Mode, Tab};

/// Split the screen into the four stacked regions: tab bar, breadcrumb/input,
/// the list, and the keybinding footer. Shared by [`render`] and mouse routing.
pub fn layout(area: Rect) -> [Rect; 4] {
    let rows = Layout::vertical([
        Constraint::Length(1), // tab bar
        Constraint::Length(1), // breadcrumb / input line
        Constraint::Min(0),    // list
        Constraint::Length(1), // footer hints
    ])
    .split(area);
    [rows[0], rows[1], rows[2], rows[3]]
}

/// Draw the whole UI for the current app state.
pub fn render(app: &App, frame: &mut Frame) {
    let [tabs, header, body, footer] = layout(frame.area());

    render_tabs(app, frame, tabs);
    render_header(app, frame, header);
    render_list(app, frame, body);
    render_footer(app, frame, footer);

    if app.mode() == &Mode::Help {
        render_help(frame, frame.area());
    }
}

/// The tab bar, with the active tab bracketed so it reads in plain ascii too.
fn render_tabs(app: &App, frame: &mut Frame, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    for tab in Tab::ALL {
        let label = if tab == app.tab() {
            Span::styled(
                format!("[{}]", tab.title()),
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            )
        } else {
            Span::raw(format!(" {} ", tab.title()))
        };
        spans.push(label);
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The line under the tabs: a breadcrumb in normal mode, or the active input
/// prompt (search / rename / add / delete-confirm). Any status note is appended.
fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let text = match app.mode() {
        Mode::Search => format!("/{}", app.filter()),
        Mode::Rename(buf) => format!("rename \u{203a} {buf}"),
        Mode::Add(buf) => format!("add \u{203a} {buf}"),
        Mode::ConfirmDelete => {
            let name = app.current().unwrap_or_default();
            format!("delete '{name}'? (y/n)")
        }
        _ => {
            let mut line = app.breadcrumb();
            if !app.filter().is_empty() {
                line.push_str(&format!("  (filter: {})", app.filter()));
            }
            line
        }
    };
    let mut line = text;
    if let Some(status) = app.status() {
        line.push_str(&format!("   — {status}"));
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            line,
            Style::default().add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

/// The list body — or, when the current level is empty, its onboarding hint.
fn render_list(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(list_title(app));
    let items = app.items();
    if items.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(app.empty_hint()).style(Style::default().add_modifier(Modifier::DIM)),
            inner,
        );
        return;
    }

    let rows: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, name)| {
            // A leading `>` gutter marks the selection in plain-ascii captures,
            // where reverse-video styling doesn't show.
            let gutter = if i == app.selected() { "> " } else { "  " };
            let style = if i == app.selected() {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(format!("{gutter}{name}"))).style(style)
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.selected()));
    frame.render_stateful_widget(List::new(rows).block(block), area, &mut state);
}

/// What the bordered list frame is titled — the noun of the current level.
fn list_title(app: &App) -> String {
    match app.level() {
        Level::Providers => " providers ".to_string(),
        Level::Models(p) => format!(" models · {p} "),
        Level::Specialists => " specialists ".to_string(),
        Level::Tools => " tools ".to_string(),
    }
}

/// The always-on keybinding footer, zellij-style. Contents track the mode so the
/// hints stay relevant.
fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    let hints: &[(&str, &str)] = match app.mode() {
        Mode::Search => &[("type", "filter"), ("⏎", "apply"), ("esc", "cancel")],
        Mode::Rename(_) | Mode::Add(_) => &[("type", "name"), ("⏎", "ok"), ("esc", "cancel")],
        Mode::ConfirmDelete => &[("y", "delete"), ("n", "keep")],
        Mode::Help => &[("any", "close")],
        Mode::Normal => &[
            ("p/s/t", "tabs"),
            ("hjkl", "nav"),
            ("a", "add"),
            ("o", "open"),
            ("e", "edit"),
            ("r", "rename"),
            ("d", "del"),
            ("/", "search"),
            ("^r", "refresh"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };
    frame.render_widget(Paragraph::new(footer_line(hints)), area);
}

/// Build the footer's `key label` chips into one styled line.
fn footer_line(hints: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (k, label) in hints {
        spans.push(Span::styled(
            format!(" {k} "),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::raw(format!(" {label}  ")));
    }
    Line::from(spans)
}

/// The centered help popup listing every binding.
fn render_help(frame: &mut Frame, area: Rect) {
    let lines = [
        "Keys",
        "",
        "  p / s / t      providers · specialists · tools",
        "  h j k l ←↓↑→   navigate (left: back · right: into / open)",
        "  enter          into folder / open file",
        "  a              add",
        "  o              open (chat specialist · run tool · provider console)",
        "  e              edit in $EDITOR",
        "  d              delete (confirm y/n)",
        "  r              rename",
        "  /              search current view (enter applies)",
        "  ctrl+r         refresh from disk",
        "  ?              this help",
        "  q ctrl+c ^d    quit",
        "",
        "  press any key to close",
    ];
    let height = (lines.len() as u16 + 2).min(area.height);
    let width = 64.min(area.width);
    let popup = centered_rect(width, height, area);
    frame.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL).title(" help ");
    let text: Vec<Line> = lines.iter().map(|l| Line::from(*l)).collect();
    frame.render_widget(Paragraph::new(text).block(block), popup);
}

/// A `width`×`height` rect centered in `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
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
}
