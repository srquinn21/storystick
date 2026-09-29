//! Builds the tui-tree-widget item tree from the flat part list, mirroring
//! the CAD assembly hierarchy each part's path already encodes (folders
//! are just path segments -- there's no separate model for them).

use super::{part_flag, Part};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use storystick_core::nesting::Material;
use tui_tree_widget::TreeItem;

enum Node {
    Folder { order: Vec<String>, children: HashMap<String, Node> },
    Leaf { part_index: usize },
}

/// Deepest folder nesting a row's column alignment compensates for (see
/// `depth_indent`). A row deeper than this still renders correctly, just
/// with its columns drifted right of the header -- there's no
/// correctness issue, only a cosmetic one, past this depth.
const MAX_EXPECTED_DEPTH: usize = 4;

/// Passed to `Tree::highlight_symbol` in `ui::draw_tree` -- shared from
/// here, not redeclared there, so the column math below and the actual
/// rendered symbol can never silently drift apart. Empty: the selected
/// row's own blue highlight background is already an unambiguous marker,
/// and a real symbol here would reserve its width as permanent blank
/// indent on every row, selected or not.
pub(super) const HIGHLIGHT_SYMBOL: &str = "";

/// Before a row's own `text` even starts, tui-tree-widget draws, in
/// order: `highlight_symbol`'s width as reserved space on *every* row
/// (zero-width here, see `HIGHLIGHT_SYMBOL`), then `depth * 2` indent
/// columns, then a 2-wide expand/collapse symbol. A row's `text` then
/// adds its own 2-wide prefix
/// -- a real flag marker for a leaf (see `leaf_line`), two blank spaces
/// standing in for one on a folder (see `folder_line`), so both line up
/// identically. None of this is visible to `header_line`, which is a
/// separate `Paragraph` drawn with no tree-widget involvement at all --
/// so for the header's "Part" label to land in the same screen column as
/// every row's name, regardless of depth, `depth_indent` pads a row by
/// `(MAX_EXPECTED_DEPTH - depth) * 2` extra spaces before that 2-wide
/// prefix, exactly cancelling out the depth-dependent part of the
/// widget's own indent -- and `header_line` pads by this same total
/// (highlight symbol + depth-independent remainder) up front, since it
/// has no widget-drawn prefix of its own to offset against.
const NAME_COLUMN_START: usize = HIGHLIGHT_SYMBOL.len() + MAX_EXPECTED_DEPTH * 2 + 4;

/// Width of the name column itself (folder or leaf name, after the
/// indent/marker prefix), before the vertical rule and data columns.
const NAME_FIELD_WIDTH: usize = 32;

fn depth_indent(depth: usize) -> String {
    " ".repeat(MAX_EXPECTED_DEPTH.saturating_sub(depth) * 2)
}

/// The rule separating the tree (folders and part names) from the data
/// columns to its right. A fixed neutral color on every row -- header,
/// folder, or leaf, flagged or not -- so it reads as one continuous
/// architectural line down the whole tree rather than part of any one
/// row's own color.
fn divider() -> Span<'static> {
    Span::styled("\u{2502}", Style::new().fg(Color::DarkGray))
}

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
            let depth = path_prefix.len();
            match &children[name] {
                Node::Leaf { part_index } => {
                    selection_index.insert(path.join(" / "), *part_index);
                    let part = &parts[*part_index];
                    let flagged = part_flag(part, materials).is_some();
                    TreeItem::new_leaf(name.clone(), leaf_line(part, flagged, depth))
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
    let header_style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
    let pad = " ".repeat(NAME_COLUMN_START);
    let left = format!("{pad}{:<NAME_FIELD_WIDTH$}", "Part");
    let measurements = format!(" {:>9}  {:>9}  {:>8} ", "Length", "Width", "Thick");
    let material = " Material".to_string();
    Line::from(vec![
        Span::styled(left, header_style),
        divider(),
        Span::styled(measurements, header_style),
        divider(),
        Span::styled(material, header_style),
    ])
}

fn leaf_line(part: &Part, flagged: bool, depth: usize) -> Line<'static> {
    let name = part.path.rsplit(" / ").next().unwrap_or(&part.path);
    let marker = if flagged { "! " } else { "  " };
    let material = part.material.as_deref().unwrap_or("-");
    let left = format!("{}{marker}{name:<NAME_FIELD_WIDTH$}", depth_indent(depth));
    let measurements = format!(" {:>9.4}  {:>9.4}  {:>8.4} ", part.length_in, part.width_in, part.thickness_in);
    let material = format!(" {material}");
    let style = if flagged { Style::new().fg(Color::Red) } else { Style::default() };
    Line::from(vec![
        Span::styled(left, style),
        divider(),
        Span::styled(measurements, style),
        divider(),
        Span::styled(material, style),
    ])
}

/// Folders get their own bold accent color so the tree's shape reads at
/// a glance the same way a directory listing's does -- yellow rather
/// than LSCOLORS' traditional blue, which reads poorly against this
/// user's One Dark terminal theme. The `(N !)` flagged-descendant count
/// stays red regardless -- a warning needs to stay a warning color, not
/// blend into the folder's own.
///
/// Unlike `leaf_line`, a folder doesn't get the depth-compensating
/// indent or the vertical rule: it carries no data-column content to
/// separate from, and forcing every folder's name out to the same fixed
/// column leaf rows use would push shallow, top-level folders across
/// most of the screen for no reason. Folders just sit at their own
/// natural, tree-shaped indent, the same as any other tree/file browser.
fn folder_line(name: &str, flagged: usize) -> Line<'static> {
    let folder_style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    if flagged > 0 {
        Line::from(vec![Span::styled(name.to_string(), folder_style), Span::styled(format!("  ({flagged} !)"), Style::new().fg(Color::Red))])
    } else {
        Line::from(Span::styled(name.to_string(), folder_style))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn material(name: &str, thickness_in: f64) -> Material {
        Material { name: name.to_string(), thickness_mm: thickness_in * 25.4 }
    }

    /// A part with no material assigned (so `part_flag` always flags it by
    /// default -- see `part_flag`'s "no material assigned" case) and an
    /// `assignment_key` that just echoes `path`, since these tests never
    /// touch the sidecar.
    fn part(path: &str) -> Part {
        Part {
            path: path.to_string(),
            assignment_key: path.to_string(),
            raw_length_in: 30.0,
            raw_width_in: 20.0,
            raw_thickness_in: 0.75,
            length_in: 30.0,
            width_in: 20.0,
            thickness_in: 0.75,
            unreliable: false,
            thickness_mismatch: false,
            material: None,
            swapped: false,
        }
    }

    #[test]
    fn build_maps_each_leaf_identifier_to_its_part_index() {
        let parts = vec![part("Bench / Top"), part("Bench / Leg")];
        let (items, index) = build(&parts, &[]);

        assert_eq!(items.len(), 1, "both parts share one top-level folder, Bench");
        assert_eq!(index.len(), 2);
        assert_eq!(index["Bench / Top"], 0);
        assert_eq!(index["Bench / Leg"], 1);
    }

    #[test]
    fn build_disambiguates_a_colliding_leaf_name_with_a_suffix() {
        // Two parts sharing a bare (folder-less) name would otherwise
        // collide on the same tree identifier -- see `insert`'s docs.
        let parts = vec![part("Body"), part("Body")];
        let (_items, index) = build(&parts, &[]);

        assert_eq!(index.get("Body"), Some(&0));
        assert_eq!(index.get("Body (2)"), Some(&1), "second collision should get a ` (2)` suffix, not silently drop");
    }

    #[test]
    fn build_disambiguates_three_colliding_leaf_names_in_order() {
        let parts = vec![part("Bench / Body"), part("Bench / Body"), part("Bench / Body")];
        let (_items, index) = build(&parts, &[]);

        assert_eq!(index.get("Bench / Body"), Some(&0));
        assert_eq!(index.get("Bench / Body (2)"), Some(&1));
        assert_eq!(index.get("Bench / Body (3)"), Some(&2));
    }

    #[test]
    fn all_folder_paths_lists_every_folder_at_every_depth() {
        let parts = vec![part("Bench / Carcasses / Carcass A / Body"), part("Bench / Doors / Door A / Body")];
        let paths: HashSet<Vec<String>> = all_folder_paths(&parts).into_iter().collect();

        let expected: HashSet<Vec<String>> = [
            vec!["Bench".to_string()],
            vec!["Bench".to_string(), "Carcasses".to_string()],
            vec!["Bench".to_string(), "Carcasses".to_string(), "Carcass A".to_string()],
            vec!["Bench".to_string(), "Doors".to_string()],
            vec!["Bench".to_string(), "Doors".to_string(), "Door A".to_string()],
        ]
        .into_iter()
        .collect();
        assert_eq!(paths, expected, "leaf names (Body) must never appear as folder paths");
    }

    #[test]
    fn all_folder_paths_is_empty_for_a_flat_tree_of_bare_leaves() {
        let parts = vec![part("Body A"), part("Body B")];
        assert!(all_folder_paths(&parts).is_empty());
    }

    #[test]
    fn count_flagged_sums_leaves_recursively_across_nested_folders() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut parts = vec![
            part("Bench / Carcasses / Carcass A / Flagged"),
            part("Bench / Carcasses / Carcass A / Resolved"),
            part("Bench / Doors / Door A / Flagged"),
        ];
        parts[1].material = Some("Baltic Birch 3/4".to_string());

        let (order, children) = build_nodes(&parts);
        assert_eq!(count_flagged(&order, &children, &parts, &materials), 2, "two of the three parts have no material assigned");
    }

    #[test]
    fn count_flagged_is_zero_once_every_part_in_the_subtree_is_resolved() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut parts = vec![part("Bench / A"), part("Bench / B")];
        for p in &mut parts {
            p.material = Some("Baltic Birch 3/4".to_string());
        }

        let (order, children) = build_nodes(&parts);
        assert_eq!(count_flagged(&order, &children, &parts, &materials), 0);
    }
}
