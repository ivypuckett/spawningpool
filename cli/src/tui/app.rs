//! The TUI's state machine: navigation, selection, search, and the modal
//! commands (add/edit/delete/rename/open). Deliberately pure — it owns an
//! in-memory [`Registry`] snapshot and turns key/mouse events into state
//! changes, never touching the terminal. Side effects that *can't* be pure
//! (spawning `$EDITOR`, running a specialist, opening a provider's console) are
//! emitted as an [`Action`] for the event loop to carry out, which is what keeps
//! the whole thing testable against a `TestBackend`.

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use spawningpool::ai::{Api, Reasoning};
use spawningpool::{ModelDef, ProviderDef, Registry, Specialist};

/// The three top-level tabs, in display (and `Tab`-cycle) order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Providers,
    Specialists,
    Tools,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Providers, Tab::Specialists, Tab::Tools];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Providers => "Providers",
            Tab::Specialists => "Specialists",
            Tab::Tools => "Tools",
        }
    }
}

/// Which list the cursor is currently in. Only providers nest (into their
/// models); specialists and tools are flat. Carries the data each command and
/// the renderer need without re-deriving it from the raw nav fields.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Level {
    Providers,
    /// Inside provider `0`, listing its models.
    Models(String),
    Specialists,
    Tools,
}

/// The modal state layered over the list. `Normal` is plain navigation; the
/// rest capture input (search/rename/add) or a yes/no (delete) or show help.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    /// Typing a live filter; the text lives in [`App::filter`].
    Search,
    /// Confirming deletion of the named, current selection.
    ConfirmDelete,
    /// Typing the new name for the current selection; holds the edit buffer.
    Rename(String),
    /// Typing the name of a new entity; holds the edit buffer.
    Add(String),
    Help,
}

/// A side effect the event loop performs on the app's behalf, because it can't
/// be done purely: anything that shells out or blocks. After running one the
/// loop reloads the app from disk so the change shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    /// Run a specialist against a prompt the loop will ask for ("chat").
    OpenSpecialist(String),
    /// Run a tool's script directly.
    RunTool(String),
    /// Open a provider's console (its `base_url`) in the browser.
    OpenProvider(String),
    /// Edit an entity in `$EDITOR`: a registry entity as JSON, or a tool's
    /// script in place.
    Edit(EditTarget),
    /// Create a new tool: scaffold an executable script then edit it.
    AddTool(String),
}

/// What an [`Action::Edit`] targets. Registry entities are edited as JSON;
/// tools are scripts edited in place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EditTarget {
    Provider(String),
    Model(String),
    Specialist(String),
    Tool(String),
}

/// The whole TUI state. Holds an in-memory [`Registry`] plus the tool names read
/// from disk, and the paths it persists through so edits survive a reload.
pub struct App {
    registry: Registry,
    tools: Vec<String>,
    tab: Tab,
    /// `Some(provider_name)` when drilled into a provider's models.
    drill: Option<String>,
    selected: usize,
    /// Provider-list cursor, parked while drilled into its models.
    parked_selected: usize,
    filter: String,
    mode: Mode,
    status: Option<String>,
    pending: Option<Action>,
    quit: bool,
    registry_path: PathBuf,
    tools_dir: PathBuf,
}

impl App {
    /// Build from an in-memory registry and tool list, persisting (when a
    /// command mutates) to the given paths. The default landing spot matches the
    /// spec: the Specialists tab, first item focused.
    pub fn new(
        registry: Registry,
        tools: Vec<String>,
        registry_path: PathBuf,
        tools_dir: PathBuf,
    ) -> Self {
        App {
            registry,
            tools,
            tab: Tab::Specialists,
            drill: None,
            selected: 0,
            parked_selected: 0,
            filter: String::new(),
            mode: Mode::Normal,
            status: None,
            pending: None,
            quit: false,
            registry_path,
            tools_dir,
        }
    }

    /// Load registry + tools from the store's resolved paths.
    pub fn load() -> Result<Self, String> {
        let registry = spawningpool::store::load()?;
        let tools_dir = spawningpool::store::tools_dir();
        let tools = spawningpool::tools::list(&tools_dir)?;
        Ok(App::new(
            registry,
            tools,
            spawningpool::store::registry_path(),
            tools_dir,
        ))
    }

    /// Re-read the registry and tools from disk, preserving the cursor as best
    /// it can. Used by `ctrl+r` and after every [`Action`] the loop runs.
    pub fn refresh(&mut self) {
        match spawningpool::store::load() {
            Ok(registry) => self.registry = registry,
            Err(e) => self.status = Some(e),
        }
        match spawningpool::tools::list(&self.tools_dir) {
            Ok(tools) => self.tools = tools,
            Err(e) => self.status = Some(e),
        }
        // A drilled-into provider may have vanished; fall back to its root.
        if let Some(name) = &self.drill {
            if !self.registry.providers.contains_key(name) {
                self.drill = None;
                self.selected = self.parked_selected;
            }
        }
        self.clamp_selection();
    }

    // ---- getters the renderer reads -------------------------------------

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Set the transient status line (used by the loop to report action errors).
    pub fn set_status(&mut self, message: String) {
        self.status = Some(message);
    }

    /// A provider's configured `base_url`, for the "open console" action.
    pub fn provider_base_url(&self, name: &str) -> Option<String> {
        self.registry
            .providers
            .get(name)
            .map(|p| p.base_url.clone())
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// The level the cursor is in right now, derived from tab + drill state.
    pub fn level(&self) -> Level {
        match self.tab {
            Tab::Providers => match &self.drill {
                Some(name) => Level::Models(name.clone()),
                None => Level::Providers,
            },
            Tab::Specialists => Level::Specialists,
            Tab::Tools => Level::Tools,
        }
    }

    /// The visible, filter-applied items at the current level, sorted.
    pub fn items(&self) -> Vec<String> {
        let all = match self.level() {
            Level::Providers => sorted_keys(self.registry.providers.keys()),
            Level::Models(provider) => {
                let mut ids: Vec<String> = self
                    .registry
                    .models
                    .values()
                    .filter(|m| m.provider == provider)
                    .map(|m| m.id.clone())
                    .collect();
                ids.sort();
                ids
            }
            Level::Specialists => sorted_keys(self.registry.specialists.keys()),
            Level::Tools => self.tools.clone(),
        };
        if self.filter.is_empty() {
            return all;
        }
        let needle = self.filter.to_lowercase();
        all.into_iter()
            .filter(|name| name.to_lowercase().contains(&needle))
            .collect()
    }

    /// The currently highlighted item's name, if the list is non-empty.
    pub fn current(&self) -> Option<String> {
        self.items().get(self.selected).cloned()
    }

    /// Breadcrumb trail for the header, e.g. `Providers › anthropic`.
    pub fn breadcrumb(&self) -> String {
        match self.level() {
            Level::Models(provider) => format!("Providers \u{203a} {provider}"),
            other => match other {
                Level::Providers => "Providers".to_string(),
                Level::Specialists => "Specialists".to_string(),
                Level::Tools => "Tools".to_string(),
                Level::Models(_) => unreachable!(),
            },
        }
    }

    /// The empty-state hint shown when the current list has no items, mirroring
    /// the CLI's onboarding nudges.
    pub fn empty_hint(&self) -> String {
        match self.level() {
            Level::Providers => {
                "No providers yet. Press 'a' to add one — the API your specialists talk to."
                    .to_string()
            }
            Level::Models(provider) => {
                format!("No models under '{provider}' yet. Press 'a' to add one.")
            }
            Level::Specialists => {
                "No specialists yet. Press 'a' to add a hyper-specific agent.".to_string()
            }
            Level::Tools => {
                "No tools yet. Press 'a' to scaffold an executable tool script.".to_string()
            }
        }
    }

    // ---- the action queue -----------------------------------------------

    /// Take the pending side effect, if any, for the loop to perform.
    pub fn take_action(&mut self) -> Option<Action> {
        self.pending.take()
    }

    // ---- input ----------------------------------------------------------

    /// Route a key press according to the current [`Mode`]. Pressing a key
    /// clears any transient status line first, so stale messages don't linger.
    pub fn on_key(&mut self, key: KeyEvent) {
        self.status = None;
        match self.mode.clone() {
            Mode::Normal => self.on_key_normal(key),
            Mode::Search => self.on_key_search(key),
            Mode::ConfirmDelete => self.on_key_confirm(key),
            Mode::Rename(buf) => self.on_key_rename(key, buf),
            Mode::Add(buf) => self.on_key_add(key, buf),
            Mode::Help => self.mode = Mode::Normal,
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('d') if ctrl => self.quit = true,
            KeyCode::Char('r') if ctrl => {
                self.refresh();
                self.status = Some("refreshed".to_string());
            }
            KeyCode::Char('p') => self.switch_tab(Tab::Providers),
            KeyCode::Char('s') => self.switch_tab(Tab::Specialists),
            KeyCode::Char('t') => self.switch_tab(Tab::Tools),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Char('j') | KeyCode::Down => self.move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.move_up(),
            KeyCode::Char('h') | KeyCode::Left => self.move_left(),
            KeyCode::Char('l') | KeyCode::Right => self.move_right(),
            KeyCode::Enter => self.move_right(),
            KeyCode::Char('a') => self.start_add(),
            KeyCode::Char('o') => self.open_current(),
            KeyCode::Char('e') => self.edit_current(),
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('d') => self.start_delete(),
            KeyCode::Esc => {
                // Esc clears an active filter, otherwise does nothing.
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.clamp_selection();
                }
            }
            _ => {}
        }
    }

    fn on_key_search(&mut self, key: KeyEvent) {
        match key.code {
            // Enter keeps the filter and returns focus to the (filtered) list.
            KeyCode::Enter => self.mode = Mode::Normal,
            // Esc abandons the search, clearing the filter.
            KeyCode::Esc => {
                self.filter.clear();
                self.mode = Mode::Normal;
                self.clamp_selection();
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.clamp_selection();
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.clamp_selection();
            }
            _ => {}
        }
    }

    fn on_key_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.delete_current(),
            _ => self.mode = Mode::Normal,
        }
    }

    fn on_key_rename(&mut self, key: KeyEvent, mut buf: String) {
        match key.code {
            KeyCode::Enter => self.commit_rename(buf),
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                buf.pop();
                self.mode = Mode::Rename(buf);
            }
            KeyCode::Char(c) => {
                buf.push(c);
                self.mode = Mode::Rename(buf);
            }
            _ => {}
        }
    }

    fn on_key_add(&mut self, key: KeyEvent, mut buf: String) {
        match key.code {
            KeyCode::Enter => self.commit_add(buf),
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                buf.pop();
                self.mode = Mode::Add(buf);
            }
            KeyCode::Char(c) => {
                buf.push(c);
                self.mode = Mode::Add(buf);
            }
            _ => {}
        }
    }

    // ---- mouse ----------------------------------------------------------

    /// Select the list row at viewport `row` (0-based within the list's inner
    /// area). Out-of-range clicks are ignored.
    pub fn click_row(&mut self, row: usize) {
        if row < self.items().len() {
            self.selected = row;
        }
    }

    /// Switch to the tab at `index` (a click on the tab bar).
    pub fn click_tab(&mut self, index: usize) {
        if let Some(tab) = Tab::ALL.get(index) {
            self.switch_tab(*tab);
        }
    }

    // ---- navigation -----------------------------------------------------

    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.drill = None;
        self.filter.clear();
        self.selected = 0;
        self.parked_selected = 0;
    }

    fn move_down(&mut self) {
        let len = self.items().len();
        if len > 0 && self.selected + 1 < len {
            self.selected += 1;
        }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Left pops out of a provider's models back to the provider list; at any
    /// root it does nothing.
    fn move_left(&mut self) {
        if let Level::Models(provider) = self.level() {
            self.drill = None;
            self.filter.clear();
            // Restore the cursor onto the provider we drilled into by name, not
            // a parked index: the index was taken against the (possibly
            // filtered) list at drill time and won't line up with the full list
            // we're popping back to.
            self.select_by_name(&provider);
        }
    }

    /// Right drills a provider into its models, or — on a leaf (model,
    /// specialist, tool) — opens it, exactly as `o` would.
    fn move_right(&mut self) {
        match self.level() {
            Level::Providers => {
                if let Some(name) = self.current() {
                    self.parked_selected = self.selected;
                    self.drill = Some(name);
                    self.selected = 0;
                    self.filter.clear();
                }
            }
            _ => self.open_current(),
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.items().len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    // ---- commands -------------------------------------------------------

    /// `o` / right-on-leaf: open the current item per its kind. A provider
    /// opens its console; a model is edited; a specialist runs; a tool runs.
    fn open_current(&mut self) {
        let Some(name) = self.current() else {
            return;
        };
        self.pending = Some(match self.level() {
            Level::Providers => Action::OpenProvider(name),
            Level::Models(_) => Action::Edit(EditTarget::Model(name)),
            Level::Specialists => Action::OpenSpecialist(name),
            Level::Tools => Action::RunTool(name),
        });
    }

    /// `e`: edit the current item in `$EDITOR`.
    fn edit_current(&mut self) {
        let Some(name) = self.current() else {
            return;
        };
        let target = match self.level() {
            Level::Providers => EditTarget::Provider(name),
            Level::Models(_) => EditTarget::Model(name),
            Level::Specialists => EditTarget::Specialist(name),
            Level::Tools => EditTarget::Tool(name),
        };
        self.pending = Some(Action::Edit(target));
    }

    fn start_add(&mut self) {
        self.mode = Mode::Add(String::new());
    }

    fn start_rename(&mut self) {
        if let Some(name) = self.current() {
            self.mode = Mode::Rename(name);
        }
    }

    fn start_delete(&mut self) {
        if self.current().is_some() {
            self.mode = Mode::ConfirmDelete;
        }
    }

    /// Create the named entity. Registry entities get a minimal valid stub
    /// inserted and saved, then the user is dropped into the editor to finish
    /// it; tools are scaffolded as a script by the loop.
    fn commit_add(&mut self, name: String) {
        self.mode = Mode::Normal;
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        // Drop any active filter so the entity we're about to create is visible
        // (and can be selected) rather than hidden behind a stale search.
        self.filter.clear();
        match self.level() {
            Level::Providers => {
                let def = ProviderDef {
                    name: name.clone(),
                    api: Api::AnthropicMessages,
                    base_url: String::new(),
                    api_key_env: None,
                    constrained_decoding: false,
                };
                self.registry.providers.insert(name.clone(), def);
                if self.persist() {
                    self.select_by_name(&name);
                    self.pending = Some(Action::Edit(EditTarget::Provider(name)));
                }
            }
            Level::Models(provider) => {
                let def = ModelDef {
                    id: name.clone(),
                    name: name.clone(),
                    provider,
                    max_tokens: 4096,
                    context_window: 200_000,
                };
                self.registry.models.insert(name.clone(), def);
                if self.persist() {
                    self.select_by_name(&name);
                    self.pending = Some(Action::Edit(EditTarget::Model(name)));
                }
            }
            Level::Specialists => {
                let def = Specialist {
                    name: name.clone(),
                    provider: String::new(),
                    model: String::new(),
                    system_prompt: String::new(),
                    tools: Vec::new(),
                    constraint: None,
                    reasoning: Reasoning::Off,
                    stream: false,
                };
                self.registry.specialists.insert(name.clone(), def);
                if self.persist() {
                    self.select_by_name(&name);
                    self.pending = Some(Action::Edit(EditTarget::Specialist(name)));
                }
            }
            Level::Tools => {
                if !spawningpool::tools::is_valid_tool_name(&name) {
                    self.status = Some(format!(
                        "'{name}' isn't a valid tool name (letters, digits, '_' or '-')."
                    ));
                    return;
                }
                self.pending = Some(Action::AddTool(name));
            }
        }
    }

    /// Rename the current selection to `to`, repointing the registry key. Tools
    /// rename their backing file, which the loop handles, so this only covers
    /// registry entities; an empty or unchanged name is a no-op.
    fn commit_rename(&mut self, to: String) {
        self.mode = Mode::Normal;
        let to = to.trim().to_string();
        let Some(from) = self.current() else {
            return;
        };
        if to.is_empty() || to == from {
            return;
        }
        match self.level() {
            Level::Providers => {
                if let Some(mut def) = self.registry.providers.remove(&from) {
                    def.name = to.clone();
                    // Repoint everything that referenced the old provider name.
                    for model in self.registry.models.values_mut() {
                        if model.provider == from {
                            model.provider = to.clone();
                        }
                    }
                    for spec in self.registry.specialists.values_mut() {
                        if spec.provider == from {
                            spec.provider = to.clone();
                        }
                    }
                    self.registry.providers.insert(to.clone(), def);
                    if self.persist() {
                        if self.drill.as_deref() == Some(&from) {
                            self.drill = Some(to.clone());
                        }
                        self.select_by_name(&to);
                    }
                }
            }
            Level::Models(_) => {
                if let Some(mut def) = self.registry.models.remove(&from) {
                    def.id = to.clone();
                    def.name = to.clone();
                    for spec in self.registry.specialists.values_mut() {
                        if spec.model == from {
                            spec.model = to.clone();
                        }
                    }
                    self.registry.models.insert(to.clone(), def);
                    if self.persist() {
                        self.select_by_name(&to);
                    }
                }
            }
            Level::Specialists => {
                if let Some(mut def) = self.registry.specialists.remove(&from) {
                    def.name = to.clone();
                    self.registry.specialists.insert(to.clone(), def);
                    if self.persist() {
                        self.select_by_name(&to);
                    }
                }
            }
            Level::Tools => {
                self.status = Some(
                    "renaming a tool: rename its script file in the tools folder.".to_string(),
                );
            }
        }
    }

    /// Delete the confirmed selection. Tools are files (handled by the loop on
    /// refresh path via direct removal here); registry entities are removed and
    /// the registry re-saved.
    fn delete_current(&mut self) {
        self.mode = Mode::Normal;
        let Some(name) = self.current() else {
            return;
        };
        match self.level() {
            Level::Providers => {
                self.registry.providers.remove(&name);
                self.persist();
            }
            Level::Models(_) => {
                self.registry.models.remove(&name);
                self.persist();
            }
            Level::Specialists => {
                self.registry.specialists.remove(&name);
                self.persist();
            }
            Level::Tools => match spawningpool::tools::remove(&self.tools_dir, &name) {
                Ok(_) => {
                    self.tools.retain(|t| t != &name);
                }
                Err(e) => self.status = Some(e),
            },
        }
        self.clamp_selection();
    }

    /// Move the cursor onto `name` if it's in the current (filtered) list.
    fn select_by_name(&mut self, name: &str) {
        if let Some(i) = self.items().iter().position(|n| n == name) {
            self.selected = i;
        }
    }

    /// Save the registry, surfacing any error in the status line. Returns
    /// whether the save succeeded so callers can gate follow-up actions.
    fn persist(&mut self) -> bool {
        match spawningpool::store::save_to(&self.registry_path, &self.registry) {
            Ok(()) => true,
            Err(e) => {
                self.status = Some(e);
                false
            }
        }
    }
}

/// Sorted clone of a set of `&String` keys.
fn sorted_keys<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut names: Vec<String> = keys.cloned().collect();
    names.sort();
    names
}

#[cfg(test)]
pub(crate) mod tests {
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
        // Drilling must not emit an open action — that distinction matters.
        let mut app = sample();
        app.on_key(key('p'));
        app.on_key(key('l'));
        assert_eq!(app.take_action(), None);
    }

    #[test]
    fn open_provider_emits_open_action() {
        let mut app = sample();
        app.on_key(key('p'));
        app.on_key(key('o'));
        assert_eq!(
            app.take_action(),
            Some(Action::OpenProvider("anthropic".into()))
        );
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
}
