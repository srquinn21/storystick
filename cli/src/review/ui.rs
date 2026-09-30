use super::{tree, App, BulkState, Command, PartEditField, PickerTarget, PrintField};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

/// The main screen's resting bottom-line help -- deliberately just a
/// pointer to the full reference (`?`, see `HELP_SECTIONS`/
/// `draw_help_screen`) rather than trying to cram every binding into one
/// line. This line is also `App::default_status`, so shrinking it also
/// frees up the status line for transient messages (a save confirmation,
/// an error) most of the time.
const TREE_HELP: &[(&str, &str)] = &[("?", "help")];

/// Every binding, grouped for the full-screen reference (`?`, see
/// `draw_help_screen`) -- the one authoritative, always-accurate list,
/// since the resting status line (`TREE_HELP`) no longer tries to be.
const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Movement",
        &[
            ("j / k", "move selection"),
            ("h / l", "up a level / drill in"),
            ("gg / G", "top / bottom"),
            ("Enter", "edit part / drill in"),
        ],
    ),
    (
        "Find",
        &[
            ("/", "fuzzy-jump to a part by name"),
            ("]f / [f", "next / previous flagged part"),
            ("n / N", "repeat the last jump, forward / back"),
        ],
    ),
    (
        "Commands",
        &[
            ("space, then b", "bulk edit"),
            ("space, then p", "print settings"),
            ("Ctrl-p", "command palette"),
        ],
    ),
    (
        "Part edit",
        &[
            ("Tab / j / k", "switch Material / Grain / Dimensions"),
            ("Enter", "edit the focused field"),
            (
                "1 / 2 / 3",
                "on Dimensions: swap L/W, L/T, or W/T (Enter = L/W)",
            ),
        ],
    ),
    (
        "Other",
        &[("w", "save"), ("q", "quit"), ("?", "this help screen")],
    ),
];

/// Shown on the bottom status line while the full help screen (`App::
/// help_open`) has focus.
const HELP_SCREEN_HELP: &[(&str, &str)] = &[("Esc / q / ?", "close")];

/// Shown on the bottom status line while bulk-edit's tag list (see
/// `BulkState`) has focus.
const BULK_TAG_HELP: &[(&str, &str)] =
    &[("j/k", "move"), ("Enter", "set material"), ("Esc", "done")];

/// Shown on the bottom status line while a material picker (`App::picker`,
/// either a single part's or a bulk-edit tag's) has focus.
const PICKER_HELP: &[(&str, &str)] = &[("j/k", "move"), ("Enter", "confirm"), ("Esc", "cancel")];

/// Shown on the bottom status line while a part's edit modal (`App::
/// part_edit`) has focus.
const PART_EDIT_HELP: &[(&str, &str)] = &[
    ("Tab/j/k", "switch field"),
    ("Enter", "edit field"),
    ("1/2/3", "on Dimensions: swap L/W, L/T, W/T"),
    ("Esc", "done"),
];

/// Shown on the bottom status line while the command palette (`App::
/// palette`) has focus.
const PALETTE_HELP: &[(&str, &str)] = &[
    ("type", "filter"),
    ("Up/Down", "move"),
    ("Enter", "run"),
    ("Esc", "cancel"),
];

/// Shown on the bottom status line while `/`'s own input line (`App::
/// jump`, while `editing`) has focus.
const NAME_JUMP_HELP: &[(&str, &str)] = &[
    ("type", "filter by name"),
    ("Enter", "jump"),
    ("Esc", "cancel"),
];

/// Shown on the bottom status line (see `draw_status`) while the
/// print-settings popup has focus, replacing the resting help line --
/// none of those keys apply while this popup is open.
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

    draw_assembly_list(frame, chunks[0], app);
    draw_status(frame, chunks[1], app);

    // Each of these draws on top of whatever's already on screen, in an
    // order chosen so a modal that hands off to another (bulk-edit and
    // part-edit both hand off to the material picker) is drawn first,
    // leaving the one it handed off to on top -- mirroring `run`'s own
    // key-handling priority.
    if app.bulk.is_some() {
        draw_bulk(frame, area, app);
    }
    if app.part_edit.is_some() {
        draw_part_edit(frame, area, app);
    }
    if app.picker.is_some() {
        draw_picker(frame, area, app);
    }
    if app.palette.is_some() {
        draw_command_palette(frame, area, app);
    }
    if matches!(&app.jump, Some(j) if j.editing) {
        draw_name_jump_prompt(frame, area, app);
    }
    if app.print_settings.is_some() {
        draw_print_settings(frame, area, app);
    }
    if app.confirm_quit {
        draw_confirm_quit(frame, area);
    }
    if app.help_open {
        draw_help_screen(frame, area);
    }
}

fn draw_assembly_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows = tree::rows_at(&app.parts, &app.materials, &app.breadcrumb)
        .expect("breadcrumb only ever holds paths this session's own navigation produced");
    let (resolved, total) = app.resolved_counts();
    // Red until every part is cleanly resolved (material assigned, no
    // `part_flag` left standing), then green -- the count is the one
    // thing in the title actually worth a glance-and-go signal.
    let count_color = if total > 0 && resolved == total {
        Color::Green
    } else {
        Color::Red
    };
    // The breadcrumb, not the file name, is the thing that actually
    // changes as you navigate -- there's normally exactly one STEP file
    // open at a time (see `docs/poc.md`'s single-full-project-export
    // direction), so "where am I" earns the title bar far more often
    // than "which file" does.
    let location = if app.breadcrumb.is_empty() {
        "Project root".to_string()
    } else {
        app.breadcrumb.join(" / ")
    };
    let left_spans = vec![
        Span::styled(
            " storystick -- ",
            Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
        Span::raw(location),
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

    // The row list has no header of its own, so the outer block is
    // rendered here directly (not via `List::block`) to make room for a
    // fixed header line, column-aligned with each leaf row, above it.
    // Two separate titles (rather than one line with padding in between)
    // so the resolved count stays pinned to the border's right edge
    // regardless of how long the breadcrumb is.
    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(left_spans))
        .title_top(right_title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let inner_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    frame.render_widget(Paragraph::new(tree::header_line()), inner_rows[0]);

    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let line = match row.kind {
                tree::RowKind::Leaf {
                    part_index,
                    flagged,
                } => tree::leaf_line(&app.parts[part_index], flagged),
                tree::RowKind::Folder { flagged } => tree::folder_line(&row.name, flagged),
            };
            ListItem::new(line)
        })
        .collect();
    let list =
        List::new(items).highlight_style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD));

    frame.render_stateful_widget(list, inner_rows[1], &mut app.list_state);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    // While any popup has focus, the main screen's own keys don't apply
    // -- showing them here would be actively misleading about what the
    // keyboard does right now. A transient message (a save confirmation,
    // an error) is free-form prose, not key/action pairs, so it renders
    // plain rather than through `help_line`. `picker` is checked ahead of
    // `bulk`/`part_edit` since both hand off to it while leaving
    // themselves in place underneath, so more than one can be `Some` at
    // once and the picker is the one with focus.
    let line = if app.print_settings.is_some() {
        help_line(PRINT_HELP)
    } else if app.confirm_quit {
        help_line(QUIT_HELP)
    } else if app.help_open {
        help_line(HELP_SCREEN_HELP)
    } else if app.picker.is_some() {
        help_line(PICKER_HELP)
    } else if matches!(app.bulk, Some(BulkState::PickTag { .. })) {
        help_line(BULK_TAG_HELP)
    } else if app.part_edit.is_some() {
        help_line(PART_EDIT_HELP)
    } else if app.palette.is_some() {
        help_line(PALETTE_HELP)
    } else if matches!(&app.jump, Some(j) if j.editing) {
        help_line(NAME_JUMP_HELP)
    } else if app.status_is_default() {
        // Right-aligned, unlike every other help line here -- it's just
        // a pointer to the real reference (`?`), not something worth the
        // same left-edge prominence as an active modal's own keys.
        help_line(TREE_HELP).right_aligned()
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

/// A single part's Material + Grain + Dimensions fields on one screen
/// (`App::part_edit`) -- Tab/`j`/`k` cycles which field is focused,
/// `Enter` acts on it (opens the material picker, toggles grain, or swaps
/// length/width -- the commonest dimension fix). While Dimensions has
/// focus, its line also shows `1`/`2`/`3`, the other two pairwise swaps
/// (see `App::swap_length_width` and friends). Drawn before `draw_picker`
/// in `draw` so a Material-triggered picker renders on top of this.
fn draw_part_edit(frame: &mut Frame, area: Rect, app: &App) {
    let Some(pe) = &app.part_edit else {
        return;
    };
    let part = &app.parts[pe.part_index];

    let material = part
        .material
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("-");
    let grain = if part.grain_along_length {
        "along length"
    } else {
        "along width"
    };
    let dimensions_focused = pe.focus == PartEditField::Dimensions;
    let dimensions = if dimensions_focused {
        format!(
            "L {:.4}  W {:.4}  T {:.4}   [1] L/W  [2] L/T  [3] W/T",
            part.length_in, part.width_in, part.thickness_in
        )
    } else {
        format!(
            "L {:.4}  W {:.4}  T {:.4}",
            part.length_in, part.width_in, part.thickness_in
        )
    };

    let field_text = |label: &str, value: &str| format!(" {label:<12}{value}");
    let material_text = field_text("Material:", material);
    let grain_text = field_text("Grain:", grain);
    let dimensions_text = field_text("Dimensions:", &dimensions);
    let title = format!(" {} ", part.path);

    let width = popup_width(
        60,
        [&material_text, &grain_text, &dimensions_text]
            .iter()
            .map(|s| s.chars().count())
            .chain(std::iter::once(title.chars().count())),
    );
    let popup = centered_rect(width, 8, area);

    let styled = |text: String, focused: bool| {
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
        styled(material_text, pe.focus == PartEditField::Material),
        Line::from(""),
        styled(grain_text, pe.focus == PartEditField::Grain),
        Line::from(""),
        styled(dimensions_text, dimensions_focused),
    ];

    let block = Block::default().borders(Borders::ALL).title(title);
    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, popup);
    frame.render_widget(paragraph, popup);
}

/// The `ctrl+p` command palette (`App::palette`): a query line filtering
/// `Command::all()` by fuzzy match, ranked best-first.
fn draw_command_palette(frame: &mut Frame, area: Rect, app: &App) {
    let Some(palette) = &app.palette else {
        return;
    };
    let labels: Vec<&str> = Command::all().iter().map(Command::label).collect();
    let title = format!(" command: {} ", palette.query);
    let width = popup_width(
        30,
        labels
            .iter()
            .map(|l| l.chars().count())
            .chain(std::iter::once(title.chars().count())),
    );
    let popup = centered_rect(width, (palette.matches.len() as u16 + 4).min(20), area);

    let items: Vec<ListItem> = palette
        .matches
        .iter()
        .map(|&i| ListItem::new(labels[i]))
        .collect();
    let mut list_state = ListState::default();
    list_state.select(Some(palette.selected));
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(list, popup, &mut list_state);
}

/// The `/` fuzzy-jump-by-name prompt (`App::filter`, while `editing`) --
/// no list on screen, since cycling through ranked matches happens via
/// `n`/`N` once this input line closes on `Enter`.
fn draw_name_jump_prompt(frame: &mut Frame, area: Rect, app: &App) {
    let Some(j) = &app.jump else {
        return;
    };
    let popup = centered_rect(50, 3, area);
    let position = if j.matches.is_empty() {
        "no matches".to_string()
    } else {
        format!("{}/{} matches", j.current + 1, j.matches.len())
    };
    let text = format!(" /{}", j.query);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {position} "));
    let paragraph = Paragraph::new(Line::from(text)).block(block);

    frame.render_widget(Clear, popup);
    frame.render_widget(paragraph, popup);
}

/// The full keybinding reference (bare `?`, `App::help_open`) -- a
/// static, grouped listing of `HELP_SECTIONS`, the one authoritative
/// place every binding is documented, so the resting status line
/// (`TREE_HELP`) doesn't have to try.
fn draw_help_screen(frame: &mut Frame, area: Rect) {
    let width = 56u16;
    let height = HELP_SECTIONS
        .iter()
        .map(|(_, pairs)| pairs.len() as u16 + 2)
        .sum::<u16>()
        + 2;
    let popup = centered_rect(width, height, area);

    let mut lines: Vec<Line> = Vec::new();
    for (section, pairs) in HELP_SECTIONS {
        lines.push(Line::styled(
            format!(" {section}"),
            Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ));
        for (key, action) in *pairs {
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled(format!("{key:<14}"), Style::new().fg(Color::Blue)),
                Span::styled(*action, Style::new().fg(Color::DarkGray)),
            ]));
        }
        lines.push(Line::from(""));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Keybindings ");
    let paragraph = Paragraph::new(lines).block(block);

    frame.render_widget(Clear, popup);
    frame.render_widget(paragraph, popup);
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
