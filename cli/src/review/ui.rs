use super::{tree, App, BulkState, PickerTarget, PrintField};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use tui_tree_widget::Tree;

/// The tree screen's resting bottom-line help, as (key, action) pairs so
/// `help_line` can color the key distinctly from what it does. No part
/// count here -- that's already the top-right title's `resolved` count
/// (see `draw_tree`), and a bare `42` here with no unit would be a
/// second, differently-shaped answer to the same question.
const TREE_HELP: &[(&str, &str)] = &[
    ("j/k", "move"),
    ("h/l", "fold"),
    ("e/c", "expand/collapse all"),
    ("Enter/m", "assign"),
    ("b", "bulk edit"),
    ("g", "swap L/W"),
    ("Ctrl-d/u", "page"),
    ("s", "save"),
    ("p", "print"),
    ("q", "quit"),
];

/// Shown on the bottom status line while bulk-edit's tag list (see
/// `BulkState`) has focus.
const BULK_TAG_HELP: &[(&str, &str)] =
    &[("j/k", "move"), ("Enter", "set material"), ("Esc", "done")];

/// Shown on the bottom status line while a material picker (`App::picker`,
/// either a single part's or a bulk-edit tag's) has focus.
const PICKER_HELP: &[(&str, &str)] = &[("j/k", "move"), ("Enter", "confirm"), ("Esc", "cancel")];

/// Shown on the bottom status line (see `draw_status`) while the
/// print-settings popup has focus, replacing the tree's own keyboard
/// help -- none of those keys apply while this popup is open.
const PRINT_HELP: &[(&str, &str)] = &[
    ("Tab", "switch field"),
    ("Enter", "print"),
    ("Esc", "cancel"),
];

/// Shown on the bottom status line while the "save before exiting?"
/// popup has focus. `y`/`Enter` are listed together since `Enter` is
/// just the capitalized default in the popup's own "[Y/n]" -- see
/// `run`'s `confirm_quit` handling.
const QUIT_HELP: &[(&str, &str)] = &[
    ("y/Enter", "save & quit"),
    ("n", "quit without saving"),
    ("Esc", "cancel"),
];

/// Plain-text rendering of a help line's (key, action) pairs -- used for
/// `App::default_status`, which is compared for equality (see
/// `App::expire_status`) to tell a transient message apart from the
/// resting help line, so it can't itself carry color/span structure.
pub(super) fn tree_help_text() -> String {
    plain_help_text(TREE_HELP)
}

fn plain_help_text(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, action)| format!("{key} {action}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders a help line with each key in blue and what it does in dark
/// gray, so the keys themselves jump out at a glance.
fn help_line(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(pairs.len() * 3);
    for (i, (key, action)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        spans.push(Span::styled(key.to_string(), Style::new().fg(Color::Blue)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            action.to_string(),
            Style::new().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // A trailing blank row so the status line doesn't sit flush against a
    // terminal multiplexer's own status bar (tmux, etc.) directly below --
    // that last row is simply never drawn into, left as the terminal's
    // default blank background.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    draw_tree(frame, chunks[0], app);
    draw_status(frame, chunks[1], app);

    // Bulk-edit's tag list is drawn before the picker so that, when a tag
    // hands off to the shared material picker (`App::picker`, left open
    // underneath -- see `BulkState`), the picker renders on top of the
    // tag list rather than being hidden behind it.
    if app.bulk.is_some() {
        draw_bulk(frame, area, app);
    }
    if app.picker.is_some() {
        draw_picker(frame, area, app);
    }
    if app.print_settings.is_some() {
        draw_print_settings(frame, area, app);
    }
    if app.confirm_quit {
        draw_confirm_quit(frame, area);
    }
}

fn draw_tree(frame: &mut Frame, area: Rect, app: &mut App) {
    let (items, selection_index) = tree::build(&app.parts, &app.materials);
    app.selection_index = selection_index;
    let (resolved, total) = app.resolved_counts();
    // Red until every part is cleanly resolved (material assigned, no
    // `part_flag` left standing), then green -- the count is the one
    // thing in the title actually worth a glance-and-go signal; the rest
    // of the title is just identifying which file this is.
    let count_color = if total > 0 && resolved == total {
        Color::Green
    } else {
        Color::Red
    };
    // The full path is mostly the same directory over and over across a
    // multi-file project (e.g. this model's own `-Carcass`/`-Uppers`
    // siblings) -- the file name is the part that actually distinguishes
    // one run's title from another.
    let file_name = app
        .step_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| app.step_path.display().to_string());
    let left_spans = vec![
        Span::styled(
            " storystick -- ",
            Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
        Span::raw(file_name),
    ];
    // `[modified]` goes last so it sits at the very right edge of the
    // border, past the resolved count -- the most urgent thing (unsaved
    // changes) should be the last thing pushed off the edge, not buried
    // in the middle of the right-aligned group.
    let mut right_spans = vec![
        Span::styled(
            format!("{resolved}/{total}"),
            Style::new().fg(count_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" resolved"),
    ];
    if app.dirty {
        right_spans.push(Span::styled(" [modified]", Style::new().fg(Color::Yellow)));
    }
    right_spans.push(Span::raw(" "));
    let right_title = Line::from(right_spans).right_aligned();

    // The tree widget has no header row of its own, so the outer block is
    // rendered here directly (not via `Tree::block`) to make room for a
    // fixed header line, column-aligned with each leaf row, above it.
    // Two separate titles (rather than one line with padding in between)
    // so the resolved count stays pinned to the border's right edge
    // regardless of how long the file name is.
    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(left_spans))
        .title_top(right_title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    frame.render_widget(Paragraph::new(tree::header_line()), rows[0]);

    let widget = Tree::new(&items)
        .expect("sibling names disambiguated in tree::insert")
        .highlight_style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD))
        .highlight_symbol(tree::HIGHLIGHT_SYMBOL);

    // What a page-scroll (Ctrl-d/u) should actually jump by.
    app.last_tree_height = rows[1].height;

    frame.render_stateful_widget(widget, rows[1], &mut app.tree_state);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    // While any popup has focus, the tree's own keys (j/k, e/c, m, g,
    // s...) don't apply -- showing them here would be actively misleading
    // about what the keyboard does right now. A transient message (a save
    // confirmation, an error) is free-form prose, not key/action pairs,
    // so it renders plain rather than through `help_line`. `picker` is
    // checked ahead of `bulk` since a bulk-edit tag hands off to the
    // picker while leaving itself in place underneath (see `BulkState`),
    // so both can be `Some` at once and the picker is the one with focus.
    let line = if app.print_settings.is_some() {
        help_line(PRINT_HELP)
    } else if app.confirm_quit {
        help_line(QUIT_HELP)
    } else if app.picker.is_some() {
        help_line(PICKER_HELP)
    } else if matches!(app.bulk, Some(BulkState::PickTag { .. })) {
        help_line(BULK_TAG_HELP)
    } else if app.status_is_default() {
        help_line(TREE_HELP)
    } else {
        Line::from(app.status.as_str())
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

/// A popup's width should fit its longest line -- title included -- so a
/// long tag or material name doesn't get clipped at some fixed guess;
/// `centered_rect` still clamps the result down to the terminal's own
/// width on a narrow screen.
fn popup_width(min: u16, lines: impl Iterator<Item = usize>) -> u16 {
    let longest = lines.max().unwrap_or(0) as u16;
    (longest + 4).max(min)
}

fn draw_picker(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(picker) = &mut app.picker else {
        return;
    };
    let title = match &picker.target {
        PickerTarget::Part(_) => " pick a material ".to_string(),
        PickerTarget::Tag(tag) => format!(" pick a material for {tag} "),
    };
    let width = popup_width(
        50,
        picker
            .options
            .iter()
            .map(|o| o.chars().count())
            .chain(std::iter::once(title.chars().count())),
    );
    let popup = centered_rect(width, (picker.options.len() as u16 + 4).min(20), area);

    let items: Vec<ListItem> = picker
        .options
        .iter()
        .map(|name| ListItem::new(name.as_str()))
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(list, popup, &mut picker.list_state);
}

/// Bulk-edit's own screen (see `BulkState`): the tag list, each row
/// showing its current rule material if one is set. Picking a material
/// is just `draw_picker` with a `PickerTarget::Tag`, drawn on top once
/// `App::bulk_pick_tag` hands off to `App::picker` -- `draw` orders the
/// two calls so that picker isn't hidden behind this one.
fn draw_bulk(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(BulkState::PickTag { tags, list_state }) = &mut app.bulk else {
        return;
    };
    let title = " bulk edit ";
    let labels: Vec<String> = tags
        .iter()
        .map(|(tag, count, material)| match material {
            Some(m) => format!("{tag}  ({count})  -- {m}"),
            None => format!("{tag}  ({count})"),
        })
        .collect();
    let width = popup_width(
        50,
        labels
            .iter()
            .map(|l| l.chars().count())
            .chain(std::iter::once(title.chars().count())),
    );
    let popup = centered_rect(width, (tags.len() as u16 + 4).min(20), area);
    let items: Vec<ListItem> = labels.into_iter().map(ListItem::new).collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");
    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(list, popup, list_state);
}

fn draw_confirm_quit(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(46, 4, area);
    let lines = vec![
        Line::from(""),
        Line::styled(
            " Save changes before exiting?  [Y/n]",
            Style::new().fg(Color::Yellow),
        ),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Unsaved Changes ");
    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, popup);
    frame.render_widget(paragraph, popup);
}

fn draw_print_settings(frame: &mut Frame, area: Rect, app: &App) {
    let Some(ps) = &app.print_settings else {
        return;
    };
    let popup = centered_rect(56, 6, area);

    let field_line = |label: &str, value: &str, focused: bool| {
        let cursor = if focused { "_" } else { "" };
        let text = format!(" {label:<22}{value}{cursor}");
        if focused {
            Line::styled(
                text,
                Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD),
            )
        } else {
            Line::from(text)
        }
    };

    let lines = vec![
        field_line("Kerf (in):", &ps.kerf_in, ps.focus == PrintField::Kerf),
        Line::from(""),
        field_line(
            "Trim allowance (in):",
            &ps.trim_allowance_in,
            ps.focus == PrintField::TrimAllowance,
        ),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Print Project Plan ");
    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, popup);
    frame.render_widget(paragraph, popup);
}
