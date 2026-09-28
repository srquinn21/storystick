use super::{tree, App};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use tui_tree_widget::Tree;

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3), Constraint::Length(1)]).split(area);

    draw_tree(frame, chunks[0], app);
    draw_status(frame, chunks[1], app);

    if app.picker.is_some() {
        draw_picker(frame, area, app);
    }
}

fn draw_tree(frame: &mut Frame, area: Rect, app: &mut App) {
    let (items, selection_index) = tree::build(&app.parts, &app.materials);
    app.selection_index = selection_index;
    let (assigned, total) = app.assigned_counts();
    let title = format!(
        " storystick -- {}{}   {assigned}/{total} assigned ",
        app.step_path.display(),
        if app.dirty { " [modified]" } else { "" },
    );

    let widget = Tree::new(&items)
        .expect("sibling names disambiguated in tree::insert")
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    // Borders eat two rows; the rest is what a page-scroll (Ctrl-d/u)
    // should actually jump by.
    app.last_tree_height = area.height.saturating_sub(2);

    frame.render_stateful_widget(widget, area, &mut app.tree_state);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Paragraph::new(Line::from(app.status.as_str())), area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn draw_picker(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(picker) = &mut app.picker else { return };
    let popup = centered_rect(50, (picker.options.len() as u16 + 4).min(20), area);

    let items: Vec<ListItem> = picker.options.iter().map(|name| ListItem::new(name.as_str())).collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" pick a material (Enter to confirm, Esc to cancel) "))
        .highlight_style(Style::new().bg(Color::Blue).add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(list, popup, &mut picker.list_state);
}
