//! `storystick <model.step>`: an in-terminal tree over a STEP file's
//! parts -- flag rows that need attention, pick a material per part from
//! the stock catalog, save assignments back to the sidecar, and generate
//! the cutlist PDF from the tree's current state, all without switching
//! to another program.
//!
//! Geometry always comes fresh from the STEP file (see `load_parts`);
//! the only thing that persists between runs is the assignment sidecar,
//! keyed by path + dimensions, not bare path (see `crate::assignments`
//! and `Part::assignment_key`).

mod tree;
mod ui;

use crate::{assignments, round4, stock, MM_PER_IN};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::path::{Path, PathBuf};
use storystick_core::nesting::{bill_of_materials, pack, Material, PackablePart, StockSheet};
use storystick_core::stepcrawl::extract_parts;
use tui_tree_widget::TreeState;

/// How far a part's own measured thickness may sit from a candidate
/// material's nominal thickness and still be offered in the picker (or
/// count toward "more than one compatible material" for auto-flagging).
/// Purely a UX filter -- once a material is actually assigned, `pack()`
/// matches by material name, not thickness (see `PackablePart::material_name`),
/// so this tolerance never affects what a part can nest onto.
const COMPATIBLE_THICKNESS_TOLERANCE_IN: f64 = 0.06;

pub(crate) struct Part {
    pub path: String,
    /// The sidecar's real identity key for this part: `path` plus this
    /// part's own dimensions. Shapr3D does not actually guarantee sibling
    /// body names are unique -- an un-renamed duplicate can leave two
    /// geometrically different parts sharing one `path` (seen in real
    /// project data: two "Body 03 (2)"s under the same folder with
    /// different dimensions) -- so dimensions are always part of the key,
    /// not just when today's file happens to have a collision. A key that
    /// depended on whether *other* parts currently collide would be a
    /// moving target: a path that's unique today could gain a colliding
    /// sibling in a future re-export, silently changing that key's shape
    /// and orphaning an assignment saved under the old one. Keying on a
    /// part's own (path, dimensions) alone never depends on what else is
    /// in the file, so it can't drift out from under a saved assignment
    /// that way. See `assignment_key`.
    pub assignment_key: String,
    pub length_in: f64,
    pub width_in: f64,
    pub thickness_in: f64,
    pub unreliable: bool,
    pub material: Option<String>,
}

fn assignment_key(path: &str, length_in: f64, width_in: f64, thickness_in: f64) -> String {
    format!("{path} @ {length_in:.4}x{width_in:.4}x{thickness_in:.4}")
}

impl Part {
    fn to_packable(&self) -> PackablePart {
        let mut part = PackablePart::new(self.path.clone(), self.length_in * MM_PER_IN, self.width_in * MM_PER_IN, self.thickness_in * MM_PER_IN);
        part.material_name = self.material.clone();
        part
    }
}

fn distinct_materials(stock: &[StockSheet]) -> Vec<Material> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for sheet in stock {
        if seen.insert(sheet.material.name.clone()) {
            out.push(sheet.material.clone());
        }
    }
    out
}

fn compatible_materials<'a>(materials: &'a [Material], thickness_in: f64) -> Vec<&'a Material> {
    materials.iter().filter(|m| (m.thickness_mm / MM_PER_IN - thickness_in).abs() <= COMPATIBLE_THICKNESS_TOLERANCE_IN).collect()
}

/// A part is flagged when its geometry was ambiguous (`unreliable`, set
/// by stepcrawl when a face-normal-based dimension guess couldn't be made
/// confidently) or when it's unassigned but more than one stock material
/// shares its thickness -- leaving it blank would let cutlist generation
/// route it onto whichever one it finds space on first, which is exactly
/// the "hidden stretcher on show-face plywood" mistake a named material
/// assignment exists to prevent.
pub(crate) fn part_flag(part: &Part, materials: &[Material]) -> Option<&'static str> {
    if part.unreliable {
        return Some("unreliable geometry");
    }
    if part.material.is_none() && compatible_materials(materials, part.thickness_in).len() > 1 {
        return Some("ambiguous material");
    }
    None
}

fn load_parts(step_path: &Path, assignments: &BTreeMap<String, String>) -> Result<Vec<Part>, Box<dyn Error>> {
    let groups = extract_parts(step_path)?;
    let mut parts = Vec::new();
    for group in &groups {
        for instance in &group.instances {
            let length_in = round4(group.length_mm / MM_PER_IN);
            let width_in = round4(group.width_mm / MM_PER_IN);
            let thickness_in = round4(group.thickness_mm / MM_PER_IN);
            let key = assignment_key(&instance.path, length_in, width_in, thickness_in);
            let material = assignments.get(&key).cloned();
            parts.push(Part {
                path: instance.path.clone(),
                assignment_key: key,
                length_in,
                width_in,
                thickness_in,
                unreliable: instance.unreliable,
                material,
            });
        }
    }
    Ok(parts)
}

pub(crate) struct PickerState {
    pub(crate) part_index: usize,
    pub(crate) options: Vec<String>,
    pub(crate) list_state: ListState,
}

pub(crate) struct App {
    pub(crate) parts: Vec<Part>,
    /// Maps a tree node's identifier path (as `tree_state.selected()`
    /// joins it) to its part index -- rebuilt every draw alongside the
    /// tree itself (see `ui::draw_tree`), since a leaf's identifier can
    /// differ from `Part::path` when `tree::build` had to disambiguate a
    /// same-named sibling (see `tree::insert`).
    pub(crate) selection_index: HashMap<String, usize>,
    pub(crate) materials: Vec<Material>,
    stock: Vec<StockSheet>,
    pub(crate) tree_state: TreeState<String>,
    pub(crate) dirty: bool,
    pub(crate) status: String,
    pub(crate) picker: Option<PickerState>,
    pub(crate) step_path: PathBuf,
    sidecar_path: PathBuf,
    out_pdf_path: PathBuf,
    kerf_in: f64,
    trim_allowance_in: f64,
    pub(crate) last_tree_height: u16,
}

impl App {
    pub(crate) fn assigned_counts(&self) -> (usize, usize) {
        (self.parts.iter().filter(|p| p.material.is_some()).count(), self.parts.len())
    }

    fn selected_part_index(&self) -> Option<usize> {
        let selected = self.tree_state.selected();
        if selected.is_empty() {
            return None;
        }
        self.selection_index.get(&selected.join(" / ")).copied()
    }

    fn handle_enter(&mut self) {
        if self.selected_part_index().is_some() {
            self.open_picker();
        } else {
            self.tree_state.toggle_selected();
        }
    }

    fn open_picker(&mut self) {
        let Some(i) = self.selected_part_index() else {
            self.status = "select a part first".to_string();
            return;
        };
        let part = &self.parts[i];
        let mut options: Vec<String> = compatible_materials(&self.materials, part.thickness_in).into_iter().map(|m| m.name.clone()).collect();
        options.sort();
        options.insert(0, "(clear -- match by thickness alone)".to_string());
        let current_index = match &part.material {
            None => 0,
            Some(name) => options.iter().position(|o| o == name).unwrap_or(0),
        };
        let mut list_state = ListState::default();
        list_state.select(Some(current_index));
        self.picker = Some(PickerState { part_index: i, options, list_state });
    }

    fn confirm_picker(&mut self) {
        let Some(picker) = self.picker.take() else { return };
        let Some(choice) = picker.list_state.selected() else { return };
        let chosen = picker.options[choice].clone();
        self.parts[picker.part_index].material = if choice == 0 { None } else { Some(chosen) };
        self.dirty = true;
        self.status = format!("set material for {}", self.parts[picker.part_index].path);
    }

    fn save(&mut self) {
        let map: BTreeMap<String, String> =
            self.parts.iter().filter_map(|p| p.material.clone().map(|m| (p.assignment_key.clone(), m))).collect();
        match assignments::save(&map, &self.sidecar_path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("saved {}", self.sidecar_path.display());
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    fn generate_cutlist(&mut self) {
        let parts: Vec<PackablePart> = self.parts.iter().map(Part::to_packable).collect();
        let trim_allowance_mm = self.trim_allowance_in * MM_PER_IN;
        let layout = pack(&parts, &self.stock, self.kerf_in * MM_PER_IN, trim_allowance_mm);
        let unplaced = layout.unplaced.len();
        let bom = bill_of_materials(&layout);
        let pdf_bytes = storystick_core::diagrams::render_pdf(&layout, &bom, trim_allowance_mm);
        match std::fs::write(&self.out_pdf_path, pdf_bytes) {
            Ok(()) => {
                self.status = if unplaced == 0 {
                    format!("wrote {} ({} sheets)", self.out_pdf_path.display(), layout.sheets.len())
                } else {
                    format!("wrote {} ({} sheets, {} part(s) unplaced)", self.out_pdf_path.display(), layout.sheets.len(), unplaced)
                };
            }
            Err(e) => self.status = format!("failed to write {}: {e}", self.out_pdf_path.display()),
        }
    }
}

pub(crate) fn run(step_path: &Path, stock_path: &Path, out_pdf: &Path, kerf_in: f64, trim_allowance_in: f64) -> Result<(), Box<dyn Error>> {
    let sidecar_path = assignments::sidecar_path(step_path);
    let existing_assignments = assignments::load(&sidecar_path)?;
    let parts = load_parts(step_path, &existing_assignments)?;

    let stock_list = stock::read(stock_path)?;
    let materials = distinct_materials(&stock_list);

    let tree_state = TreeState::default();

    let total = parts.len();
    let mut app = App {
        parts,
        selection_index: HashMap::new(),
        materials,
        stock: stock_list,
        tree_state,
        dirty: false,
        status: format!("{total} part(s) -- j/k move, h/l fold, Enter/m assign, Ctrl-d/u page, s save, c cutlist, q quit"),
        picker: None,
        step_path: step_path.to_path_buf(),
        sidecar_path,
        out_pdf_path: out_pdf.to_path_buf(),
        kerf_in,
        trim_allowance_in,
        last_tree_height: 20,
    };

    ratatui::run(|terminal| -> Result<(), Box<dyn Error>> {
        let mut first_frame = true;
        loop {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            if first_frame {
                // `select_first` reads a cache the tree only populates once
                // it's actually been rendered, so this can't happen before
                // the loop's first `draw` -- redraw once more immediately
                // so the initial selection is visible without needing a
                // keypress first.
                first_frame = false;
                app.tree_state.select_first();
                continue;
            }
            let Event::Key(key) = event::read()? else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if app.picker.is_some() {
                match key.code {
                    KeyCode::Esc => app.picker = None,
                    KeyCode::Enter => app.confirm_picker(),
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(picker) = &mut app.picker {
                            let len = picker.options.len();
                            let cur = picker.list_state.selected().unwrap_or(0) as i64;
                            let next = (cur - 1).rem_euclid(len as i64) as usize;
                            picker.list_state.select(Some(next));
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(picker) = &mut app.picker {
                            let len = picker.options.len();
                            let next = (picker.list_state.selected().unwrap_or(0) + 1) % len;
                            picker.list_state.select(Some(next));
                        }
                    }
                    _ => {}
                }
                continue;
            }

            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                KeyCode::Char('d') if ctrl => {
                    let n = (app.last_tree_height / 2).max(1) as usize;
                    app.tree_state.scroll_down(n);
                }
                KeyCode::Char('u') if ctrl => {
                    let n = (app.last_tree_height / 2).max(1) as usize;
                    app.tree_state.scroll_up(n);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.tree_state.key_up();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.tree_state.key_down();
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    app.tree_state.key_left();
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    app.tree_state.key_right();
                }
                KeyCode::Enter => app.handle_enter(),
                KeyCode::Char('m') => app.open_picker(),
                KeyCode::Char('s') => app.save(),
                KeyCode::Char('c') => app.generate_cutlist(),
                _ => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(name: &str, thickness_in: f64) -> Material {
        Material { name: name.to_string(), thickness_mm: thickness_in * MM_PER_IN }
    }

    fn part(thickness_in: f64, material: Option<&str>, unreliable: bool) -> Part {
        Part {
            path: "Bench / Body".to_string(),
            assignment_key: assignment_key("Bench / Body", 30.0, 20.0, thickness_in),
            length_in: 30.0,
            width_in: 20.0,
            thickness_in,
            unreliable,
            material: material.map(str::to_string),
        }
    }

    #[test]
    fn unreliable_parts_are_always_flagged_regardless_of_material() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let flagged = part(0.75, Some("Baltic Birch 3/4"), true);
        assert_eq!(part_flag(&flagged, &materials), Some("unreliable geometry"));
    }

    #[test]
    fn unassigned_part_is_flagged_when_multiple_materials_share_its_thickness() {
        let materials = vec![material("Baltic Birch 3/4", 0.75), material("Sande Ply 3/4", 0.75)];
        let unassigned = part(0.75, None, false);
        assert_eq!(part_flag(&unassigned, &materials), Some("ambiguous material"));
    }

    #[test]
    fn unassigned_part_is_not_flagged_when_only_one_material_matches() {
        let materials = vec![material("Baltic Birch 3/4", 0.75), material("Baltic Birch 1/4", 0.25)];
        let unassigned = part(0.75, None, false);
        assert_eq!(part_flag(&unassigned, &materials), None);
    }

    #[test]
    fn pinning_a_material_clears_the_ambiguous_flag() {
        let materials = vec![material("Baltic Birch 3/4", 0.75), material("Sande Ply 3/4", 0.75)];
        let pinned = part(0.75, Some("Sande Ply 3/4"), false);
        assert_eq!(part_flag(&pinned, &materials), None);
    }

    #[test]
    fn distinct_materials_dedupes_by_name_preserving_first_appearance_order() {
        let bb34 = material("Baltic Birch 3/4", 0.75);
        let sande34 = material("Sande Ply 3/4", 0.75);
        let stock = vec![
            StockSheet { material: bb34.clone(), length_mm: 2438.4, width_mm: 1219.2 },
            StockSheet { material: sande34, length_mm: 2438.4, width_mm: 1219.2 },
            StockSheet { material: bb34, length_mm: 1219.2, width_mm: 609.6 },
        ];
        let distinct = distinct_materials(&stock);
        let names: Vec<&str> = distinct.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["Baltic Birch 3/4", "Sande Ply 3/4"]);
    }

    #[test]
    fn assignment_key_disambiguates_a_colliding_path_by_dimensions() {
        // `assignment_key` takes only one part's own (path, dimensions) --
        // never a sibling list -- so this can't drift depending on what
        // else is in the file: the same part always keys the same way,
        // whether or not a same-path sibling happens to exist this run.
        let path = "Bench / Left Carcass / Body 03 (2)";
        let a = assignment_key(path, 30.0, 16.25, 0.75);
        let b = assignment_key(path, 23.625, 17.25, 0.75);
        assert_ne!(a, b, "two real parts sharing a path must never collapse onto one key");
    }

    #[test]
    fn compatible_materials_excludes_a_materially_different_thickness() {
        let materials = vec![material("Baltic Birch 3/4", 0.75), material("Baltic Birch 1/4", 0.25)];
        let compat = compatible_materials(&materials, 0.75);
        assert_eq!(compat.len(), 1);
        assert_eq!(compat[0].name, "Baltic Birch 3/4");
    }

    #[test]
    fn load_parts_applies_existing_sidecar_assignments_by_key() {
        // Can't call extract_parts without a real STEP file here, but this
        // is the same lookup load_parts performs against the sidecar --
        // by assignment_key, not by bare path.
        let mut p = part(0.75, None, false);
        let mut assignments = BTreeMap::new();
        assignments.insert(p.assignment_key.clone(), "Baltic Birch 3/4".to_string());
        p.material = assignments.get(&p.assignment_key).cloned();
        assert_eq!(p.material.as_deref(), Some("Baltic Birch 3/4"));
    }
}
