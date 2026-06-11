//! The `spawningpool tui` front-end: a Ratatui terminal UI over the same registry the CLI
//! manages. The interesting logic lives in [`app`] (pure state) and [`render`]
//! (pure drawing); this module is the thin I/O shell — the terminal setup, the
//! event loop, and the handful of side effects ([`app::Action`]s) that can't be
//! pure: spawning `$EDITOR`, running a specialist.

mod app;
mod open;
mod render;

use std::collections::HashMap;
use std::io::{self, Write};
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::Rect;
use ratatui::Terminal;

use app::{Action, App, EditTarget, Tab};

type Tui = Terminal<CrosstermBackend<io::Stdout>>;

/// Launch the TUI: set up the terminal, run the event loop, and restore the
/// terminal on the way out (even if the loop errors).
pub async fn launch() -> Result<(), String> {
    let mut app = App::load()?;
    let mut terminal = setup().map_err(|e| format!("failed to start TUI: {e}"))?;
    let result = run_loop(&mut terminal, &mut app).await;
    let _ = teardown();
    result
}

fn setup() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn teardown() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

async fn run_loop(terminal: &mut Tui, app: &mut App) -> Result<(), String> {
    while !app.should_quit() {
        terminal
            .draw(|f| render::render(app, f))
            .map_err(|e| e.to_string())?;

        // A short poll keeps the loop responsive without busy-spinning.
        if event::poll(Duration::from_millis(200)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    on_click(app, terminal.get_frame().area(), m.column, m.row);
                }
                _ => {}
            }
        }

        if let Some(action) = app.take_action() {
            handle_action(terminal, app, action).await;
            // The action may have changed disk state (a save, an edit); reload
            // so the view reflects it.
            app.refresh();
        }
    }
    Ok(())
}

/// Route a left-click: onto the tab bar (switch tabs) or onto a list row
/// (select it).
fn on_click(app: &mut App, area: Rect, column: u16, row: u16) {
    let [tabs, _header, body, _footer] = render::layout(area);
    if row == tabs.y {
        if let Some(i) = tab_at_x(column) {
            app.click_tab(i);
        }
        return;
    }
    // The list sits inside a border, so its first row is body.y + 1.
    let inner_top = body.y + 1;
    let inner_bottom = body.bottom().saturating_sub(1);
    if row >= inner_top && row < inner_bottom {
        app.click_row((row - inner_top) as usize);
    }
}

/// Which tab title, if any, sits under column `x`. Mirrors the renderer's tab
/// layout: a leading space, then each `[Title]`/` Title ` chip (both
/// `title.len() + 2` wide) followed by one space.
fn tab_at_x(x: u16) -> Option<usize> {
    let mut start = 1u16; // leading space
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let width = tab.title().len() as u16 + 2;
        if x >= start && x < start + width {
            return Some(i);
        }
        start += width + 1; // chip + trailing space
    }
    None
}

/// Perform one side effect, reporting failures back into the app's status line.
async fn handle_action(terminal: &mut Tui, app: &mut App, action: Action) {
    let result = match action {
        Action::OpenSpecialist(name) => run_specialist_interactive(terminal, &name).await,
        Action::RunTool(name) => run_tool(terminal, &name),
        Action::Edit(target) => edit(terminal, target),
        Action::AddTool(name) => add_tool(terminal, &name),
    };
    if let Err(e) = result {
        app.set_status(e);
    }
}

/// Drop out of the alternate screen, run `f` against the normal terminal, then
/// restore the TUI. Used to host an inline editor or a specialist's streamed
/// output where the user reads/types normally.
fn suspended<T>(terminal: &mut Tui, f: impl FnOnce() -> T) -> io::Result<T> {
    teardown()?;
    let out = f();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;
    Ok(out)
}

/// "Chat" with a specialist: prompt for input on the normal terminal, then run
/// it (streaming output via the CLI's existing renderer) and wait for a key.
async fn run_specialist_interactive(terminal: &mut Tui, name: &str) -> Result<(), String> {
    // Leave the alternate screen so the prompt and streamed output render
    // normally; we re-enter afterwards.
    teardown().map_err(|e| e.to_string())?;
    let prompt = read_line(&format!("prompt for '{name}'> "));
    let outcome = match prompt {
        Some(prompt) if !prompt.trim().is_empty() => {
            // Reuse the CLI's full run-and-render path.
            crate::run_specialist(name, prompt.trim()).await
        }
        _ => Ok(()),
    };
    if let Err(e) = &outcome {
        eprintln!("error: {e}");
    }
    pause("\nPress Enter to return to the TUI…");
    enable_raw_mode().map_err(|e| e.to_string())?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture).map_err(|e| e.to_string())?;
    terminal.clear().map_err(|e| e.to_string())?;
    outcome
}

/// Run a tool's script directly (no arguments) and show its output.
fn run_tool(terminal: &mut Tui, name: &str) -> Result<(), String> {
    let dir = spawningpool::store::tools_dir();
    let tool = spawningpool::tools::resolve(&dir, name)?;
    suspended(terminal, || {
        println!("[tool {name}] running {}…\n", tool.script.display());
        match spawningpool::run_script(&tool.script, &HashMap::new()) {
            Ok(run) => {
                print!("{}", run.output);
                if !run.success {
                    eprintln!("\n[tool {name}] exited non-zero");
                }
            }
            Err(e) => eprintln!("[tool {name}] could not run: {e}"),
        }
        pause("\nPress Enter to return to the TUI…");
    })
    .map_err(|e| e.to_string())
}

/// Edit an entity. Tools are scripts edited in place (in a multiplexer pane when
/// available); registry entities are edited as JSON inline so the result can be
/// re-parsed and saved when the editor closes.
fn edit(terminal: &mut Tui, target: EditTarget) -> Result<(), String> {
    match target {
        EditTarget::Tool(name) => {
            let dir = spawningpool::store::tools_dir();
            let tool = spawningpool::tools::resolve(&dir, &name)?;
            let path = tool.script.to_string_lossy().to_string();
            launch_editor(terminal, &path)
        }
        other => edit_registry_entity(terminal, other),
    }
}

/// Round-trip a registry entity through `$EDITOR` as JSON: dump it to a temp
/// file, edit inline, then re-parse and save. A parse/validation failure leaves
/// the registry untouched and surfaces the reason.
fn edit_registry_entity(terminal: &mut Tui, target: EditTarget) -> Result<(), String> {
    // Load fresh so we serialize (and later re-save) the on-disk truth.
    let mut registry = spawningpool::store::load()?;
    let (key, json) = entity_json(&registry, &target)?;

    let path = std::env::temp_dir().join(format!(
        "sp-edit-{}-{}.json",
        key.replace(['/', ' '], "_"),
        std::process::id()
    ));
    std::fs::write(&path, json).map_err(|e| format!("failed to stage edit: {e}"))?;
    let path_str = path.to_string_lossy().to_string();

    // Force an inline editor (never a multiplexer pane): we must block until the
    // editor exits before re-reading the file. A detached pane would return
    // immediately, so we'd read back — and delete — the temp file before the
    // user ever edited it, losing the edit and showing them a blank buffer.
    let argv = open::inline_editor(&path_str, |k| std::env::var(k).ok());
    suspended(terminal, || spawn_wait(&argv))
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("editor failed: {e}"))?;

    let edited = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    std::fs::remove_file(&path).ok();
    apply_entity_json(&mut registry, &target, &key, &edited)?;
    spawningpool::store::save(&registry)
}

/// Serialize the targeted entity to pretty JSON, returning its current key too.
fn entity_json(
    registry: &spawningpool::Registry,
    target: &EditTarget,
) -> Result<(String, String), String> {
    match target {
        EditTarget::Provider(name) => registry
            .providers
            .get(name)
            .map(|d| (name.clone(), serde_json::to_string_pretty(d).unwrap()))
            .ok_or_else(|| format!("no such provider '{name}'")),
        EditTarget::Model(name) => registry
            .models
            .get(name)
            .map(|d| (name.clone(), serde_json::to_string_pretty(d).unwrap()))
            .ok_or_else(|| format!("no such model '{name}'")),
        EditTarget::Specialist(name) => registry
            .specialists
            .get(name)
            .map(|d| (name.clone(), serde_json::to_string_pretty(d).unwrap()))
            .ok_or_else(|| format!("no such specialist '{name}'")),
        EditTarget::Tool(_) => unreachable!("tools are edited as scripts"),
    }
}

/// Parse the edited JSON back into the registry, re-keying if the name changed.
fn apply_entity_json(
    registry: &mut spawningpool::Registry,
    target: &EditTarget,
    old_key: &str,
    json: &str,
) -> Result<(), String> {
    match target {
        EditTarget::Provider(_) => {
            let def: spawningpool::ProviderDef =
                serde_json::from_str(json).map_err(|e| format!("invalid provider JSON: {e}"))?;
            registry.providers.remove(old_key);
            registry.providers.insert(def.name.clone(), def);
        }
        EditTarget::Model(_) => {
            let def: spawningpool::ModelDef =
                serde_json::from_str(json).map_err(|e| format!("invalid model JSON: {e}"))?;
            registry.models.remove(old_key);
            registry.models.insert(def.id.clone(), def);
        }
        EditTarget::Specialist(_) => {
            let def: spawningpool::Specialist =
                serde_json::from_str(json).map_err(|e| format!("invalid specialist JSON: {e}"))?;
            def.validate()?;
            registry.specialists.remove(old_key);
            registry.specialists.insert(def.name.clone(), def);
        }
        EditTarget::Tool(_) => unreachable!("tools are edited as scripts"),
    }
    Ok(())
}

/// Scaffold a new tool: write an executable template script into the tools
/// folder, then open it for editing.
fn add_tool(terminal: &mut Tui, name: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = spawningpool::store::tools_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    let path = dir.join(name);
    if path.exists() {
        return Err(format!("a tool named '{name}' already exists"));
    }
    let template = format!(
        "#!/bin/sh\n# desc: {name} — describe what this tool does\n# params: \n\n\
         # Arguments arrive as environment variables named after each param.\n\
         echo \"hello from {name}\"\n"
    );
    std::fs::write(&path, template)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    launch_editor(terminal, &path.to_string_lossy())
}

/// Open a real file (a tool script) in the editor, in a multiplexer pane when
/// we're in one, otherwise inline.
fn launch_editor(terminal: &mut Tui, file: &str) -> Result<(), String> {
    let launch = open::editor_launch(file, |k| std::env::var(k).ok());
    if launch.inline {
        suspended(terminal, || spawn_wait(&launch.argv))
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("editor failed: {e}"))
    } else {
        spawn_wait(&launch.argv).map_err(|e| format!("couldn't open editor pane: {e}"))
    }
}

/// Run `argv` to completion, erroring on a non-zero exit.
fn spawn_wait(argv: &[String]) -> io::Result<()> {
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "command exited with status {status}"
        )))
    }
}

/// Print a prompt and read a line from stdin (used while suspended). Returns
/// `None` on EOF.
fn read_line(prompt: &str) -> Option<String> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line),
        Err(_) => None,
    }
}

/// Show `message` and block until the user presses Enter.
fn pause(message: &str) {
    print!("{message}");
    io::stdout().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use spawningpool::ai::{Api, Reasoning};
    use spawningpool::{ModelDef, ProviderDef, Registry, Specialist};

    /// A registry with one provider, one model, and one specialist, each keyed
    /// the way the app keys them (providers/specialists by name, models by id).
    fn registry() -> Registry {
        let mut registry = Registry::default();
        registry.providers.insert(
            "anthropic".into(),
            ProviderDef {
                name: "anthropic".into(),
                api: Api::AnthropicMessages,
                base_url: "https://api.anthropic.com".into(),
                api_key_env: Some("ANTHROPIC_API_KEY".into()),
                constrained_decoding: false,
            },
        );
        registry.models.insert(
            "claude-opus".into(),
            ModelDef {
                id: "claude-opus".into(),
                name: "Claude Opus".into(),
                provider: "anthropic".into(),
                max_tokens: 4096,
                context_window: 200_000,
            },
        );
        registry.specialists.insert(
            "classifier".into(),
            Specialist {
                name: "classifier".into(),
                provider: "anthropic".into(),
                model: "claude-opus".into(),
                system_prompt: "sort it".into(),
                tools: Vec::new(),
                constraint: None,
                reasoning: Reasoning::Off,
                stream: false,
            },
        );
        registry
    }

    /// Editing any registry entity must hand the editor that entity's populated
    /// JSON — never a blank buffer. Covers all three editable kinds.
    #[test]
    fn entity_json_serializes_each_kind_with_its_fields() {
        let reg = registry();

        let (key, json) = entity_json(&reg, &EditTarget::Provider("anthropic".into())).unwrap();
        assert_eq!(key, "anthropic");
        assert!(json.contains("\"name\": \"anthropic\""), "{json}");
        assert!(json.contains("base_url"), "{json}");

        let (key, json) = entity_json(&reg, &EditTarget::Model("claude-opus".into())).unwrap();
        assert_eq!(key, "claude-opus");
        assert!(json.contains("\"id\": \"claude-opus\""), "{json}");
        assert!(json.contains("context_window"), "{json}");

        let (key, json) = entity_json(&reg, &EditTarget::Specialist("classifier".into())).unwrap();
        assert_eq!(key, "classifier");
        assert!(json.contains("\"name\": \"classifier\""), "{json}");
        assert!(json.contains("system_prompt"), "{json}");
    }

    /// A missing entity is a reported error, not a blank edit.
    #[test]
    fn entity_json_reports_a_missing_entity() {
        let reg = registry();
        let err = entity_json(&reg, &EditTarget::Model("ghost".into())).unwrap_err();
        assert!(err.contains("no such model 'ghost'"), "{err}");
    }

    /// Applying edited JSON updates the entity and re-keys it when its identity
    /// field (name / id) changed, for each editable kind.
    #[test]
    fn apply_entity_json_round_trips_and_rekeys() {
        // Provider: round-trip an edited base_url under the same key.
        let mut reg = registry();
        let json = r#"{"name":"anthropic","api":"anthropic-messages","base_url":"https://example.test","api_key_env":"ANTHROPIC_API_KEY","constrained_decoding":false}"#;
        apply_entity_json(
            &mut reg,
            &EditTarget::Provider("anthropic".into()),
            "anthropic",
            json,
        )
        .unwrap();
        assert_eq!(reg.providers["anthropic"].base_url, "https://example.test");

        // Model: renaming the id re-keys the map.
        let json = r#"{"id":"claude-opus-4","name":"Claude Opus","provider":"anthropic","max_tokens":4096,"context_window":200000}"#;
        apply_entity_json(
            &mut reg,
            &EditTarget::Model("claude-opus".into()),
            "claude-opus",
            json,
        )
        .unwrap();
        assert!(!reg.models.contains_key("claude-opus"));
        assert!(reg.models.contains_key("claude-opus-4"));

        // Specialist: renaming the name re-keys the map.
        let json = r#"{"name":"sorter","provider":"anthropic","model":"claude-opus-4","system_prompt":"sort it","tools":[],"constraint":null,"reasoning":"off","stream":false}"#;
        apply_entity_json(
            &mut reg,
            &EditTarget::Specialist("classifier".into()),
            "classifier",
            json,
        )
        .unwrap();
        assert!(!reg.specialists.contains_key("classifier"));
        assert!(reg.specialists.contains_key("sorter"));
    }

    /// A specialist whose edited JSON is invalid (both tools and a constraint)
    /// is rejected, leaving the registry untouched.
    #[test]
    fn apply_entity_json_rejects_an_invalid_specialist() {
        let mut reg = registry();
        let json = r#"{"name":"classifier","provider":"anthropic","model":"claude-opus","system_prompt":"","tools":["a"],"constraint":"a","reasoning":"off","stream":false}"#;
        let err = apply_entity_json(
            &mut reg,
            &EditTarget::Specialist("classifier".into()),
            "classifier",
            json,
        )
        .unwrap_err();
        assert!(err.contains("tools and a constraint"), "{err}");
        // The original specialist is still intact.
        assert!(reg.specialists.contains_key("classifier"));
    }

    #[test]
    fn tab_hit_testing_matches_render_layout() {
        // Leading space at x=0 hits nothing.
        assert_eq!(tab_at_x(0), None);
        // "[Providers]" spans x=1..12.
        assert_eq!(tab_at_x(1), Some(0));
        assert_eq!(tab_at_x(11), Some(0));
        // The space between chips hits nothing.
        assert_eq!(tab_at_x(12), None);
        // "[Specialists]" starts at x=13.
        assert_eq!(tab_at_x(13), Some(1));
        // Far right is past the tools chip.
        assert_eq!(tab_at_x(200), None);
    }
}
