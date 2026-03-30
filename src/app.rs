use crate::catalog::Entry;
use crate::demo;
use crate::enrichment::{EnrichmentResult, enrich_entry};
use crate::search::{ParsedQuery, SearchEngine, SearchResult, parse_query};
use crate::state;
use crate::tree::TreeState;
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Onboarding,
    Tree,
    Search,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelpState {
    pub scroll: usize,
    pub page_size: usize,
    pub max_scroll: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    Tree,
    Description,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateChoice {
    pub value: String,
    pub label: String,
    pub has_explicit_label: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplatePickerState {
    pub choices: Vec<TemplateChoice>,
    pub cursor: usize,
}

pub struct App {
    pub catalog_root: PathBuf,
    pub entries: Vec<Entry>,
    pub mode: Mode,
    pub prev_mode: Mode,
    pub query: String,
    pub query_cursor: usize,
    pub parsed_query: Option<ParsedQuery>,
    pub tree: TreeState,
    pub tree_scroll: usize,
    pub tree_page_size: usize,
    pub description_scroll: usize,
    pub description_page_size: usize,
    pub description_max_scroll: usize,
    pub description_wrap_width: usize,
    pub description_needs_scroll_sync: bool,
    pub search_results: Vec<SearchResult>,
    pub search_cursor: usize,
    pub search_scroll: usize,
    pub search_page_size: usize,
    /// Shared selection between Tree and Search modes.
    pub selected_entry_index: Option<usize>,
    pub enrichment_rx: Option<Receiver<EnrichmentResult>>,
    pub enrichment_tx: Option<Sender<EnrichmentResult>>,
    pub search_engine: SearchEngine,
    pub should_quit: bool,
    /// Set to Some(filename) to exit and print filename.
    pub output: Option<String>,
    pub editor: Option<crate::editor::EditorState>,
    pub template_picker: Option<TemplatePickerState>,
    pub overlay: Option<Overlay>,
    pub help_state: HelpState,
    pub focus: PaneFocus,
    pub pane_split_percent: u16,
}

impl App {
    pub fn new(
        catalog_root: PathBuf,
        entries: Vec<Entry>,
        enrichment: Option<(Sender<EnrichmentResult>, Receiver<EnrichmentResult>)>,
        pane_split_percent: u16,
    ) -> Self {
        let tree = TreeState::new(&entries);
        let selected = tree.selected_entry_index;
        let has_entries = !entries.is_empty();
        let (enrichment_tx, enrichment_rx) = match enrichment {
            Some((tx, rx)) => (Some(tx), Some(rx)),
            None => (None, None),
        };
        App {
            catalog_root,
            entries,
            mode: if has_entries {
                Mode::Tree
            } else {
                Mode::Onboarding
            },
            prev_mode: Mode::Tree,
            query: String::new(),
            query_cursor: 0,
            parsed_query: None,
            tree,
            tree_scroll: 0,
            tree_page_size: 0,
            description_scroll: 0,
            description_page_size: 0,
            description_max_scroll: 0,
            description_wrap_width: 0,
            description_needs_scroll_sync: false,
            search_results: Vec::new(),
            search_cursor: 0,
            search_scroll: 0,
            search_page_size: 0,
            selected_entry_index: selected,
            enrichment_rx,
            enrichment_tx,
            search_engine: SearchEngine::new(),
            should_quit: false,
            output: None,
            editor: None,
            template_picker: None,
            overlay: None,
            help_state: HelpState {
                scroll: 0,
                page_size: 0,
                max_scroll: 0,
            },
            focus: PaneFocus::Tree,
            pane_split_percent: state::clamp_pane_split_percent(pane_split_percent),
        }
    }

    pub fn create_demo_catalog(&mut self) -> io::Result<()> {
        demo::ensure_demo_catalog(&self.catalog_root)?;
        self.switch_catalog(self.catalog_root.clone())
    }

    pub fn switch_catalog(&mut self, catalog_root: PathBuf) -> io::Result<()> {
        std::fs::create_dir_all(&catalog_root)?;
        let entries = crate::catalog::load_catalog(&catalog_root)?;

        self.catalog_root = catalog_root;
        self.entries = entries;
        self.tree = TreeState::new(&self.entries);
        self.tree_scroll = 0;
        self.tree_page_size = 0;
        self.description_scroll = 0;
        self.description_page_size = 0;
        self.description_max_scroll = 0;
        self.description_wrap_width = 0;
        self.description_needs_scroll_sync = false;
        self.search_results.clear();
        self.search_cursor = 0;
        self.search_scroll = 0;
        self.search_page_size = 0;
        self.selected_entry_index = self.tree.selected_entry_index;
        self.template_picker = None;
        self.editor = None;
        self.overlay = None;
        self.help_state = HelpState {
            scroll: 0,
            page_size: 0,
            max_scroll: 0,
        };
        self.focus = PaneFocus::Tree;
        self.query.clear();
        self.query_cursor = 0;
        self.parsed_query = None;
        self.mode = if self.entries.is_empty() {
            Mode::Onboarding
        } else {
            Mode::Tree
        };
        self.prev_mode = Mode::Tree;

        if let Some(tx) = &self.enrichment_tx {
            for entry in &self.entries {
                enrich_entry(entry, tx);
            }
        }

        Ok(())
    }

    pub fn adjust_pane_split(&mut self, delta: i16) {
        let current = self.pane_split_percent as i16;
        let next = (current + delta).clamp(10, 90) as u16;
        if next != self.pane_split_percent {
            self.pane_split_percent = next;
            let _ = state::save_pane_split_percent(next);
        }
    }

    /// Drain enrichment channel and apply results. Call each tick.
    pub fn apply_enrichment(&mut self) {
        let results: Vec<EnrichmentResult> = match &self.enrichment_rx {
            None => return,
            Some(rx) => {
                let mut v = Vec::new();
                while let Ok(r) = rx.try_recv() {
                    v.push(r);
                }
                v
            }
        };
        if results.is_empty() {
            return;
        }
        for result in results {
            if let Some(entry) = self.entries.iter_mut().find(|e| e.path == result.path) {
                if result.command_index < entry.enriched_output.len() {
                    entry.enriched_output[result.command_index] = result.output;
                    if result.command_index < entry.enriched_status.len() {
                        entry.enriched_status[result.command_index] = result.status_text;
                    }
                }
            }
        }
        if self.mode == Mode::Search {
            self.run_search();
        }
    }

    /// Update query string, switch modes, and re-run search.
    pub fn set_query(&mut self, query: String) {
        let cursor = query.len();
        self.set_query_with_cursor(query, cursor);
    }

    fn set_query_with_cursor(&mut self, query: String, cursor: usize) {
        self.query = query;
        self.query_cursor = cursor.min(self.query.len());
        if self.query.is_empty() {
            self.mode = Mode::Tree;
            self.parsed_query = None;
            if let Some(idx) = self.selected_entry_index {
                self.tree.focus_entry(idx, &self.entries);
            }
        } else {
            self.mode = Mode::Search;
            self.parsed_query = Some(parse_query(&self.query));
            self.run_search();
        }
    }

    fn run_search(&mut self) {
        if let Some(pq) = &self.parsed_query {
            self.search_results = self.search_engine.search(&self.entries, pq);
            self.search_cursor = 0;
            self.search_scroll = 0;
            self.description_needs_scroll_sync = true;
            let selected = self
                .search_results
                .get(self.search_cursor)
                .map(|r| r.entry_index);
            set_selected_entry(self, selected);
        }
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        self.selected_entry_index.map(|i| &self.entries[i])
    }

    fn prev_query_boundary(&self) -> usize {
        self.query[..self.query_cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next_query_boundary(&self) -> usize {
        self.query[self.query_cursor..]
            .chars()
            .next()
            .map(|ch| self.query_cursor + ch.len_utf8())
            .unwrap_or(self.query.len())
    }

    fn prev_query_word_boundary(&self) -> usize {
        let mut chars = self.query[..self.query_cursor]
            .char_indices()
            .collect::<Vec<_>>();

        while let Some((_, ch)) = chars.last() {
            if ch.is_whitespace() {
                chars.pop();
            } else {
                break;
            }
        }

        let mut boundary = 0;
        while let Some((index, ch)) = chars.pop() {
            if ch.is_whitespace() {
                boundary = index + ch.len_utf8();
                break;
            }
            boundary = index;
        }

        boundary
    }

    fn next_query_word_boundary(&self) -> usize {
        let slice = &self.query[self.query_cursor..];
        let mut offset = self.query_cursor;
        let mut chars = slice.chars().peekable();

        while let Some(ch) = chars.peek() {
            if ch.is_whitespace() {
                break;
            }
            offset += ch.len_utf8();
            chars.next();
        }

        while let Some(ch) = chars.peek() {
            if !ch.is_whitespace() {
                break;
            }
            offset += ch.len_utf8();
            chars.next();
        }

        offset
    }

    pub fn move_query_cursor_left(&mut self) {
        self.query_cursor = self.prev_query_boundary();
    }

    pub fn move_query_cursor_right(&mut self) {
        self.query_cursor = self.next_query_boundary();
    }

    pub fn move_query_cursor_home(&mut self) {
        self.query_cursor = 0;
    }

    pub fn move_query_cursor_end(&mut self) {
        self.query_cursor = self.query.len();
    }

    pub fn move_query_cursor_word_left(&mut self) {
        self.query_cursor = self.prev_query_word_boundary();
    }

    pub fn move_query_cursor_word_right(&mut self) {
        self.query_cursor = self.next_query_word_boundary();
    }

    pub fn insert_query_char(&mut self, ch: char) {
        let mut query = self.query.clone();
        query.insert(self.query_cursor, ch);
        self.set_query_with_cursor(query, self.query_cursor + ch.len_utf8());
    }

    pub fn delete_query_backward(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let start = self.prev_query_boundary();
        let mut query = self.query.clone();
        query.drain(start..self.query_cursor);
        self.set_query_with_cursor(query, start);
    }

    pub fn delete_query_forward(&mut self) {
        if self.query_cursor == self.query.len() {
            return;
        }
        let end = self.next_query_boundary();
        let mut query = self.query.clone();
        query.drain(self.query_cursor..end);
        self.set_query_with_cursor(query, self.query_cursor);
    }

    pub fn delete_query_word_backward(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let start = self.prev_query_word_boundary();
        let mut query = self.query.clone();
        query.drain(start..self.query_cursor);
        self.set_query_with_cursor(query, start);
    }

    pub fn delete_query_to_start(&mut self) {
        if self.query_cursor == 0 {
            return;
        }
        let mut query = self.query.clone();
        query.drain(..self.query_cursor);
        self.set_query_with_cursor(query, 0);
    }

    pub fn delete_query_to_end(&mut self) {
        if self.query_cursor == self.query.len() {
            return;
        }
        let mut query = self.query.clone();
        query.drain(self.query_cursor..);
        self.set_query_with_cursor(query, self.query_cursor);
    }

    pub fn template_choices_for_entry(entry: &Entry) -> Vec<TemplateChoice> {
        let mut seen = HashSet::new();
        let mut choices = Vec::new();

        for template in &entry.templates {
            if seen.insert(template.value.clone()) {
                choices.push(TemplateChoice {
                    value: template.value.clone(),
                    label: template.label.clone(),
                    has_explicit_label: template.label != template.value,
                });
            }
        }

        if choices.is_empty() {
            choices.push(TemplateChoice {
                value: entry.filename.clone(),
                label: entry.filename.clone(),
                has_explicit_label: false,
            });
        }

        choices
    }

    fn submit_entry_selection(&mut self, entry_index: usize) {
        let choices = Self::template_choices_for_entry(&self.entries[entry_index]);
        if choices.len() == 1 {
            self.output = Some(choices[0].value.clone());
        } else {
            self.template_picker = Some(TemplatePickerState { choices, cursor: 0 });
        }
    }

    /// Open editor for the currently selected entry.
    pub fn open_editor(&mut self) {
        let idx = match self.selected_entry_index {
            Some(i) => i,
            None => return,
        };
        let content = std::fs::read_to_string(&self.entries[idx].path).unwrap_or_default();
        let normalized_content = content.lines().collect::<Vec<_>>().join("\n");
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let textarea = tui_textarea::TextArea::from(lines);
        self.editor = Some(crate::editor::EditorState {
            textarea,
            entry_index: idx,
            original_content: normalized_content,
        });
        self.prev_mode = self.mode.clone();
        self.mode = Mode::Editor;
    }

    /// Save editor content, reload entry, restart enrichment.
    pub fn save_editor(&mut self) {
        let (content, entry_index) = match &self.editor {
            Some(ed) => (ed.textarea.lines().join("\n"), ed.entry_index),
            None => return,
        };
        let path = self.entries[entry_index].path.clone();
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &content).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        if let Ok(updated) = crate::catalog::parse_entry(&path, &self.catalog_root) {
            self.entries[entry_index] = updated;
        }
        // Re-run enrichment for this entry
        if let Some(tx) = &self.enrichment_tx {
            enrich_entry(&self.entries[entry_index], tx);
        }
        self.editor = None;
        self.mode = self.prev_mode.clone();
    }

    pub fn close_editor(&mut self) {
        self.editor = None;
        self.mode = self.prev_mode.clone();
    }

    pub fn select_tree_visible_index(&mut self, index: usize) {
        let len = self.tree.visible_items(&self.entries).len();
        if len == 0 {
            return;
        }
        self.tree.cursor = index.min(len - 1);
        sync_tree_selection(self);
    }

    pub fn select_search_visible_index(&mut self, index: usize) {
        if self.search_results.is_empty() {
            return;
        }
        self.search_cursor = index.min(self.search_results.len() - 1);
        sync_search_selection(self);
    }

    pub fn activate_current_selection(&mut self) {
        match self.mode {
            Mode::Onboarding => {}
            Mode::Tree => {
                let items = self.tree.visible_items(&self.entries);
                if let Some(item) = items.get(self.tree.cursor).cloned() {
                    match item.kind {
                        crate::tree::TreeItemKind::Entry { entry_index } => {
                            self.submit_entry_selection(entry_index);
                        }
                        crate::tree::TreeItemKind::Category { .. } => {
                            self.tree.toggle_collapse(&self.entries);
                            self.selected_entry_index = self.tree.selected_entry_index;
                        }
                    }
                }
            }
            Mode::Search => {
                if let Some(result) = self.search_results.get(self.search_cursor) {
                    self.submit_entry_selection(result.entry_index);
                }
            }
            Mode::Editor => {}
        }
    }

    pub fn submit_template_choice(&mut self, index: usize) {
        let selected = self
            .template_picker
            .as_ref()
            .and_then(|picker| picker.choices.get(index))
            .map(|choice| choice.value.clone());
        self.template_picker = None;
        if let Some(value) = selected {
            self.output = Some(value);
        }
    }

    pub fn toggle_help(&mut self) {
        self.overlay = match self.overlay {
            Some(Overlay::Help) => None,
            None => {
                self.help_state.scroll = 0;
                Some(Overlay::Help)
            }
        };
    }
}

pub fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};

    if matches!(
        (key.modifiers, key.code),
        (KeyModifiers::CONTROL, KeyCode::Char('h'))
    ) {
        app.toggle_help();
        return;
    }

    if app.overlay == Some(Overlay::Help) {
        handle_help_key(app, key);
        return;
    }

    if app.template_picker.is_some() {
        handle_template_picker_key(app, key);
        return;
    }
    match app.mode {
        Mode::Onboarding => handle_onboarding_key(app, key),
        Mode::Tree => handle_tree_key(app, key),
        Mode::Search => handle_search_key(app, key),
        Mode::Editor => handle_editor_key(app, key),
    }
}

fn handle_help_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;

    match key.code {
        KeyCode::Up => {
            app.help_state.scroll = app.help_state.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            app.help_state.scroll = (app.help_state.scroll + 1).min(app.help_state.max_scroll);
        }
        KeyCode::PageUp => {
            let page = app.help_state.page_size.max(1);
            app.help_state.scroll = app.help_state.scroll.saturating_sub(page);
        }
        KeyCode::PageDown => {
            let page = app.help_state.page_size.max(1);
            app.help_state.scroll = (app.help_state.scroll + page).min(app.help_state.max_scroll);
        }
        KeyCode::Home => {
            app.help_state.scroll = 0;
        }
        KeyCode::End => {
            app.help_state.scroll = app.help_state.max_scroll;
        }
        _ => {
            app.overlay = None;
        }
    }
}

fn set_selected_entry(app: &mut App, selected: Option<usize>) {
    if app.selected_entry_index != selected {
        app.description_scroll = 0;
        app.description_needs_scroll_sync = true;
    }
    app.selected_entry_index = selected;
}

fn sync_tree_selection(app: &mut App) {
    let items = app.tree.visible_items(&app.entries);
    app.tree.selected_entry_index = items.get(app.tree.cursor).and_then(|item| {
        if let crate::tree::TreeItemKind::Entry { entry_index } = item.kind {
            Some(entry_index)
        } else {
            None
        }
    });
    set_selected_entry(app, app.tree.selected_entry_index);
}

fn sync_search_selection(app: &mut App) {
    let selected = app
        .search_results
        .get(app.search_cursor)
        .map(|r| r.entry_index);
    set_selected_entry(app, selected);
}

fn scroll_description_down(app: &mut App, amount: usize) {
    app.description_scroll = (app.description_scroll + amount).min(app.description_max_scroll);
}

fn handle_onboarding_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    match (key.modifiers, key.code) {
        (mods, KeyCode::Char('c')) if mods.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true
        }
        (_, KeyCode::Enter) => {
            let _ = app.create_demo_catalog();
        }
        (_, KeyCode::Esc) => app.should_quit = true,
        _ => {}
    }
}

fn handle_tree_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    match (key.modifiers, key.code) {
        (mods, KeyCode::Left) if mods.contains(KeyModifiers::SHIFT) => app.adjust_pane_split(-10),
        (mods, KeyCode::Right) if mods.contains(KeyModifiers::SHIFT) => app.adjust_pane_split(10),
        (mods, KeyCode::Char('c')) if mods.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if app.focus == PaneFocus::Description => {
            app.open_editor()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if app.focus == PaneFocus::Tree => {
            app.open_editor()
        }
        (_, KeyCode::Char(c)) => {
            app.insert_query_char(c);
        }
        (_, KeyCode::Backspace) => {
            app.delete_query_backward();
        }
        (_, KeyCode::Down) if app.focus == PaneFocus::Tree => {
            app.tree.move_down(&app.entries);
            set_selected_entry(app, app.tree.selected_entry_index);
        }
        (_, KeyCode::Up) if app.focus == PaneFocus::Tree => {
            app.tree.move_up(&app.entries);
            set_selected_entry(app, app.tree.selected_entry_index);
        }
        (_, KeyCode::Left) if app.focus == PaneFocus::Tree => {
            app.tree.move_left(&app.entries);
            set_selected_entry(app, app.tree.selected_entry_index);
        }
        (_, KeyCode::Right) if app.focus == PaneFocus::Tree => {
            if app.selected_entry_index.is_some() {
                app.focus = PaneFocus::Description;
            } else {
                app.tree.move_right(&app.entries);
                set_selected_entry(app, app.tree.selected_entry_index);
            }
        }
        (_, KeyCode::Left) if app.focus == PaneFocus::Description => {
            app.focus = PaneFocus::Tree;
        }
        (_, KeyCode::PageDown) if app.focus == PaneFocus::Tree => {
            let page = app.tree_page_size.max(1);
            let len = app.tree.visible_items(&app.entries).len();
            if len > 0 {
                app.tree.cursor = (app.tree.cursor + page).min(len - 1);
                sync_tree_selection(app);
            }
        }
        (_, KeyCode::PageUp) if app.focus == PaneFocus::Tree => {
            let page = app.tree_page_size.max(1);
            app.tree.cursor = app.tree.cursor.saturating_sub(page);
            sync_tree_selection(app);
        }
        (_, KeyCode::Down) if app.focus == PaneFocus::Description => {
            scroll_description_down(app, 1);
        }
        (_, KeyCode::Up) if app.focus == PaneFocus::Description => {
            app.description_scroll = app.description_scroll.saturating_sub(1);
        }
        (_, KeyCode::PageDown) if app.focus == PaneFocus::Description => {
            scroll_description_down(app, app.description_page_size.max(1));
        }
        (_, KeyCode::PageUp) if app.focus == PaneFocus::Description => {
            let page = app.description_page_size.max(1);
            app.description_scroll = app.description_scroll.saturating_sub(page);
        }
        (_, KeyCode::Enter) => app.activate_current_selection(),
        (_, KeyCode::Esc) => app.should_quit = true,
        _ => {}
    }
}

fn handle_search_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};

    match (key.modifiers, key.code) {
        (mods, KeyCode::Left) if mods.contains(KeyModifiers::SHIFT) => app.adjust_pane_split(-10),
        (mods, KeyCode::Right) if mods.contains(KeyModifiers::SHIFT) => app.adjust_pane_split(10),
        (mods, KeyCode::Char('c')) if mods.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if app.focus == PaneFocus::Description => {
            app.open_editor()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if app.focus == PaneFocus::Tree => {
            app.open_editor()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) if app.focus == PaneFocus::Tree => {
            app.move_query_cursor_home()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('b')) if app.focus == PaneFocus::Tree => {
            app.move_query_cursor_left()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('f')) if app.focus == PaneFocus::Tree => {
            app.move_query_cursor_right()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) if app.focus == PaneFocus::Tree => {
            app.delete_query_word_backward()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) if app.focus == PaneFocus::Tree => {
            app.delete_query_to_start()
        }
        (KeyModifiers::CONTROL, KeyCode::Char('k')) if app.focus == PaneFocus::Tree => {
            app.delete_query_to_end()
        }
        (_, KeyCode::Char(c)) => {
            app.insert_query_char(c);
        }
        (mods, KeyCode::Backspace)
            if app.focus == PaneFocus::Tree && mods.contains(KeyModifiers::ALT) =>
        {
            app.delete_query_word_backward();
        }
        (_, KeyCode::Backspace) => {
            app.delete_query_backward();
        }
        (_, KeyCode::Delete) => {
            app.delete_query_forward();
        }
        (mods, KeyCode::Left)
            if app.focus == PaneFocus::Tree && mods.contains(KeyModifiers::ALT) =>
        {
            app.move_query_cursor_word_left();
        }
        (mods, KeyCode::Right)
            if app.focus == PaneFocus::Tree && mods.contains(KeyModifiers::ALT) =>
        {
            app.move_query_cursor_word_right();
        }
        (_, KeyCode::Home) => {
            app.move_query_cursor_home();
        }
        (_, KeyCode::End) => {
            app.move_query_cursor_end();
        }
        (_, KeyCode::Left) if app.focus == PaneFocus::Tree => {
            app.move_query_cursor_left();
        }
        (_, KeyCode::Right) if app.focus == PaneFocus::Tree => {
            app.move_query_cursor_right();
        }
        (_, KeyCode::Down) if app.focus == PaneFocus::Tree => {
            if app.search_cursor + 1 < app.search_results.len() {
                app.search_cursor += 1;
                sync_search_selection(app);
            }
        }
        (_, KeyCode::Up) if app.focus == PaneFocus::Tree => {
            if app.search_cursor > 0 {
                app.search_cursor -= 1;
                sync_search_selection(app);
            }
        }
        (_, KeyCode::PageDown) if app.focus == PaneFocus::Tree => {
            let page = app.search_page_size.max(1);
            if !app.search_results.is_empty() {
                app.search_cursor = (app.search_cursor + page).min(app.search_results.len() - 1);
                sync_search_selection(app);
            }
        }
        (_, KeyCode::PageUp) if app.focus == PaneFocus::Tree => {
            let page = app.search_page_size.max(1);
            app.search_cursor = app.search_cursor.saturating_sub(page);
            sync_search_selection(app);
        }
        (_, KeyCode::Tab) if app.focus == PaneFocus::Tree => {
            if app.selected_entry_index.is_some() {
                app.focus = PaneFocus::Description;
            }
        }
        (_, KeyCode::Left) if app.focus == PaneFocus::Description => {
            app.focus = PaneFocus::Tree;
        }
        (_, KeyCode::Down) if app.focus == PaneFocus::Description => {
            scroll_description_down(app, 1);
        }
        (_, KeyCode::Up) if app.focus == PaneFocus::Description => {
            app.description_scroll = app.description_scroll.saturating_sub(1);
        }
        (_, KeyCode::PageDown) if app.focus == PaneFocus::Description => {
            scroll_description_down(app, app.description_page_size.max(1));
        }
        (_, KeyCode::PageUp) if app.focus == PaneFocus::Description => {
            let page = app.description_page_size.max(1);
            app.description_scroll = app.description_scroll.saturating_sub(page);
        }
        (_, KeyCode::Enter) => app.activate_current_selection(),
        (_, KeyCode::Esc) => app.set_query(String::new()),
        _ => {}
    }
}

fn handle_template_picker_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;

    if app.template_picker.is_none() {
        return;
    }

    match key.code {
        KeyCode::Down => {
            let picker = app.template_picker.as_mut().expect("checked above");
            if picker.cursor + 1 < picker.choices.len() {
                picker.cursor += 1;
            }
        }
        KeyCode::Up => {
            let picker = app.template_picker.as_mut().expect("checked above");
            picker.cursor = picker.cursor.saturating_sub(1);
        }
        KeyCode::PageDown => {
            let picker = app.template_picker.as_mut().expect("checked above");
            let page = 10usize;
            if !picker.choices.is_empty() {
                picker.cursor = (picker.cursor + page).min(picker.choices.len() - 1);
            }
        }
        KeyCode::PageUp => {
            let picker = app.template_picker.as_mut().expect("checked above");
            picker.cursor = picker.cursor.saturating_sub(10);
        }
        KeyCode::Enter => {
            let cursor = app
                .template_picker
                .as_ref()
                .map(|picker| picker.cursor)
                .unwrap_or(0);
            app.submit_template_choice(cursor);
        }
        KeyCode::Esc => {
            app.template_picker = None;
        }
        _ => {}
    }
}

fn handle_editor_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    match (key.modifiers, key.code) {
        (mods, KeyCode::Left) if mods.contains(KeyModifiers::SHIFT) => app.adjust_pane_split(-10),
        (mods, KeyCode::Right) if mods.contains(KeyModifiers::SHIFT) => app.adjust_pane_split(10),
        (mods, KeyCode::Char('c')) if mods.contains(KeyModifiers::CONTROL) => {
            app.close_editor();
            app.should_quit = true;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => app.save_editor(),
        (_, KeyCode::Esc) => app.close_editor(),
        _ => {
            if let Some(ed) = &mut app.editor {
                ed.textarea.input(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{App, Mode, Overlay, TemplateChoice, handle_key};
    use crate::catalog::{BodyLine, Entry, Template};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn entry_with_templates(templates: Vec<Template>) -> Entry {
        Entry {
            filename: "tool".to_string(),
            display_name: None,
            description: templates
                .iter()
                .map(|template| template.raw_line.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            body_lines: templates
                .iter()
                .enumerate()
                .map(|(i, _)| BodyLine::Template(i))
                .collect(),
            templates,
            enrich_commands: vec![],
            enriched_output: vec![],
            enriched_status: vec![],
            category: "misc".to_string(),
            path: PathBuf::from("misc/tool"),
        }
    }

    #[test]
    fn template_choices_fall_back_to_filename() {
        let entry = entry_with_templates(vec![]);
        assert_eq!(
            App::template_choices_for_entry(&entry),
            vec![TemplateChoice {
                value: "tool".to_string(),
                label: "tool".to_string(),
                has_explicit_label: false
            }]
        );
    }

    #[test]
    fn template_choices_deduplicate_by_value() {
        let entry = entry_with_templates(vec![
            Template {
                value: "cargo test".to_string(),
                label: "Test".to_string(),
                raw_line: "@ (Test) cargo test".to_string(),
            },
            Template {
                value: "cargo test".to_string(),
                label: "Duplicate".to_string(),
                raw_line: "@ (Duplicate) cargo test".to_string(),
            },
            Template {
                value: "cargo watch -x test".to_string(),
                label: "Watch".to_string(),
                raw_line: "@ (Watch) cargo watch -x test".to_string(),
            },
        ]);

        assert_eq!(
            App::template_choices_for_entry(&entry),
            vec![
                TemplateChoice {
                    value: "cargo test".to_string(),
                    label: "Test".to_string(),
                    has_explicit_label: true
                },
                TemplateChoice {
                    value: "cargo watch -x test".to_string(),
                    label: "Watch".to_string(),
                    has_explicit_label: true
                }
            ]
        );
    }

    #[test]
    fn empty_catalog_starts_in_onboarding_mode() {
        let app = App::new(PathBuf::from("/tmp/fz1-empty"), vec![], None, 50);
        assert_eq!(app.mode, Mode::Onboarding);
    }

    #[test]
    fn creating_demo_catalog_loads_entries_and_tree_mode() {
        let temp = TempDir::new().unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir_all(&catalog_root).unwrap();

        let mut app = App::new(catalog_root.clone(), vec![], None, 50);

        app.create_demo_catalog().unwrap();

        assert_eq!(app.mode, Mode::Tree);
        assert_eq!(app.catalog_root, catalog_root);
        assert_eq!(app.entries.len(), 2);
        assert!(app.entries.iter().any(|entry| entry.filename == "git"));
        assert!(app.entries.iter().any(|entry| entry.filename == "curl"));
    }

    #[test]
    fn activating_selected_entry_without_templates_returns_filename() {
        let mut entry = entry_with_templates(vec![]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);

        app.activate_current_selection();

        assert_eq!(app.output.as_deref(), Some("tool"));
        assert!(app.template_picker.is_none());
    }

    #[test]
    fn activating_selected_entry_with_multiple_templates_opens_picker_and_returns_choice() {
        let mut entry = entry_with_templates(vec![
            Template {
                value: "git status".to_string(),
                label: "git status".to_string(),
                raw_line: "@ git status".to_string(),
            },
            Template {
                value: "git log --oneline".to_string(),
                label: "History".to_string(),
                raw_line: "@ (History) git log --oneline".to_string(),
            },
        ]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);

        app.activate_current_selection();

        assert!(app.output.is_none());
        assert_eq!(
            app.template_picker
                .as_ref()
                .map(|picker| picker.choices.len()),
            Some(2)
        );

        app.submit_template_choice(1);

        assert_eq!(app.output.as_deref(), Some("git log --oneline"));
        assert!(app.template_picker.is_none());
    }

    #[test]
    fn ctrl_h_toggles_help_overlay() {
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![], None, 50);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.overlay, Some(Overlay::Help));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.overlay, None);
    }

    #[test]
    fn any_non_mouse_key_closes_help_overlay() {
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![], None, 50);
        app.overlay = Some(Overlay::Help);

        handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(app.overlay, None);
    }

    #[test]
    fn arrow_keys_scroll_help_overlay() {
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![], None, 50);
        app.overlay = Some(Overlay::Help);
        app.help_state.page_size = 5;
        app.help_state.max_scroll = 10;
        app.help_state.scroll = 4;

        handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.help_state.scroll, 5);
        assert_eq!(app.overlay, Some(Overlay::Help));

        handle_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.help_state.scroll, 4);
        assert_eq!(app.overlay, Some(Overlay::Help));
    }

    #[test]
    fn search_query_supports_mid_string_editing() {
        let mut entry = entry_with_templates(vec![]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        );
        handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
        );

        assert_eq!(app.query, "abc");
        assert_eq!(app.query_cursor, 2);
    }

    #[test]
    fn search_query_delete_removes_character_at_cursor() {
        let mut entry = entry_with_templates(vec![]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);

        app.set_query("abcd".to_string());
        app.query_cursor = 2;

        handle_key(&mut app, KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));

        assert_eq!(app.query, "abd");
        assert_eq!(app.query_cursor, 2);
    }

    #[test]
    fn alt_left_and_right_move_by_word_boundaries() {
        let mut entry = entry_with_templates(vec![]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);

        app.set_query("git status --short".to_string());
        app.query_cursor = app.query.len();

        handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(app.query_cursor, "git status ".len());

        handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(app.query_cursor, "git ".len());

        handle_key(&mut app, KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(app.query_cursor, "git status ".len());
    }

    #[test]
    fn alt_backspace_deletes_previous_word() {
        let mut entry = entry_with_templates(vec![]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);

        app.set_query("git status --short".to_string());
        app.query_cursor = app.query.len();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
        );

        assert_eq!(app.query, "git status ");
        assert_eq!(app.query_cursor, "git status ".len());
    }

    #[test]
    fn ctrl_shortcuts_use_terminal_line_editing_conventions() {
        let mut entry = entry_with_templates(vec![]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);

        app.set_query("git status --short".to_string());
        app.query_cursor = "git status".len();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.query_cursor, 0);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.query_cursor, 1);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.query_cursor, 0);

        app.query_cursor = app.query.len();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.query, "");
        assert_eq!(app.query_cursor, 0);

        app.set_query("git status --short".to_string());
        app.query_cursor = "git ".len();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.query, "git ");
        assert_eq!(app.query_cursor, "git ".len());

        app.set_query("git status --short".to_string());
        app.query_cursor = app.query.len();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.query, "git status ");
        assert_eq!(app.query_cursor, "git status ".len());
    }

    #[test]
    fn ctrl_e_opens_editor_from_search_results() {
        let mut entry = entry_with_templates(vec![]);
        entry.category.clear();
        entry.path = PathBuf::from("tool");
        let mut app = App::new(PathBuf::from("/tmp/fz1"), vec![entry], None, 50);
        app.set_query("tool".to_string());

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        );

        assert_eq!(app.mode, Mode::Editor);
        assert!(app.editor.is_some());
    }
}
