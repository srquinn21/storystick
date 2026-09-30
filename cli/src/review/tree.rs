//! Builds the assembly hierarchy the review TUI navigates, mirroring the
//! CAD assembly hierarchy each part's path already encodes (folders are
//! just path segments -- there's no separate model for them). Every
//! folder is an assembly; a part with no children is just a trivial
//! one-part assembly, so there's no special-casing between the two here.
//!
//! The TUI shows one assembly's direct children at a time (see
//! `App::breadcrumb`/`App::current_rows` in `mod.rs`), not the whole tree
//! at once -- `rows_at` is the one function that answers "what's under
//! this path," and `locate_part`/`navigate_to_part` are the reverse
//! direction, turning a part index back into a breadcrumb.

use super::{part_flag, Part};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use storystick_core::nesting::Material;

enum Node {
    Folder {
        order: Vec<String>,
        children: HashMap<String, Node>,
    },
    Leaf {
        part_index: usize,
    },
}

/// Width of the name column itself (folder or leaf name), before the
/// vertical rule and data columns on a leaf row.
const NAME_FIELD_WIDTH: usize = 32;

/// Left padding shared by every row (folder or leaf) so names all start
/// in the same screen column regardless of kind -- a leaf's own flag
/// marker (`! `/`  `) is exactly this wide; a folder has no marker of
/// its own; and this is a flat, single-level list now, so there's no
/// per-row depth to additionally compensate for.
const ROW_PREFIX_WIDTH: usize = 2;

/// One row of the current assembly's direct children -- either a
/// sub-assembly (folder) or a part (leaf), never both.
pub(super) struct Row {
    pub(super) name: String,
    pub(super) kind: RowKind,
}

pub(super) enum RowKind {
    Folder { flagged: usize },
    Leaf { part_index: usize, flagged: bool },
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

/// Two parts landing on the exact same path (or a leaf's own name
/// colliding with a sibling folder's) shouldn't happen given how Shapr3D
/// disambiguates sibling names -- but this is CAD-authored data, an
/// external boundary, so a genuine collision gets a display-only suffix
/// instead of silently dropping a part from the tree. The underlying
/// `Part::path` (and the assignment sidecar's real key, `Part::assignment_key`)
/// are never touched.
fn insert(
    order: &mut Vec<String>,
    children: &mut HashMap<String, Node>,
    segments: &[&str],
    part_index: usize,
) {
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
        children.insert(
            head.clone(),
            Node::Folder {
                order: Vec::new(),
                children: HashMap::new(),
            },
        );
    }
    if let Some(Node::Folder {
        order: sub_order,
        children: sub_children,
    }) = children.get_mut(&head)
    {
        insert(sub_order, sub_children, &segments[1..], part_index);
    }
}

fn count_flagged(
    order: &[String],
    children: &HashMap<String, Node>,
    parts: &[Part],
    materials: &[Material],
) -> usize {
    order
        .iter()
        .map(|name| match &children[name] {
            Node::Leaf { part_index } => {
                usize::from(part_flag(&parts[*part_index], materials).is_some())
            }
            Node::Folder {
                order: o,
                children: c,
            } => count_flagged(o, c, parts, materials),
        })
        .sum()
}

/// Direct children of the assembly at `path` (an empty path means the
/// project root), in insertion order -- never the whole subtree, just
/// one level. `None` if `path` runs through a leaf, or names a segment
/// that doesn't exist (e.g. a stale breadcrumb -- callers that build
/// `path` from `locate_part`/`navigate_to_part` never hit this in
/// practice, since `Part::path` never changes mid-session).
pub(super) fn rows_at(parts: &[Part], materials: &[Material], path: &[String]) -> Option<Vec<Row>> {
    let (root_order, root_children) = build_nodes(parts);
    let mut order = &root_order;
    let mut children = &root_children;
    for segment in path {
        match children.get(segment)? {
            Node::Folder {
                order: o,
                children: c,
            } => {
                order = o;
                children = c;
            }
            Node::Leaf { .. } => return None,
        }
    }
    Some(
        order
            .iter()
            .map(|name| match &children[name] {
                Node::Leaf { part_index } => Row {
                    name: name.clone(),
                    kind: RowKind::Leaf {
                        part_index: *part_index,
                        flagged: part_flag(&parts[*part_index], materials).is_some(),
                    },
                },
                Node::Folder {
                    order: sub_order,
                    children: sub_children,
                } => Row {
                    name: name.clone(),
                    kind: RowKind::Folder {
                        flagged: count_flagged(sub_order, sub_children, parts, materials),
                    },
                },
            })
            .collect(),
    )
}

fn find_part(
    order: &[String],
    children: &HashMap<String, Node>,
    target_index: usize,
) -> Option<Vec<String>> {
    for name in order {
        match &children[name] {
            Node::Leaf { part_index } if *part_index == target_index => {
                return Some(vec![name.clone()]);
            }
            Node::Leaf { .. } => continue,
            Node::Folder {
                order: o,
                children: c,
            } => {
                if let Some(mut sub_path) = find_part(o, c, target_index) {
                    sub_path.insert(0, name.clone());
                    return Some(sub_path);
                }
            }
        }
    }
    None
}

/// A part's full disambiguated path (folder segments, then its own
/// possibly-`" (2)"`-suffixed name) within the current assembly tree --
/// the only correct way to turn a part index back into a breadcrumb,
/// since same-named siblings can disambiguate (see `insert`).
pub(super) fn locate_part(parts: &[Part], target_index: usize) -> Vec<String> {
    let (order, children) = build_nodes(parts);
    find_part(&order, &children, target_index)
        .expect("target_index must name a real part in `parts`")
}

/// Where to land the breadcrumb/selection to show `target_index`: the
/// assembly path containing it, plus that part's row position within
/// `rows_at(parts, materials, &that_path)`. The one chokepoint both
/// jump-to-flagged and fuzzy-jump-by-name call to land on a specific
/// part, so "find a part, then show it" is implemented exactly once.
pub(super) fn navigate_to_part(
    parts: &[Part],
    materials: &[Material],
    target_index: usize,
) -> (Vec<String>, usize) {
    let mut path = locate_part(parts, target_index);
    let leaf_name = path.pop().expect("locate_part never returns an empty path");
    let rows = rows_at(parts, materials, &path)
        .expect("a path popped from locate_part's own result is always valid");
    let row_index = rows
        .iter()
        .position(|r| r.name == leaf_name)
        .expect("leaf_name must appear among its own parent's rows");
    (path, row_index)
}

/// Every part index in depth-first tree order -- the order a user
/// actually encounters parts browsing the assembly list top to bottom,
/// drilling into each sub-assembly before moving to its next sibling.
/// This is *not* the same as `parts`' own Vec order: `stepcrawl::
/// group_parts` groups same-shaped parts together (for cutlist/BOM
/// purposes) regardless of which sub-assembly they live in, so two
/// instances of the same panel shape in different assemblies can sit
/// next to each other in `parts` while being nowhere near each other in
/// the tree. Jump-to-flagged (`next_flagged`/`prev_flagged` in `mod.rs`)
/// walks *this* order, not `parts`' raw index order, so it moves you
/// through what you're browsing rather than jumping to an unrelated
/// same-shaped part elsewhere in the model.
pub(super) fn depth_first_part_order(parts: &[Part]) -> Vec<usize> {
    let (order, children) = build_nodes(parts);
    let mut out = Vec::with_capacity(parts.len());
    collect_depth_first(&order, &children, &mut out);
    out
}

fn collect_depth_first(order: &[String], children: &HashMap<String, Node>, out: &mut Vec<usize>) {
    for name in order {
        match &children[name] {
            Node::Leaf { part_index } => out.push(*part_index),
            Node::Folder {
                order: sub_order,
                children: sub_children,
            } => collect_depth_first(sub_order, sub_children, out),
        }
    }
}

/// Column header for the leaf rows, aligned field-for-field with
/// `leaf_line` -- this is a plain `Paragraph` drawn above the row list
/// (see `ui::draw_assembly_list`), not part of the list widget itself.
/// Which of a part's two in-plane dimensions the grain runs along, in the
/// same terms the "Length"/"Width" columns already use -- see `Part::
/// grain_along_length`.
fn grain_label(part: &Part) -> &'static str {
    if part.grain_along_length {
        "Length"
    } else {
        "Width"
    }
}

pub(super) fn header_line() -> Line<'static> {
    let header_style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
    let pad = " ".repeat(ROW_PREFIX_WIDTH);
    let left = format!("{pad}{:<NAME_FIELD_WIDTH$}", "Part");
    let measurements = format!(" {:>9}  {:>9}  {:>8} ", "Length", "Width", "Thick");
    let grain = format!(" {:<6} ", "Grain");
    let material = " Material".to_string();
    Line::from(vec![
        Span::styled(left, header_style),
        divider(),
        Span::styled(measurements, header_style),
        divider(),
        Span::styled(grain, header_style),
        divider(),
        Span::styled(material, header_style),
    ])
}

pub(super) fn leaf_line(part: &Part, flagged: bool) -> Line<'static> {
    let name = part.path.rsplit(" / ").next().unwrap_or(&part.path);
    let marker = if flagged { "! " } else { "  " };
    let material = part
        .material
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("-");
    let left = format!("{marker}{name:<NAME_FIELD_WIDTH$}");
    let measurements = format!(
        " {:>9.4}  {:>9.4}  {:>8.4} ",
        part.length_in, part.width_in, part.thickness_in
    );
    let grain = format!(" {:<6} ", grain_label(part));
    let material = format!(" {material}");
    let style = if flagged {
        Style::new().fg(Color::Red)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::styled(left, style),
        divider(),
        Span::styled(measurements, style),
        divider(),
        Span::styled(grain, style),
        divider(),
        Span::styled(material, style),
    ])
}

/// Folders get their own bold accent color so the assembly list's shape
/// reads at a glance the same way a directory listing's does -- yellow
/// rather than LSCOLORS' traditional blue, which reads poorly against
/// this user's One Dark terminal theme. The `(N !)` flagged-descendant
/// count stays red regardless -- a warning needs to stay a warning
/// color, not blend into the folder's own. Padded by the same
/// `ROW_PREFIX_WIDTH` a leaf's flag marker takes, so folder and leaf
/// names in the same list start in the same screen column.
pub(super) fn folder_line(name: &str, flagged: usize) -> Line<'static> {
    let folder_style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let pad = " ".repeat(ROW_PREFIX_WIDTH);
    if flagged > 0 {
        Line::from(vec![
            Span::raw(pad),
            Span::styled(name.to_string(), folder_style),
            Span::styled(format!("  ({flagged} !)"), Style::new().fg(Color::Red)),
        ])
    } else {
        Line::from(vec![
            Span::raw(pad),
            Span::styled(name.to_string(), folder_style),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(name: &str, thickness_in: f64) -> Material {
        Material {
            name: name.to_string(),
            thickness_mm: thickness_in * 25.4,
        }
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
            is_exception: false,
            grain_along_length: true,
            dimensions: crate::assignments::DimensionAssignment::AS_GUESSED,
        }
    }

    fn row_names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn rows_at_root_lists_direct_children_only() {
        let parts = vec![
            part("Carcasses / Carcass A / Body"),
            part("Doors / Door A / Body"),
        ];
        let rows = rows_at(&parts, &[], &[]).expect("root always resolves");
        assert_eq!(row_names(&rows), vec!["Carcasses", "Doors"]);
        for row in &rows {
            assert!(
                matches!(row.kind, RowKind::Folder { flagged: 1 }),
                "each top-level assembly has exactly one unresolved part beneath it"
            );
        }
    }

    #[test]
    fn rows_at_nested_path_lists_that_assemblys_direct_children() {
        let parts = vec![
            part("Carcasses / Carcass A / Body"),
            part("Carcasses / Carcass A / Side"),
            part("Carcasses / Carcass B / Body"),
        ];
        let path = vec!["Carcasses".to_string()];
        let rows = rows_at(&parts, &[], &path).unwrap();
        assert_eq!(row_names(&rows), vec!["Carcass A", "Carcass B"]);

        let path = vec!["Carcasses".to_string(), "Carcass A".to_string()];
        let rows = rows_at(&parts, &[], &path).unwrap();
        assert_eq!(row_names(&rows), vec!["Body", "Side"]);
        assert!(rows
            .iter()
            .all(|r| matches!(r.kind, RowKind::Leaf { flagged: true, .. })));
    }

    #[test]
    fn rows_at_root_accepts_bare_leaves_alongside_folders() {
        let parts = vec![part("Standalone Jig"), part("Carcasses / Carcass A / Body")];
        let rows = rows_at(&parts, &[], &[]).unwrap();
        assert_eq!(row_names(&rows), vec!["Standalone Jig", "Carcasses"]);
        assert!(matches!(rows[0].kind, RowKind::Leaf { part_index: 0, .. }));
    }

    #[test]
    fn rows_at_returns_none_through_a_leaf() {
        let parts = vec![part("Body")];
        let path = vec!["Body".to_string(), "Anything".to_string()];
        assert!(rows_at(&parts, &[], &path).is_none());
    }

    #[test]
    fn rows_at_returns_none_for_an_unknown_path() {
        let parts = vec![part("Carcasses / Carcass A / Body")];
        let path = vec!["Doors".to_string()];
        assert!(rows_at(&parts, &[], &path).is_none());
    }

    #[test]
    fn rows_at_reflects_resolved_parts_in_flagged_counts() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut parts = vec![
            part("Carcasses / Carcass A / Flagged"),
            part("Carcasses / Carcass A / Resolved"),
        ];
        parts[1].material = Some(material("Baltic Birch 3/4", 0.75));

        let path = vec!["Carcasses".to_string()];
        let rows = rows_at(&parts, &materials, &path).unwrap();
        assert!(matches!(rows[0].kind, RowKind::Folder { flagged: 1 }));

        let rows = rows_at(&parts, &materials, &[]).unwrap();
        assert!(matches!(rows[0].kind, RowKind::Folder { flagged: 1 }));
    }

    #[test]
    fn locate_part_finds_a_top_level_leaf() {
        let parts = vec![part("Body")];
        assert_eq!(locate_part(&parts, 0), vec!["Body".to_string()]);
    }

    #[test]
    fn locate_part_finds_a_nested_leaf() {
        let parts = vec![part("Carcasses / Carcass A / Body")];
        assert_eq!(
            locate_part(&parts, 0),
            vec![
                "Carcasses".to_string(),
                "Carcass A".to_string(),
                "Body".to_string()
            ]
        );
    }

    #[test]
    fn locate_part_finds_disambiguated_colliding_siblings() {
        let parts = vec![
            part("Bench / Body"),
            part("Bench / Body"),
            part("Bench / Body"),
        ];
        assert_eq!(
            locate_part(&parts, 1),
            vec!["Bench".to_string(), "Body (2)".to_string()]
        );
        assert_eq!(
            locate_part(&parts, 2),
            vec!["Bench".to_string(), "Body (3)".to_string()]
        );
    }

    #[test]
    fn depth_first_part_order_follows_the_tree_not_the_parts_vec() {
        // Mirrors what `stepcrawl::group_parts` actually produces: same-
        // shaped parts across sibling assemblies land next to each other
        // in `Part`'s own Vec order (index 0 and 2 here are both
        // "Backer", from different carcasses), interleaved with an
        // unrelated part from the assembly in between (index 1). Tree
        // order must still walk Left's own parts before moving on to
        // Middle's, regardless of that interleaving.
        let parts = vec![
            part("Carcasses / Left / Backer"),   // 0
            part("Carcasses / Left / Panel"),    // 1
            part("Carcasses / Middle / Backer"), // 2
            part("Carcasses / Middle / Panel"),  // 3
        ];
        assert_eq!(depth_first_part_order(&parts), vec![0, 1, 2, 3]);
    }

    #[test]
    fn navigate_to_part_round_trips_to_a_nested_part() {
        let parts = vec![
            part("Carcasses / Carcass A / Body"),
            part("Carcasses / Carcass A / Side"),
            part("Doors / Door A / Body"),
        ];
        let (breadcrumb, row_index) = navigate_to_part(&parts, &[], 1);
        assert_eq!(
            breadcrumb,
            vec!["Carcasses".to_string(), "Carcass A".to_string()]
        );
        let rows = rows_at(&parts, &[], &breadcrumb).unwrap();
        assert!(matches!(
            rows[row_index].kind,
            RowKind::Leaf { part_index: 1, .. }
        ));
        assert_eq!(rows[row_index].name, "Side");
    }

    #[test]
    fn navigate_to_part_round_trips_to_a_root_level_part() {
        let parts = vec![part("Standalone Jig")];
        let (breadcrumb, row_index) = navigate_to_part(&parts, &[], 0);
        assert!(breadcrumb.is_empty());
        assert_eq!(row_index, 0);
    }

    #[test]
    fn count_flagged_sums_leaves_recursively_across_nested_folders() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut parts = vec![
            part("Bench / Carcasses / Carcass A / Flagged"),
            part("Bench / Carcasses / Carcass A / Resolved"),
            part("Bench / Doors / Door A / Flagged"),
        ];
        parts[1].material = Some(material("Baltic Birch 3/4", 0.75));

        let (order, children) = build_nodes(&parts);
        assert_eq!(
            count_flagged(&order, &children, &parts, &materials),
            2,
            "two of the three parts have no material assigned"
        );
    }

    #[test]
    fn count_flagged_is_zero_once_every_part_in_the_subtree_is_resolved() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut parts = vec![part("Bench / A"), part("Bench / B")];
        for p in &mut parts {
            p.material = Some(material("Baltic Birch 3/4", 0.75));
        }

        let (order, children) = build_nodes(&parts);
        assert_eq!(count_flagged(&order, &children, &parts, &materials), 0);
    }
}
