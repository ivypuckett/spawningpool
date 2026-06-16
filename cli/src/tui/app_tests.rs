//! Tests for [`super`]. Extracted from `app.rs` and included via
//! `#[path]` so they remain a child module with access to private items.

use super::*;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn provider(name: &str) -> ProviderDef {
    ProviderDef {
        name: name.to_string(),
        api: Api::AnthropicMessages,
        base_url: "https://api.anthropic.com".to_string(),
        api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
        constrained_decoding: false,
    }
}

fn model(id: &str, provider: &str) -> ModelDef {
    ModelDef {
        id: id.to_string(),
        name: id.to_string(),
        provider: provider.to_string(),
        max_tokens: 4096,
        context_window: 200_000,
    }
}

fn specialist(name: &str) -> Specialist {
    Specialist {
        name: name.to_string(),
        provider: "anthropic".to_string(),
        model: "claude".to_string(),
        system_prompt: String::new(),
        tools: Vec::new(),
        constraint: None,
        reasoning: Reasoning::Off,
        stream: false,
    }
}

/// A registry with two providers, two models under `anthropic`, and three
/// specialists, plus a couple of tool names. Persisted nowhere by default.
pub(crate) fn sample() -> App {
    let mut registry = Registry::default();
    registry
        .providers
        .insert("anthropic".into(), provider("anthropic"));
    registry
        .providers
        .insert("lmstudio".into(), provider("lmstudio"));
    registry
        .models
        .insert("claude-opus".into(), model("claude-opus", "anthropic"));
    registry
        .models
        .insert("claude-haiku".into(), model("claude-haiku", "anthropic"));
    for name in ["summarizer", "classifier", "router"] {
        registry.specialists.insert(name.into(), specialist(name));
    }
    App::new(
        registry,
        vec!["deploy".into(), "ping".into()],
        PathBuf::from("/dev/null/registry.json"),
        PathBuf::from("/dev/null/tools"),
    )
}

#[test]
fn defaults_to_specialists_tab_first_item() {
    let app = sample();
    assert_eq!(app.tab(), Tab::Specialists);
    assert_eq!(app.selected(), 0);
    assert_eq!(app.items(), vec!["classifier", "router", "summarizer"]);
    assert_eq!(app.current().as_deref(), Some("classifier"));
}

#[test]
fn tab_keys_switch_sections() {
    let mut app = sample();
    app.on_key(key('p'));
    assert_eq!(app.tab(), Tab::Providers);
    assert_eq!(app.items(), vec!["anthropic", "lmstudio"]);
    app.on_key(key('t'));
    assert_eq!(app.tab(), Tab::Tools);
    assert_eq!(app.items(), vec!["deploy", "ping"]);
    app.on_key(key('s'));
    assert_eq!(app.tab(), Tab::Specialists);
}

#[test]
fn vim_keys_move_within_bounds() {
    let mut app = sample();
    assert_eq!(app.selected(), 0);
    app.on_key(key('j'));
    assert_eq!(app.selected(), 1);
    app.on_key(key('j'));
    app.on_key(key('j')); // already at last (3 items), clamps.
    assert_eq!(app.selected(), 2);
    app.on_key(key('k'));
    assert_eq!(app.selected(), 1);
    // Up past the top clamps at 0.
    app.on_key(key('k'));
    app.on_key(key('k'));
    assert_eq!(app.selected(), 0);
}

#[test]
fn right_on_provider_drills_into_models_left_pops_back() {
    let mut app = sample();
    app.on_key(key('p'));
    // anthropic is first; drill into it.
    app.on_key(key('l'));
    assert_eq!(app.level(), Level::Models("anthropic".into()));
    assert_eq!(app.items(), vec!["claude-haiku", "claude-opus"]);
    assert_eq!(app.breadcrumb(), "Providers \u{203a} anthropic");
    // Left pops back to the provider list, cursor restored on anthropic.
    app.on_key(key('h'));
    assert_eq!(app.level(), Level::Providers);
    assert_eq!(app.current().as_deref(), Some("anthropic"));
    // Left at the root does nothing.
    app.on_key(key('h'));
    assert_eq!(app.level(), Level::Providers);
}

#[test]
fn right_on_provider_is_not_open() {
    // Drilling into a provider navigates; it must not emit a side-effect action.
    let mut app = sample();
    app.on_key(key('p'));
    app.on_key(key('l'));
    assert_eq!(app.take_action(), None);
}

#[test]
fn open_on_a_provider_drills_into_its_models() {
    // `o` on a provider drills into its models (like Enter/right), not an
    // action — a provider's base_url is an API endpoint, not a web page.
    let mut app = sample();
    app.on_key(key('p'));
    app.on_key(key('o'));
    assert_eq!(app.take_action(), None);
    assert_eq!(app.level(), Level::Models("anthropic".into()));
}

#[test]
fn open_specialist_runs_it() {
    let mut app = sample();
    app.on_key(key('o'));
    assert_eq!(
        app.take_action(),
        Some(Action::OpenSpecialist("classifier".into()))
    );
}

#[test]
fn right_on_leaf_opens_like_o() {
    let mut app = sample();
    app.on_key(code(KeyCode::Right));
    assert_eq!(
        app.take_action(),
        Some(Action::OpenSpecialist("classifier".into()))
    );
}

#[test]
fn edit_targets_the_right_kind() {
    let mut app = sample();
    app.on_key(key('e'));
    assert_eq!(
        app.take_action(),
        Some(Action::Edit(EditTarget::Specialist("classifier".into())))
    );
    app.on_key(key('p'));
    app.on_key(key('e'));
    assert_eq!(
        app.take_action(),
        Some(Action::Edit(EditTarget::Provider("anthropic".into())))
    );
}

#[test]
fn search_filters_live_and_enter_keeps_it() {
    let mut app = sample();
    app.on_key(key('/'));
    assert_eq!(app.mode(), &Mode::Search);
    for c in "rou".chars() {
        app.on_key(key(c));
    }
    assert_eq!(app.items(), vec!["router"]);
    // Enter returns to normal nav with the filter still applied.
    app.on_key(code(KeyCode::Enter));
    assert_eq!(app.mode(), &Mode::Normal);
    assert_eq!(app.items(), vec!["router"]);
    // Esc in normal clears the filter.
    app.on_key(code(KeyCode::Esc));
    assert_eq!(app.items(), vec!["classifier", "router", "summarizer"]);
}

#[test]
fn search_esc_abandons_filter() {
    let mut app = sample();
    app.on_key(key('/'));
    for c in "qq".chars() {
        app.on_key(key(c));
    }
    assert!(app.items().is_empty());
    app.on_key(code(KeyCode::Esc));
    assert_eq!(app.mode(), &Mode::Normal);
    assert_eq!(app.items().len(), 3);
}

#[test]
fn help_toggles_and_any_key_dismisses() {
    let mut app = sample();
    app.on_key(key('?'));
    assert_eq!(app.mode(), &Mode::Help);
    app.on_key(key('x'));
    assert_eq!(app.mode(), &Mode::Normal);
}

#[test]
fn quit_keys_set_quit() {
    for k in [key('q'), ctrl('c'), ctrl('d')] {
        let mut app = sample();
        app.on_key(k);
        assert!(app.should_quit());
    }
}

#[test]
fn rename_buffer_starts_from_current_name() {
    let mut app = sample();
    app.on_key(key('r'));
    assert_eq!(app.mode(), &Mode::Rename("classifier".into()));
    // Backspace edits the buffer.
    app.on_key(code(KeyCode::Backspace));
    assert_eq!(app.mode(), &Mode::Rename("classifie".into()));
}

#[test]
fn delete_flow_confirms_then_removes_in_temp_registry() {
    // Point at a real temp path so persist() can write.
    let dir = std::env::temp_dir().join(format!("sp_tui_del_{}", std::process::id()));
    let path = dir.join("registry.json");
    let mut app = sample();
    app.registry_path = path.clone();

    app.on_key(key('d'));
    assert_eq!(app.mode(), &Mode::ConfirmDelete);
    // 'n' cancels.
    app.on_key(key('n'));
    assert_eq!(app.mode(), &Mode::Normal);
    assert_eq!(app.items().len(), 3);
    // 'y' removes the selected specialist and saves.
    app.on_key(key('d'));
    app.on_key(key('y'));
    assert_eq!(app.items(), vec!["router", "summarizer"]);
    let saved = spawningpool::store::load_from(&path).unwrap();
    assert!(!saved.specialists.contains_key("classifier"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn add_tool_validates_name_and_emits_action() {
    let mut app = sample();
    app.on_key(key('t'));
    app.on_key(key('a'));
    for c in "scan".chars() {
        app.on_key(key(c));
    }
    app.on_key(code(KeyCode::Enter));
    assert_eq!(app.take_action(), Some(Action::AddTool("scan".into())));

    // An invalid name is rejected with a status, no action queued.
    app.on_key(key('a'));
    for c in "bad name".chars() {
        app.on_key(key(c));
    }
    app.on_key(code(KeyCode::Enter));
    assert_eq!(app.take_action(), None);
    assert!(app.status().unwrap().contains("valid tool name"));
}

#[test]
fn add_specialist_stub_persists_and_queues_edit() {
    let dir = std::env::temp_dir().join(format!("sp_tui_add_{}", std::process::id()));
    let path = dir.join("registry.json");
    let mut app = sample();
    app.registry_path = path.clone();

    app.on_key(key('a'));
    for c in "grader".chars() {
        app.on_key(key(c));
    }
    app.on_key(code(KeyCode::Enter));
    assert_eq!(
        app.take_action(),
        Some(Action::Edit(EditTarget::Specialist("grader".into())))
    );
    let saved = spawningpool::store::load_from(&path).unwrap();
    assert!(saved.specialists.contains_key("grader"));
    // The new stub is selected.
    assert_eq!(app.current().as_deref(), Some("grader"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rename_provider_repoints_referrers() {
    let dir = std::env::temp_dir().join(format!("sp_tui_ren_{}", std::process::id()));
    let path = dir.join("registry.json");
    let mut app = sample();
    app.registry_path = path.clone();
    app.on_key(key('p')); // providers, anthropic selected
    app.on_key(key('r'));
    // Replace the buffer wholesale.
    if let Mode::Rename(_) = app.mode() {
        app.mode = Mode::Rename("claude-co".into());
    }
    app.on_key(code(KeyCode::Enter));

    let saved = spawningpool::store::load_from(&path).unwrap();
    assert!(saved.providers.contains_key("claude-co"));
    assert!(!saved.providers.contains_key("anthropic"));
    // Models that pointed at anthropic now point at the new name.
    assert!(saved
        .models
        .values()
        .all(|m| m.provider == "claude-co" || m.provider == "lmstudio"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn mouse_click_selects_row_and_tab() {
    let mut app = sample();
    app.click_row(2);
    assert_eq!(app.selected(), 2);
    app.click_row(99); // ignored
    assert_eq!(app.selected(), 2);
    app.click_tab(2); // Tools is the third tab
    assert_eq!(app.tab(), Tab::Tools);
}

#[test]
fn switching_tabs_resets_drill_and_filter() {
    let mut app = sample();
    app.on_key(key('p'));
    app.on_key(key('l')); // drill into anthropic
    app.on_key(key('s')); // jump to specialists
    assert_eq!(app.level(), Level::Specialists);
    app.on_key(key('p')); // back to providers, no longer drilled
    assert_eq!(app.level(), Level::Providers);
}

// ---- bug-bash exploratory probes ------------------------------------
// These assert the behaviour a user would intuitively expect. Ones that
// fail are pointing at real bugs.

/// Control: drilling without a filter and popping back lands on the
/// provider you went into.
#[test]
fn drill_without_filter_pops_back_onto_the_drilled_provider() {
    let mut app = sample();
    app.on_key(key('p')); // providers: [anthropic, lmstudio]
    app.on_key(key('j')); // select lmstudio
    app.on_key(key('l')); // drill in
    app.on_key(key('h')); // pop back
    assert_eq!(app.current().as_deref(), Some("lmstudio"));
}

/// Regression: with a filter active, drilling into the only match and
/// popping back used to jump the cursor to the wrong provider — the parked
/// index was taken against the *filtered* list but restored against the full
/// one. Popping now restores by the drilled provider's name.
#[test]
fn drill_after_filtering_pops_back_onto_the_drilled_provider() {
    let mut app = sample();
    app.on_key(key('p')); // providers: [anthropic, lmstudio]
    app.on_key(key('/'));
    for c in "lm".chars() {
        app.on_key(key(c));
    }
    assert_eq!(app.items(), vec!["lmstudio"]);
    app.on_key(code(KeyCode::Enter)); // apply filter
    app.on_key(key('l')); // drill into lmstudio
    assert_eq!(app.level(), Level::Models("lmstudio".into()));
    app.on_key(key('h')); // pop back
    assert_eq!(app.level(), Level::Providers);
    assert_eq!(app.current().as_deref(), Some("lmstudio"));
}

/// Regression: adding an entity while a filter was active that excluded the
/// new name used to leave it hidden and the cursor stranded, even though it
/// was saved. Adding now clears the filter so the new entity is selected.
#[test]
fn adding_while_filtered_reveals_and_selects_the_new_entity() {
    let dir = std::env::temp_dir().join(format!("sp_tui_addf_{}", std::process::id()));
    let path = dir.join("registry.json");
    let mut app = sample();
    app.registry_path = path;
    // Filter specialists to "rou" (only "router").
    app.on_key(key('/'));
    for c in "rou".chars() {
        app.on_key(key(c));
    }
    app.on_key(code(KeyCode::Enter));
    // Add a specialist whose name doesn't match the filter.
    app.on_key(key('a'));
    for c in "grader".chars() {
        app.on_key(key(c));
    }
    app.on_key(code(KeyCode::Enter));
    assert_eq!(app.current().as_deref(), Some("grader"));
    std::fs::remove_dir_all(&dir).ok();
}

/// Control: the filter matches case-insensitively.
#[test]
fn filter_is_case_insensitive() {
    let mut app = sample();
    app.on_key(key('/'));
    for c in "ROU".chars() {
        app.on_key(key(c));
    }
    assert_eq!(app.items(), vec!["router"]);
}

/// Probe: renaming the sole filtered match to a name that still matches
/// keeps it selected.
#[test]
fn renaming_filtered_match_keeps_it_selected() {
    let dir = std::env::temp_dir().join(format!("sp_tui_renf_{}", std::process::id()));
    let path = dir.join("registry.json");
    let mut app = sample();
    app.registry_path = path;
    app.on_key(key('/'));
    for c in "rou".chars() {
        app.on_key(key(c));
    }
    app.on_key(code(KeyCode::Enter)); // filter -> [router]
    app.on_key(key('r'));
    if let Mode::Rename(_) = app.mode() {
        app.mode = Mode::Rename("routerz".into());
    }
    app.on_key(code(KeyCode::Enter));
    assert_eq!(app.current().as_deref(), Some("routerz"));
    std::fs::remove_dir_all(&dir).ok();
}

/// Control: opening a model queues an edit of that model.
#[test]
fn open_model_edits_it() {
    let mut app = sample();
    app.on_key(key('p'));
    app.on_key(key('l')); // drill into anthropic's models
    app.on_key(key('o')); // open the first model
    assert_eq!(
        app.take_action(),
        Some(Action::Edit(EditTarget::Model("claude-haiku".into())))
    );
}
