use crate::app::{App, Mode, Overlay, PaneFocus, handle_key};
use crate::catalog::BodyLine;
use crate::search::{MatchLayout, SearchEngine, SearchResult, slice_highlight_indices};
use crate::tree::TreeItemKind;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook::flag;
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(350);
const MOUSE_WHEEL_STEP: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseTarget {
    LeftItem(usize),
    RightPane,
    TemplateChoice(usize),
}

#[derive(Debug, Clone, Copy)]
struct MouseClickState {
    target: MouseTarget,
    instant: Instant,
}

#[derive(Debug, Clone, Copy)]
struct UiLayout {
    search_bar: Rect,
    status_bar: Rect,
    left_pane: Rect,
    right_pane: Rect,
    left_inner: Rect,
}

#[derive(Debug, Clone, Copy)]
struct HelpSection {
    title: &'static str,
    rows: &'static [(&'static str, &'static str)],
}

const HELP_SECTIONS: &[HelpSection] = &[
    HelpSection {
        title: "Global",
        rows: &[
            ("ctrl+h", "open help"),
            ("shift+left", "shrink left pane"),
            ("shift+right", "grow left pane"),
            ("ctrl+c", "quit"),
        ],
    },
    HelpSection {
        title: "Tree",
        rows: &[
            ("up / down", "move in category tree"),
            ("left", "collapse category or go to parent"),
            ("right", "expand category or move to description"),
            ("pageup / pagedown", "page through tree"),
            ("enter", "toggle category or select entry"),
            ("esc", "quit"),
        ],
    },
    HelpSection {
        title: "Search",
        rows: &[
            ("up / down", "move through matches"),
            ("left / right", "move cursor in query"),
            ("alt+left / alt+right", "move by word"),
            ("alt+backspace", "delete previous word"),
            ("ctrl+a", "jump to start"),
            ("ctrl+b / ctrl+f", "move cursor left/right"),
            ("ctrl+w", "delete previous word"),
            ("ctrl+u / ctrl+k", "delete to start/end"),
            ("ctrl+e", "open editor"),
            ("home / end", "jump in query"),
            ("delete", "delete character at cursor"),
            ("pageup / pagedown", "page through matches"),
            ("tab", "move to description"),
            ("esc", "clear query and return to tree"),
            ("enter", "select highlighted entry"),
        ],
    },
    HelpSection {
        title: "Description",
        rows: &[
            ("up / down", "scroll"),
            ("pageup / pagedown", "page scroll"),
            ("left", "return to left pane"),
            ("ctrl+e", "open editor"),
        ],
    },
    HelpSection {
        title: "Editor",
        rows: &[
            ("ctrl+s", "save to the same file"),
            ("esc", "close without saving"),
        ],
    },
    HelpSection {
        title: "Templates",
        rows: &[
            ("up / down", "move choice"),
            ("pageup / pagedown", "jump by page"),
            ("enter", "confirm choice"),
            ("esc", "close picker"),
        ],
    },
];

pub fn run(app: &mut App) -> io::Result<Option<String>> {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;
    let signal_exit = install_signal_handlers()?;

    let result = event_loop(&mut terminal, app, &signal_exit);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    signal_exit: &Arc<AtomicBool>,
) -> io::Result<Option<String>> {
    let mut last_mouse_click: Option<MouseClickState> = None;
    loop {
        terminal.draw(|f| render(f, app))?;
        if signal_exit.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => handle_key(app, key),
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    let layout = compute_ui_layout_for_app(area, app);
                    handle_mouse(app, mouse, layout, &mut last_mouse_click);
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        app.apply_enrichment();
        if app.should_quit {
            return Ok(None);
        }
        if let Some(output) = app.output.take() {
            return Ok(Some(output));
        }
    }
}

fn compute_ui_layout_for_app(area: Rect, app: &App) -> UiLayout {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(app.pane_split_percent),
            Constraint::Percentage(100 - app.pane_split_percent),
        ])
        .split(rows[1]);

    UiLayout {
        search_bar: rows[0],
        status_bar: rows[2],
        left_pane: panes[0],
        right_pane: panes[1],
        left_inner: Block::default().borders(Borders::RIGHT).inner(panes[0]),
    }
}

fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn handle_mouse(
    app: &mut App,
    mouse: MouseEvent,
    layout: UiLayout,
    last_mouse_click: &mut Option<MouseClickState>,
) {
    if app.overlay.is_some() {
        handle_help_overlay_mouse(app, mouse, layout, last_mouse_click);
        return;
    }

    if app.mode == Mode::Onboarding {
        return;
    }

    if app.template_picker.is_some() {
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            handle_template_picker_click(app, mouse.column, mouse.row, layout, last_mouse_click);
        }
        return;
    }

    if app.mode == Mode::Editor {
        return;
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            handle_left_click(app, mouse.column, mouse.row, layout, last_mouse_click);
        }
        MouseEventKind::ScrollDown => {
            if rect_contains(layout.left_inner, mouse.column, mouse.row) {
                app.focus = PaneFocus::Tree;
                scroll_left_panel_down(app, MOUSE_WHEEL_STEP);
                *last_mouse_click = None;
            } else if rect_contains(layout.right_pane, mouse.column, mouse.row) {
                app.focus = PaneFocus::Description;
                app.description_scroll =
                    (app.description_scroll + MOUSE_WHEEL_STEP).min(app.description_max_scroll);
                *last_mouse_click = None;
            }
        }
        MouseEventKind::ScrollUp => {
            if rect_contains(layout.left_inner, mouse.column, mouse.row) {
                app.focus = PaneFocus::Tree;
                scroll_left_panel_up(app, MOUSE_WHEEL_STEP);
                *last_mouse_click = None;
            } else if rect_contains(layout.right_pane, mouse.column, mouse.row) {
                app.focus = PaneFocus::Description;
                app.description_scroll = app.description_scroll.saturating_sub(MOUSE_WHEEL_STEP);
                *last_mouse_click = None;
            }
        }
        _ => {}
    }
}

fn scroll_left_panel_down(app: &mut App, amount: usize) {
    match app.mode {
        Mode::Onboarding => {}
        Mode::Tree => {
            let len = app.tree.visible_items(&app.entries).len();
            if len == 0 {
                return;
            }
            let next = (app.tree.cursor + amount).min(len - 1);
            app.select_tree_visible_index(next);
        }
        Mode::Search => {
            if app.search_results.is_empty() {
                return;
            }
            let next = (app.search_cursor + amount).min(app.search_results.len() - 1);
            app.select_search_visible_index(next);
        }
        Mode::Editor => {}
    }
}

fn scroll_left_panel_up(app: &mut App, amount: usize) {
    match app.mode {
        Mode::Onboarding => {}
        Mode::Tree => {
            app.select_tree_visible_index(app.tree.cursor.saturating_sub(amount));
        }
        Mode::Search => {
            app.select_search_visible_index(app.search_cursor.saturating_sub(amount));
        }
        Mode::Editor => {}
    }
}

fn handle_left_click(
    app: &mut App,
    column: u16,
    row: u16,
    layout: UiLayout,
    last_mouse_click: &mut Option<MouseClickState>,
) {
    if rect_contains(layout.left_inner, column, row) {
        app.focus = PaneFocus::Tree;
        let offset = (row - layout.left_inner.y) as usize;
        let target = match app.mode {
            Mode::Onboarding => return,
            Mode::Tree => {
                let index = app.tree_scroll + offset;
                let len = app.tree.visible_items(&app.entries).len();
                if index >= len {
                    return;
                }
                app.select_tree_visible_index(index);
                MouseTarget::LeftItem(index)
            }
            Mode::Search => {
                let index = app.search_scroll + offset;
                if index >= app.search_results.len() {
                    return;
                }
                app.select_search_visible_index(index);
                MouseTarget::LeftItem(index)
            }
            Mode::Editor => return,
        };

        if is_double_click(*last_mouse_click, target) {
            app.activate_current_selection();
            *last_mouse_click = None;
        } else {
            *last_mouse_click = Some(MouseClickState {
                target,
                instant: Instant::now(),
            });
        }
        return;
    }

    if rect_contains(layout.right_pane, column, row) {
        app.focus = PaneFocus::Description;
        if is_double_click(*last_mouse_click, MouseTarget::RightPane) {
            app.open_editor();
            *last_mouse_click = None;
        } else {
            *last_mouse_click = Some(MouseClickState {
                target: MouseTarget::RightPane,
                instant: Instant::now(),
            });
        }
        return;
    }

    *last_mouse_click = None;
}

fn is_double_click(last_click: Option<MouseClickState>, target: MouseTarget) -> bool {
    let Some(last_click) = last_click else {
        return false;
    };
    last_click.target == target && last_click.instant.elapsed() <= DOUBLE_CLICK_THRESHOLD
}

fn handle_template_picker_click(
    app: &mut App,
    column: u16,
    row: u16,
    layout: UiLayout,
    last_mouse_click: &mut Option<MouseClickState>,
) {
    let Some(picker) = &app.template_picker else {
        return;
    };

    let area = Rect {
        x: 0,
        y: 0,
        width: layout
            .left_pane
            .width
            .saturating_add(layout.right_pane.width),
        height: layout.search_bar.height + layout.left_pane.height + layout.status_bar.height,
    };
    let popup_area = template_picker_area(area, picker.choices.len());
    let inner = Block::default()
        .borders(Borders::ALL)
        .title(" Templates ")
        .inner(popup_area);

    if !rect_contains(inner, column, row) {
        app.template_picker = None;
        *last_mouse_click = None;
        return;
    }

    let offset = (row - inner.y) as usize;
    if offset >= picker.choices.len() || offset >= inner.height as usize {
        *last_mouse_click = None;
        return;
    }

    if let Some(picker) = app.template_picker.as_mut() {
        picker.cursor = offset;
    }

    let target = MouseTarget::TemplateChoice(offset);
    if is_double_click(*last_mouse_click, target) {
        app.submit_template_choice(offset);
        *last_mouse_click = None;
    } else {
        *last_mouse_click = Some(MouseClickState {
            target,
            instant: Instant::now(),
        });
    }
}

fn install_signal_handlers() -> io::Result<Arc<AtomicBool>> {
    let exit = Arc::new(AtomicBool::new(false));
    for signal in [SIGINT, SIGTERM, SIGHUP, SIGQUIT] {
        flag::register(signal, Arc::clone(&exit))
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    }
    Ok(exit)
}

fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Clear, area);

    if app.mode == Mode::Onboarding {
        render_onboarding(f, app, area);
        render_help_overlay(f, app, area);
        return;
    }

    let layout = compute_ui_layout_for_app(area, app);

    render_search_bar(f, app, layout.search_bar);

    match app.mode {
        Mode::Tree | Mode::Editor => render_tree(f, app, layout.left_pane),
        Mode::Search => render_search_results(f, app, layout.left_pane),
        Mode::Onboarding => return,
    }

    if app.mode == Mode::Editor {
        if let Some(ed) = &app.editor {
            let editor_area = Rect {
                x: layout.right_pane.x.saturating_add(1),
                width: layout.right_pane.width.saturating_sub(1),
                ..layout.right_pane
            };
            f.render_widget(&ed.textarea, editor_area);
        }
    } else {
        render_description(f, app, layout.right_pane);
    }

    render_focus_marker(f, app, layout.left_pane);
    render_template_picker(f, app, area);
    render_help_overlay(f, app, area);

    render_status_bar(f, app, layout.status_bar);
}

fn render_onboarding(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let panel_width = area.width.saturating_sub(4).max(1);
    let panel_height = rows[0].height.saturating_sub(2).max(1);
    let panel_area = centered_rect(panel_width, panel_height, rows[0]);
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(panel_area);
    f.render_widget(Clear, panel_area);
    f.render_widget(block, panel_area);

    let text = Text::from(vec![
        Line::from(vec![Span::styled(
            "Your catalog is empty.",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Current catalog: ", Style::default().fg(Color::DarkGray)),
            Span::raw(app.catalog_root.display().to_string()),
        ]),
        Line::raw(""),
        Line::from(vec![Span::styled(
            "Press Enter to create demo content",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        )]),
        Line::from(vec![Span::styled(
            "or Esc to exit",
            Style::default().fg(Color::DarkGray),
        )]),
    ]);

    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
    render_status_bar(f, app, rows[1]);
}

fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Search ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut spans = vec![Span::raw("❯ ")];
    if app.query.is_empty() {
        let italic = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);
        spans.push(Span::styled("start typing to search", italic));
    } else {
        let available_width = inner.width.saturating_sub(2) as usize;
        let (visible_query, cursor_column) =
            visible_query_window(&app.query, app.query_cursor, available_width);
        spans.push(Span::raw(visible_query));
        f.set_cursor_position((inner.x.saturating_add(2 + cursor_column as u16), inner.y));
    }

    let input_line = Line::from(spans);
    f.render_widget(Paragraph::new(input_line), inner);
}

fn visible_query_window(query: &str, cursor: usize, width: usize) -> (String, usize) {
    let chars: Vec<char> = query.chars().collect();
    let cursor_chars = query[..cursor].chars().count();

    if chars.len() <= width {
        return (query.to_string(), cursor_chars);
    }

    let mut start = cursor_chars.saturating_sub(width);
    if cursor_chars >= start + width {
        start = cursor_chars + 1 - width;
    }
    let end = (start + width).min(chars.len());

    let visible = chars[start..end].iter().collect::<String>();
    let cursor_column = cursor_chars.saturating_sub(start);
    (visible, cursor_column)
}

fn render_tree(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items = app.tree.visible_items(&app.entries);
    let height = inner.height as usize;
    app.tree_page_size = height;
    app.tree_scroll = viewport_start(app.tree_scroll, app.tree.cursor, height, items.len());
    let start = app.tree_scroll;

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, item)| {
            let prefix = tree_prefix(item);
            let selected = i == app.tree.cursor;
            let sel_style = if selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            match &item.kind {
                TreeItemKind::Category {
                    name, collapsed, ..
                } => {
                    let icon = if *collapsed { "▶" } else { "▼" };
                    Line::from(vec![
                        Span::raw(prefix),
                        Span::styled(format!("{} {}", icon, name), sel_style.fg(Color::Cyan)),
                    ])
                }
                TreeItemKind::Entry { entry_index } => {
                    let entry = &app.entries[*entry_index];
                    let template_count = App::template_choices_for_entry(entry).len();
                    let mut spans = vec![
                        Span::raw(prefix),
                        Span::styled(entry.filename.clone(), sel_style),
                    ];
                    if let Some(display_name) = entry.display_name.as_deref() {
                        spans.push(Span::styled(
                            ": ".to_string(),
                            sel_style.fg(Color::DarkGray),
                        ));
                        spans.push(Span::styled(display_name.to_string(), sel_style));
                    }
                    if template_count > 1 {
                        spans.push(Span::styled(
                            format!(" [{}]", template_count),
                            sel_style.fg(Color::DarkGray),
                        ));
                    }
                    Line::from(spans)
                }
            }
        })
        .collect();

    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn tree_prefix(item: &crate::tree::TreeItem) -> String {
    let mut prefix = String::new();
    for has_next in &item.ancestor_has_next_sibling {
        prefix.push_str(if *has_next { "│  " } else { "   " });
    }
    if item.indent > 0 {
        prefix.push_str(if item.has_next_sibling {
            "├─ "
        } else {
            "└─ "
        });
    }
    prefix
}

fn render_search_results(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let height = inner.height as usize;
    app.search_page_size = height;
    app.search_scroll = viewport_start(
        app.search_scroll,
        app.search_cursor,
        height,
        app.search_results.len(),
    );
    let start = app.search_scroll;

    let lines: Vec<Line> = app
        .search_results
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, result)| {
            let entry = &app.entries[result.entry_index];
            let template_count = App::template_choices_for_entry(entry).len();
            let selected = i == app.search_cursor;
            let sel_style = if selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let layout = SearchEngine::build_layout_for_result(entry);
            let filename_indices =
                slice_highlight_indices(&result.highlight_indices, layout.filename.as_ref());
            let display_name_indices =
                slice_highlight_indices(&result.highlight_indices, layout.display_name.as_ref());
            let mut spans = highlight_spans(&entry.filename, &filename_indices, sel_style);
            if let Some(display_name) = entry.display_name.as_deref() {
                spans.push(Span::styled(
                    ": ".to_string(),
                    sel_style.fg(Color::DarkGray),
                ));
                spans.extend(highlight_spans(
                    display_name,
                    &display_name_indices,
                    sel_style,
                ));
            }
            if template_count > 1 {
                spans.push(Span::styled(
                    format!(" [{}]", template_count),
                    sel_style.fg(Color::DarkGray),
                ));
            }
            Line::from(spans)
        })
        .collect();

    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn viewport_start(current_scroll: usize, cursor: usize, height: usize, len: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }

    let max_scroll = len.saturating_sub(height);
    let mut scroll = current_scroll.min(max_scroll);

    if cursor < scroll {
        scroll = cursor;
    } else if cursor >= scroll + height {
        scroll = cursor + 1 - height;
    }

    scroll.min(max_scroll)
}

fn wrap_plain_lines(lines: &[String], width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut wrapped = Vec::new();
    for line in lines {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            wrapped.push(String::new());
            continue;
        }
        for chunk in chars.chunks(width) {
            wrapped.push(chunk.iter().collect());
        }
    }
    wrapped
}

fn first_highlighted_wrapped_line(
    lines: &[String],
    line_highlights: &[Vec<usize>],
    width: usize,
) -> Option<usize> {
    if width == 0 {
        return None;
    }

    let mut wrapped_offset = 0usize;
    for (line, indices) in lines.iter().zip(line_highlights.iter()) {
        if let Some(first_idx) = indices.iter().min() {
            return Some(wrapped_offset + (first_idx / width));
        }

        let line_len = line.chars().count();
        wrapped_offset += line_len.max(1).div_ceil(width);
    }

    None
}

fn description_progress_text(
    wrapped_plain_lines: &[String],
    scroll: usize,
    page_size: usize,
    width: usize,
) -> Option<String> {
    if page_size == 0 || width == 0 || wrapped_plain_lines.len() <= page_size {
        return None;
    }

    let visible_end = (scroll + page_size).min(wrapped_plain_lines.len());
    let percent = ((visible_end * 100) / wrapped_plain_lines.len()).min(100);
    let text = format!("{}%", percent);
    let first_visible_width = wrapped_plain_lines
        .get(scroll)
        .map_or(0, |line| line.chars().count());

    if text.chars().count() > width || first_visible_width + text.chars().count() > width {
        None
    } else {
        Some(text)
    }
}

fn highlight_spans(text: &str, indices: &[usize], base_style: Style) -> Vec<Span<'static>> {
    let idx_set: std::collections::HashSet<usize> = indices.iter().copied().collect();
    text.chars()
        .enumerate()
        .map(|(char_idx, c)| {
            let s = c.to_string();
            if idx_set.contains(&char_idx) {
                Span::styled(s, base_style.fg(Color::Yellow).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(s, base_style)
            }
        })
        .collect()
}

fn highlight_line(text: &str, indices: &[usize], base_style: Style) -> Line<'static> {
    Line::from(highlight_spans(text, indices, base_style))
}

fn merge_highlight_indices(mut left: Vec<usize>, right: Vec<usize>) -> Vec<usize> {
    left.extend(right);
    left.sort_unstable();
    left.dedup();
    left
}

fn current_search_context(app: &App) -> Option<(&SearchResult, MatchLayout)> {
    let result = app.search_results.get(app.search_cursor)?;
    let entry = &app.entries[result.entry_index];
    Some((result, SearchEngine::build_layout_for_result(entry)))
}

fn render_description(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(entry) = app.current_entry() else {
        app.description_page_size = area.height as usize;
        app.description_max_scroll = 0;
        app.description_scroll = 0;
        app.description_wrap_width = 0;
        app.description_needs_scroll_sync = false;
        return;
    };
    let text_area = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(1),
        ..area
    };
    let search_context = if app.mode == Mode::Search {
        current_search_context(app)
    } else {
        None
    };

    let mut lines: Vec<Line> = Vec::new();
    let mut plain_lines: Vec<String> = Vec::new();
    let mut plain_line_highlights: Vec<Vec<usize>> = Vec::new();
    let mut description_offset = 0usize;
    for body_line in &entry.body_lines {
        match body_line {
            BodyLine::DisplayName(raw_line) => {
                let description_indices =
                    search_context
                        .as_ref()
                        .map_or_else(Vec::new, |(result, layout)| {
                            SearchEngine::highlight_indices_for_line(
                                &result.highlight_indices,
                                layout.description.as_ref(),
                                description_offset,
                                raw_line,
                            )
                        });
                let display_name_indices =
                    search_context
                        .as_ref()
                        .map_or_else(Vec::new, |(result, layout)| {
                            let mut indices = slice_highlight_indices(
                                &result.highlight_indices,
                                layout.display_name.as_ref(),
                            );
                            for idx in &mut indices {
                                *idx += 2;
                            }
                            indices.retain(|idx| *idx < raw_line.chars().count());
                            indices
                        });
                let indices = merge_highlight_indices(description_indices, display_name_indices);
                lines.push(highlight_line(
                    raw_line,
                    &indices,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                plain_lines.push(raw_line.clone());
                plain_line_highlights.push(indices);
                description_offset += raw_line.chars().count() + 1;
            }
            BodyLine::Text(line) => {
                let indices = search_context
                    .as_ref()
                    .map_or_else(Vec::new, |(result, layout)| {
                        SearchEngine::highlight_indices_for_line(
                            &result.highlight_indices,
                            layout.description.as_ref(),
                            description_offset,
                            line,
                        )
                    });
                lines.push(highlight_line(line, &indices, Style::default()));
                plain_lines.push(line.clone());
                plain_line_highlights.push(indices);
                description_offset += line.chars().count() + 1;
            }
            BodyLine::Template(i) => {
                let template = &entry.templates[*i];
                let indices = search_context
                    .as_ref()
                    .map_or_else(Vec::new, |(result, layout)| {
                        SearchEngine::highlight_indices_for_line(
                            &result.highlight_indices,
                            layout.description.as_ref(),
                            description_offset,
                            &template.raw_line,
                        )
                    });
                lines.push(highlight_line(
                    &template.raw_line,
                    &indices,
                    Style::default().fg(Color::Cyan),
                ));
                plain_lines.push(template.raw_line.clone());
                plain_line_highlights.push(indices);
                description_offset += template.raw_line.chars().count() + 1;
            }
            BodyLine::Command(i) => {
                let mut plain_command = format!("> {}", entry.enrich_commands[*i]);
                let mut cmd_spans = vec![Span::styled(
                    plain_command.clone(),
                    Style::default().fg(Color::DarkGray),
                )];
                if let Some(Some(status)) = entry.enriched_status.get(*i) {
                    plain_command.push_str(&format!(" ({})", status));
                    cmd_spans.push(Span::styled(
                        format!(" ({})", status),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::from(cmd_spans));
                plain_lines.push(plain_command);
                plain_line_highlights.push(Vec::new());
                if let Some(output) = entry.enriched_output.get(*i) {
                    if !output.is_empty() {
                        let range = search_context
                            .as_ref()
                            .and_then(|(_, layout)| layout.enriched_output.get(*i))
                            .and_then(|range| range.as_ref());
                        let mut output_offset = 0usize;
                        for out_line in output.lines() {
                            let indices =
                                search_context
                                    .as_ref()
                                    .map_or_else(Vec::new, |(result, _)| {
                                        SearchEngine::highlight_indices_for_line(
                                            &result.highlight_indices,
                                            range,
                                            output_offset,
                                            out_line,
                                        )
                                    });
                            lines.push(highlight_line(out_line, &indices, Style::default()));
                            plain_lines.push(out_line.to_string());
                            plain_line_highlights.push(indices);
                            output_offset += out_line.chars().count() + 1;
                        }
                    }
                }
            }
        }
    }

    let page_size = text_area.height as usize;
    let wrap_width = text_area.width as usize;
    let layout_changed =
        app.description_page_size != page_size || app.description_wrap_width != wrap_width;
    app.description_page_size = page_size;
    app.description_wrap_width = wrap_width;
    let wrapped_plain_lines = wrap_plain_lines(&plain_lines, wrap_width);
    let total_lines = wrapped_plain_lines.len();
    app.description_max_scroll = total_lines.saturating_sub(page_size);
    let needs_scroll_sync =
        app.description_needs_scroll_sync || (app.mode == Mode::Search && layout_changed);
    if needs_scroll_sync {
        if let Some(target_line) =
            first_highlighted_wrapped_line(&plain_lines, &plain_line_highlights, wrap_width)
        {
            app.description_scroll =
                viewport_start(app.description_scroll, target_line, page_size, total_lines);
        }
        app.description_needs_scroll_sync = false;
    }
    app.description_scroll = app.description_scroll.min(app.description_max_scroll);

    f.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((app.description_scroll as u16, 0)),
        text_area,
    );

    if let Some(percent) = description_progress_text(
        &wrapped_plain_lines,
        app.description_scroll,
        page_size,
        text_area.width as usize,
    ) {
        let width = percent.chars().count() as u16;
        let x = text_area.x + text_area.width.saturating_sub(width);
        f.render_widget(
            Paragraph::new(percent).style(Style::default().fg(Color::DarkGray)),
            Rect {
                x,
                y: text_area.y,
                width,
                height: 1,
            },
        );
    }
}

fn render_focus_marker(f: &mut Frame, app: &App, left_pane: Rect) {
    let symbol = if app.mode == Mode::Editor {
        match &app.editor {
            Some(ed) if ed.textarea.lines().join("\n") != ed.original_content => "*",
            _ => "e",
        }
    } else {
        match app.focus {
            PaneFocus::Tree => "<",
            PaneFocus::Description => ">",
        }
    };
    let x = left_pane.x + left_pane.width.saturating_sub(1);
    let y = left_pane.y;
    f.render_widget(
        Paragraph::new(symbol).style(Style::default()),
        Rect {
            x,
            y,
            width: 1,
            height: 1,
        },
    );
}

fn render_template_picker(f: &mut Frame, app: &App, area: Rect) {
    let Some(picker) = &app.template_picker else {
        return;
    };

    let popup_area = template_picker_area(area, picker.choices.len());

    f.render_widget(Clear, popup_area);
    let block = Block::default().borders(Borders::ALL).title(" Templates ");
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let lines: Vec<Line> = picker
        .choices
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(i, choice)| {
            let selected = i == picker.cursor;
            let style = if selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let text = if choice.has_explicit_label {
                format!("{}: {}", choice.label, choice.value)
            } else {
                choice.value.clone()
            };
            Line::from(vec![
                Span::styled(if selected { "> " } else { "  " }, style),
                Span::styled(text, style),
            ])
        })
        .collect();

    f.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Left),
        inner,
    );
}

fn render_help_overlay(f: &mut Frame, app: &mut App, area: Rect) {
    if app.overlay != Some(Overlay::Help) {
        return;
    }

    let popup_area = help_overlay_area(area);
    let content_lines = help_overlay_content_lines();
    let page_size = popup_area.height.saturating_sub(2) as usize;
    let total_lines = content_lines.len();
    let max_scroll = total_lines.saturating_sub(page_size);
    app.help_state.page_size = page_size;
    app.help_state.max_scroll = max_scroll;
    app.help_state.scroll = app.help_state.scroll.min(max_scroll);
    let scroll = app.help_state.scroll;

    f.render_widget(Clear, popup_area);
    let block = Block::default().borders(Borders::ALL).title(" Help ");
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    render_help_column(f, inner, &content_lines, scroll);
    render_help_progress(f, inner, scroll, page_size, total_lines);
}

fn render_help_column(f: &mut Frame, area: Rect, lines: &[Line], scroll: usize) {
    f.render_widget(
        Paragraph::new(Text::from(lines.to_vec()))
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        area,
    );
}

fn render_help_progress(
    f: &mut Frame,
    area: Rect,
    scroll: usize,
    page_size: usize,
    total_lines: usize,
) {
    if page_size == 0 || total_lines <= page_size || area.width == 0 {
        return;
    }

    let visible_end = (scroll + page_size).min(total_lines);
    let percent = format!("{}%", ((visible_end * 100) / total_lines).min(100));
    let width = percent.chars().count() as u16;
    if width > area.width {
        return;
    }

    f.render_widget(
        Paragraph::new(percent).style(Style::default().fg(Color::DarkGray)),
        Rect {
            x: area.x + area.width.saturating_sub(width),
            y: area.y,
            width,
            height: 1,
        },
    );
}

fn help_overlay_area(area: Rect) -> Rect {
    let compact = area.width < 100 || area.height < 28;
    if compact {
        return area;
    }

    let width = area
        .width
        .saturating_mul(80)
        .saturating_div(100)
        .clamp(80, 120);
    let height = area
        .height
        .saturating_mul(80)
        .saturating_div(100)
        .clamp(22, 36);
    centered_rect(width, height, area)
}

fn help_overlay_content_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (section_index, section) in HELP_SECTIONS.iter().enumerate() {
        if section_index > 0 {
            lines.push(Line::raw(""));
        }

        lines.push(Line::from(vec![Span::styled(
            section.title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));

        for (keys, description) in section.rows {
            lines.push(Line::from(vec![
                Span::styled(format!("{keys:<18}"), Style::default().fg(Color::White)),
                Span::styled(*description, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }
    lines
}

fn handle_help_overlay_mouse(
    app: &mut App,
    mouse: MouseEvent,
    layout: UiLayout,
    last_mouse_click: &mut Option<MouseClickState>,
) {
    let area = Rect {
        x: 0,
        y: 0,
        width: layout
            .left_pane
            .width
            .saturating_add(layout.right_pane.width),
        height: layout.search_bar.height + layout.left_pane.height + layout.status_bar.height,
    };
    let popup_area = help_overlay_area(area);

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if !rect_contains(popup_area, mouse.column, mouse.row) {
                app.overlay = None;
            }
            *last_mouse_click = None;
        }
        MouseEventKind::ScrollDown => {
            if rect_contains(popup_area, mouse.column, mouse.row) {
                app.help_state.scroll =
                    (app.help_state.scroll + MOUSE_WHEEL_STEP).min(app.help_state.max_scroll);
            }
            *last_mouse_click = None;
        }
        MouseEventKind::ScrollUp => {
            if rect_contains(popup_area, mouse.column, mouse.row) {
                app.help_state.scroll = app.help_state.scroll.saturating_sub(MOUSE_WHEEL_STEP);
            }
            *last_mouse_click = None;
        }
        _ => {}
    }
}

fn template_picker_area(area: Rect, choice_count: usize) -> Rect {
    let popup_width = area
        .width
        .saturating_mul(60)
        .saturating_div(100)
        .clamp(24, 80);
    let popup_height_max = area.height.saturating_sub(2).max(4);
    let popup_height = (choice_count as u16 + 2).clamp(4, popup_height_max);
    centered_rect(popup_width, popup_height, area)
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let key_style = Style::default().fg(Color::White);
    let text_style = Style::default().fg(Color::DarkGray);
    let mut spans = match app.mode {
        Mode::Onboarding => vec![
            Span::styled("enter", key_style),
            Span::styled(" create demo  ", text_style),
            Span::styled("esc", key_style),
            Span::styled(" quit", text_style),
        ],
        Mode::Editor => vec![
            Span::styled("ctrl+s", key_style),
            Span::styled(" save  ", text_style),
            Span::styled("esc", key_style),
            Span::styled(" discard", text_style),
        ],
        _ => match app.focus {
            PaneFocus::Tree if app.selected_entry_index.is_some() => vec![
                Span::styled("↑ ↓ ← →", key_style),
                Span::styled(" navigation  ", text_style),
                Span::styled("enter", key_style),
                Span::styled(" select  ", text_style),
                Span::styled("ctrl+e", key_style),
                Span::styled(" edit", text_style),
            ],
            PaneFocus::Tree => vec![
                Span::styled("↑ ↓ ← →", key_style),
                Span::styled(" navigation", text_style),
            ],
            PaneFocus::Description if app.selected_entry_index.is_some() => vec![
                Span::styled("↑ ↓", key_style),
                Span::styled(" navigation  ", text_style),
                Span::styled("←", key_style),
                Span::styled(" back  ", text_style),
                Span::styled("enter", key_style),
                Span::styled(" select  ", text_style),
                Span::styled("ctrl+e", key_style),
                Span::styled(" edit", text_style),
            ],
            PaneFocus::Description => vec![
                Span::styled("↑ ↓", key_style),
                Span::styled(" navigation  ", text_style),
                Span::styled("←", key_style),
                Span::styled(" back", text_style),
            ],
        },
    };
    spans.push(Span::styled("  ctrl+h", key_style));
    spans.push(Span::styled(" help", text_style));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::{
        description_progress_text, first_highlighted_wrapped_line, viewport_start, wrap_plain_lines,
    };

    #[test]
    fn viewport_does_not_scroll_while_cursor_stays_inside_window() {
        assert_eq!(viewport_start(6, 6, 5, 20), 6);
        assert_eq!(viewport_start(6, 7, 5, 20), 6);
        assert_eq!(viewport_start(6, 10, 5, 20), 6);
    }

    #[test]
    fn viewport_scrolls_only_when_cursor_crosses_window_edge() {
        assert_eq!(viewport_start(6, 5, 5, 20), 5);
        assert_eq!(viewport_start(6, 11, 5, 20), 7);
    }

    #[test]
    fn wrap_plain_lines_splits_long_lines_by_width() {
        assert_eq!(
            wrap_plain_lines(&[String::from("abcdef"), String::new()], 3),
            vec!["abc", "def", ""]
        );
    }

    #[test]
    fn description_progress_hidden_when_it_overlaps_content() {
        let wrapped = vec![String::from("123456789"), String::from("tail")];
        assert_eq!(description_progress_text(&wrapped, 0, 1, 10), None);
        assert_eq!(
            description_progress_text(&wrapped, 0, 1, 12),
            Some(String::from("50%"))
        );
    }

    #[test]
    fn first_highlighted_wrapped_line_accounts_for_wrapping() {
        let lines = vec![
            String::from("Title"),
            String::from("abcdefgh"),
            String::from("tail"),
        ];
        let highlights = vec![vec![], vec![5], vec![]];
        assert_eq!(
            first_highlighted_wrapped_line(&lines, &highlights, 4),
            Some(3)
        );
    }

    #[test]
    fn first_highlighted_wrapped_line_skips_non_matching_lines() {
        let lines = vec![String::new(), String::from("abcd"), String::from("efgh")];
        let highlights = vec![vec![], vec![], vec![1]];
        assert_eq!(
            first_highlighted_wrapped_line(&lines, &highlights, 4),
            Some(2)
        );
    }
}
