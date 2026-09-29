//! Builds the tui-tree-widget item tree from the flat part list, mirroring
//! the CAD assembly hierarchy each part's path already encodes (folders
//! are just path segments -- there's no separate model for them).

use super::{part_flag, Part};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use std::collections::HashMap;
use storystick_core::nesting::Material;
use tui_tree_widget::TreeItem;

enum Node {
    Folder { order: Vec<String>, children: HashMap<String, Node> },
    Leaf { part_index: usize },
}

/// Deepest folder nesting a leaf row's column alignment compensates for
/// (see `leaf_line`). A leaf deeper than this still renders correctly,
/// just with its columns drifted right of the header -- there's no
/// correctness issue, only a cosmetic one, past this depth.
const MAX_EXPECTED_DEPTH: usize = 4;

/// Passed to `Tree::highlight_symbol` in `ui::draw_tree` -- shared from
/// here, not redeclared there, so the column math below and the actual
/// rendered symbol can never silently drift apart.
pub(super) const HIGHLIGHT_SYMBOL: &str = ">> ";

/// Before a leaf's own `text` even starts, tui-tree-widget draws, in
/// order: `highlight_symbol`'s width as reserved space on *every* row
/// (blank when unselected, so the selected row's own symbol doesn't shift
/// anything else -- easy to miss, since it's invisible except on the
/// selected row), then `depth * 2` indent columns, then a 2-wide
/// expand/collapse symbol. A leaf's `text` then adds its own `marker`
/// (2 wide, see `leaf_line`). None of this is visible to `header_line`,
/// which is a separate `Paragraph` drawn with no tree-widget involvement
/// at all -- so for the header's "Part" label to land in the same screen
/// column as every leaf's name, regardless of that leaf's depth,
/// `leaf_line` pads itself by `(MAX_EXPECTED_DEPTH - depth) * 2` extra
/// spaces before its marker, exactly cancelling out the depth-dependent
/// part of the widget's own indent -- and `header_line` pads by this same
/// total (highlight symbol + depth-independent remainder) up front, since
/// it has no widget-drawn prefix of its own to offset against.
const NAME_COLUMN_START: usize = HIGHLIGHT_SYMBOL.len() + MAX_EXPECTED_DEPTH * 2 + 4;

fn build_nodes(parts: &[Part]) -> (Vec<String>, HashMap<String, Node>) {
    let mut order: Vec<String> = Vec::new();
    let mut children: HashMap<String, Node> = HashMap::new();
    for (i, part) in parts.iter().enumerate() {
        let segments: Vec<&str> = part.path.split(" / ").collect();
        insert(&mut order, &mut children, &segments, i);
    }
    (order, children)
}

/// Returns the tree items to render, plus a map from a leaf's identifier
/// path (joined the same way `TreeState::selected()` joins it, " / "
/// between segments) to its part index -- the identifier can differ from
/// `Part::path` when a same-named sibling forced disambiguation (see
/// `insert`), so this map is the only correct way back to a part index.
pub(super) fn build(parts: &[Part], materials: &[Material]) -> (Vec<TreeItem<'static, String>>, HashMap<String, usize>) {
    let (order, children) = build_nodes(parts);
    let mut selection_index = HashMap::new();
    let items = to_items(&order, &children, parts, materials, &[], &mut selection_index);
    (items, selection_index)
}

/// Every folder's identifier path, for `TreeState::open` -- there's no
/// built-in "open everything" on `TreeState` (only `close_all`), so
/// expand-all is just opening every one of these.
pub(super) fn all_folder_paths(parts: &[Part]) -> Vec<Vec<String>> {
    let (order, children) = build_nodes(parts);
    let mut paths = Vec::new();
    collect_folder_paths(&order, &children, &[], &mut paths);
    paths
}

fn collect_folder_paths(order: &[String], children: &HashMap<String, Node>, prefix: &[String], out: &mut Vec<Vec<String>>) {
    for name in order {
        if let Node::Folder { order: sub_order, children: sub_children } = &children[name] {
            let mut path: Vec<String> = prefix.to_vec();
            path.push(name.clone());
            out.push(path.clone());
            collect_folder_paths(sub_order, sub_children, &path, out);
        }
    }
}

/// Two parts landing on the exact same path (or a leaf's own name
/// colliding with a sibling folder's) shouldn't happen given how Shapr3D
/// disambiguates sibling names -- but this is CAD-authored data, an
/// external boundary, so a genuine collision gets a display-only suffix
/// instead of silently dropping a part from the tree. The underlying
/// `Part::path` (and the assignment sidecar's real key, `Part::assignment_key`)
/// are never touched.
fn insert(order: &mut Vec<String>, children: &mut HashMap<String, Node>, segments: &[&str], part_index: usize) {
    let head = segments[0].to_string();
    if segments.len() == 1 {
        let mut key = head.clone();
        let mut n = 1;
        while children.contains_key(&key) {
            n += 1;
            key = format!("{head} ({n})");
        }
        order.push(key.clone());
        children.insert(key, Node::Leaf { part_index });
        return;
    }
    if !children.contains_key(&head) {
        order.push(head.clone());
        children.insert(head.clone(), Node::Folder { order: Vec::new(), children: HashMap::new() });
    }
    if let Some(Node::Folder { order: sub_order, children: sub_children }) = children.get_mut(&head) {
        insert(sub_order, sub_children, &segments[1..], part_index);
    }
}

fn count_flagged(order: &[String], children: &HashMap<String, Node>, parts: &[Part], materials: &[Material]) -> usize {
    order
        .iter()
        .map(|name| match &children[name] {
            Node::Leaf { part_index } => usize::from(part_flag(&parts[*part_index], materials).is_some()),
            Node::Folder { order: o, children: c } => count_flagged(o, c, parts, materials),
        })
        .sum()
}

fn to_items(
    order: &[String],
    children: &HashMap<String, Node>,
    parts: &[Part],
    materials: &[Material],
    path_prefix: &[String],
    selection_index: &mut HashMap<String, usize>,
) -> Vec<TreeItem<'static, String>> {
    order
        .iter()
        .map(|name| {
            let mut path: Vec<String> = path_prefix.to_vec();
            path.push(name.clone());
            match &children[name] {
                Node::Leaf { part_index } => {
                    selection_index.insert(path.join(" / "), *part_index);
                    let part = &parts[*part_index];
                    let flagged = part_flag(part, materials).is_some();
                    TreeItem::new_leaf(name.clone(), leaf_line(part, flagged, path_prefix.len()))
                }
                Node::Folder { order: sub_order, children: sub_children } => {
                    let flagged = count_flagged(sub_order, sub_children, parts, materials);
                    let sub_items = to_items(sub_order, sub_children, parts, materials, &path, selection_index);
                    TreeItem::new(name.clone(), folder_line(name, flagged), sub_items).expect("sibling names disambiguated in `insert`")
                }
            }
        })
        .collect()
}

/// Column header for the leaf rows, aligned field-for-field with
/// `leaf_line` -- the tree widget has no header row of its own, so this
/// is rendered as a fixed line above it (see `ui::draw_tree`).
pub(super) fn header_line() -> Line<'static> {
    let pad = " ".repeat(NAME_COLUMN_START);
    let text = format!("{pad}{:<32} {:>9}  {:>9}  {:>8}   {}", "Part", "Length", "Width", "Thick", "Material");
    Line::styled(text, Style::new().add_modifier(Modifier::BOLD))
}

fn leaf_line(part: &Part, flagged: bool, depth: usize) -> Line<'static> {
    let name = part.path.rsplit(" / ").next().unwrap_or(&part.path);
    let extra_indent = " ".repeat(MAX_EXPECTED_DEPTH.saturating_sub(depth) * 2);
    let marker = if flagged { "! " } else { "  " };
    let material = part.material.as_deref().unwrap_or("-");
    let text = format!(
        "{extra_indent}{marker}{name:<32} {:>9.4}  {:>9.4}  {:>8.4}   {material}",
        part.length_in, part.width_in, part.thickness_in
    );
    if flagged { Line::styled(text, Style::new().fg(Color::Red)) } else { Line::from(text) }
}

fn folder_line(name: &str, flagged: usize) -> Line<'static> {
    if flagged > 0 {
        Line::styled(format!("{name}  ({flagged} !)"), Style::new().fg(Color::Red))
    } else {
        Line::from(name.to_string())
    }
}
