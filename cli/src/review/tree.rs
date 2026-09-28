//! Builds the tui-tree-widget item tree from the flat part list, mirroring
//! the CAD assembly hierarchy each part's path already encodes (folders
//! are just path segments -- there's no separate model for them).

use super::{part_flag, Part};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use std::collections::HashMap;
use storystick_core::nesting::Material;
use tui_tree_widget::TreeItem;

enum Node {
    Folder { order: Vec<String>, children: HashMap<String, Node> },
    Leaf { part_index: usize },
}

/// Returns the tree items to render, plus a map from a leaf's identifier
/// path (joined the same way `TreeState::selected()` joins it, " / "
/// between segments) to its part index -- the identifier can differ from
/// `Part::path` when a same-named sibling forced disambiguation (see
/// `insert`), so this map is the only correct way back to a part index.
pub(super) fn build(parts: &[Part], materials: &[Material]) -> (Vec<TreeItem<'static, String>>, HashMap<String, usize>) {
    let mut order: Vec<String> = Vec::new();
    let mut children: HashMap<String, Node> = HashMap::new();
    for (i, part) in parts.iter().enumerate() {
        let segments: Vec<&str> = part.path.split(" / ").collect();
        insert(&mut order, &mut children, &segments, i);
    }
    let mut selection_index = HashMap::new();
    let items = to_items(&order, &children, parts, materials, &[], &mut selection_index);
    (items, selection_index)
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
                    TreeItem::new_leaf(name.clone(), leaf_line(part, flagged))
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

fn leaf_line(part: &Part, flagged: bool) -> Line<'static> {
    let name = part.path.rsplit(" / ").next().unwrap_or(&part.path);
    let marker = if flagged { "! " } else { "  " };
    let material = part.material.as_deref().unwrap_or("-");
    let text = format!(
        "{marker}{name:<32} {:>9.4} x {:>9.4} x {:>8.4}   {material}",
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
