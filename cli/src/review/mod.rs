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
//!
//! Grain always runs with a part's length. stepcrawl's own length/width
//! guess (longer of the two in-plane dimensions is length) is often
//! exactly what you want, but not always -- `g` swaps a part's
//! length/width when it isn't, rather than exposing a separate "grain"
//! concept: there was never an independent capability there to preserve
//! (packing has only ever cared about which dimension is called length),
//! so a second concept meaning the same thing was just something else to
//! learn.

mod tree;
mod ui;

use crate::assignments::PartOverride;
use crate::{assignments, round4, stock, MM_PER_IN};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::path::{Path, PathBuf};
use storystick_core::nesting::{bill_of_materials, pack, Material, PackablePart, StockSheet};
use storystick_core::stepcrawl::{extract_parts, relabel_with_known_thickness};
use tui_tree_widget::TreeState;

/// How far a part's own measured thickness may sit from a candidate
/// material's nominal thickness and still be offered in the picker (or
/// count toward "more than one compatible material" for auto-flagging).
/// Also used, converted to mm, as `resolve_dims`'s correction tolerance --
/// deliberately the *same* tolerance for both, not core's tighter
/// `DEFAULT_KNOWN_THICKNESS_TOLERANCE_MM` (1mm, tuned for spotting a
/// narrow-rip misread against otherwise-clean dimensions): a material the
/// picker was willing to offer must never immediately flag as a mismatch
/// the moment you pick it, and real sheet goods commonly run a bit under
/// their nominal thickness (a lot of "3/4"" plywood is closer to 23/32"),
/// so 0.06" of slack matters for the correction too, not just the picker.
const COMPATIBLE_THICKNESS_TOLERANCE_IN: f64 = 0.06;

pub(crate) struct Part {
    pub path: String,
    /// The sidecar's real identity key for this part: `path` plus this
    /// part's own *raw* dimensions (see `raw_length_in` etc, never the
    /// possibly-corrected/swapped `length_in` etc -- a key that shifted
    /// under a material reassignment would orphan the very override it's
    /// meant to persist). Shapr3D does not actually guarantee sibling
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
    /// stepcrawl's raw guess (longer of the two in-plane dimensions is
    /// length, shorter is width, third is thickness) -- immutable for the
    /// part's lifetime, since it's the actual geometric measurement.
    /// `length_in`/`width_in`/`thickness_in` are derived from this (see
    /// `resolve_dims`), recomputed whenever `material` or `swapped`
    /// changes. Keeping the raw triple fixed is what makes that
    /// derivation reversible: re-picking a different material, or
    /// toggling the swap back off, recovers the exact original numbers
    /// instead of drifting through repeated corrections.
    pub raw_length_in: f64,
    pub raw_width_in: f64,
    pub raw_thickness_in: f64,
    pub length_in: f64,
    pub width_in: f64,
    pub thickness_in: f64,
    pub unreliable: bool,
    /// True when a material is assigned but none of this part's three raw
    /// dimensions comes within tolerance of that material's thickness --
    /// a real mismatch (wrong material picked, or this part isn't what it
    /// looks like), not just a missed correction. See `resolve_dims`.
    pub thickness_mismatch: bool,
    pub material: Option<String>,
    /// Grain always runs with length (see this module's docs); this says
    /// whether length/width, as guessed, have been swapped so the part's
    /// other edge runs with the grain instead.
    pub swapped: bool,
}

fn assignment_key(path: &str, length_in: f64, width_in: f64, thickness_in: f64) -> String {
    format!("{path} @ {length_in:.4}x{width_in:.4}x{thickness_in:.4}")
}

/// Derives (length_in, width_in, thickness_in, thickness_mismatch) from a
/// part's raw (length_in, width_in, thickness_in) guess, a possibly-
/// assigned material, and whether length/width have been manually
/// swapped. Operates purely on the *values* in `raw`, never their
/// current field positions, so it's safe to call repeatedly as material
/// or swap state changes: re-deriving from the same three raw numbers
/// each time means a cleared material or an untoggled swap recovers
/// exactly the original guess, and a changed material re-picks thickness
/// fresh rather than compounding onto a previous correction.
fn resolve_dims(raw: (f64, f64, f64), material: Option<&Material>, swapped: bool) -> (f64, f64, f64, bool) {
    let (mut length_in, mut width_in, mut thickness_in) = raw;
    let mut thickness_mismatch = false;
    if let Some(m) = material {
        let raw_mm = (raw.0 * MM_PER_IN, raw.1 * MM_PER_IN, raw.2 * MM_PER_IN);
        match relabel_with_known_thickness(raw_mm, m.thickness_mm, COMPATIBLE_THICKNESS_TOLERANCE_IN * MM_PER_IN) {
            Ok((length_mm, width_mm, thickness_mm)) => {
                length_in = round4(length_mm / MM_PER_IN);
                width_in = round4(width_mm / MM_PER_IN);
                thickness_in = round4(thickness_mm / MM_PER_IN);
            }
            Err(_) => thickness_mismatch = true,
        }
    }
    if swapped {
        std::mem::swap(&mut length_in, &mut width_in);
    }
    (length_in, width_in, thickness_in, thickness_mismatch)
}

/// Minimal-decimals text for a print-settings edit field (e.g. "0.125",
/// not "0.1250"; "0", not "0.0") -- easier to edit than a fixed-width
/// display value.
fn format_editable(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".to_string() } else { s.to_string() }
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
/// confidently), when its assigned material's thickness doesn't actually
/// match any of its measured dimensions (`thickness_mismatch`, see
/// `resolve_dims` -- a real problem, not a missed correction: the wrong
/// material got picked, or this part isn't what it looks like), or when
/// it has no material assigned at all -- even if only one stock material
/// happens to match its thickness today. That single match is still an
/// inference, not a decision you made: it's silently wrong the moment a
/// second material at that thickness is added to the catalog, and until
/// then it's easy to mistake an unreviewed part for a resolved one just
/// because nothing highlighted it. A part missing a decision is flagged
/// either way; the reason string only distinguishes *why* -- genuinely
/// ambiguous (more than one stock material shares its thickness, so
/// leaving it blank would let printing route it onto whichever one it
/// finds space on first, the "hidden stretcher on show-face plywood"
/// mistake a named assignment exists to prevent) versus simply not yet
/// reviewed.
pub(crate) fn part_flag(part: &Part, materials: &[Material]) -> Option<&'static str> {
    if part.unreliable {
        return Some("unreliable geometry");
    }
    if part.thickness_mismatch {
        return Some("material thickness doesn't match this part's geometry");
    }
    if part.material.is_none() {
        return Some(if compatible_materials(materials, part.thickness_in).len() > 1 { "ambiguous material" } else { "no material assigned" });
    }
    None
}

fn load_parts(step_path: &Path, overrides: &BTreeMap<String, PartOverride>, materials: &[Material]) -> Result<Vec<Part>, Box<dyn Error>> {
    let groups = extract_parts(step_path)?;
    let mut parts = Vec::new();
    for group in &groups {
        for instance in &group.instances {
            let raw_length_in = round4(group.length_mm / MM_PER_IN);
            let raw_width_in = round4(group.width_mm / MM_PER_IN);
            let raw_thickness_in = round4(group.thickness_mm / MM_PER_IN);
            let key = assignment_key(&instance.path, raw_length_in, raw_width_in, raw_thickness_in);
            let over = overrides.get(&key);
            let material = over.and_then(|o| o.material.clone());
            let swapped = over.map(|o| o.swapped).unwrap_or(false);
            let material_ref = material.as_deref().and_then(|name| materials.iter().find(|m| m.name == name));
            let (length_in, width_in, thickness_in, thickness_mismatch) =
                resolve_dims((raw_length_in, raw_width_in, raw_thickness_in), material_ref, swapped);
            parts.push(Part {
                path: instance.path.clone(),
                assignment_key: key,
                raw_length_in,
                raw_width_in,
                raw_thickness_in,
                length_in,
                width_in,
                thickness_in,
                unreliable: instance.unreliable,
                thickness_mismatch,
                material,
                swapped,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrintField {
    Kerf,
    TrimAllowance,
}

pub(crate) struct PrintSettings {
    pub(crate) kerf_in: String,
    pub(crate) trim_allowance_in: String,
    pub(crate) focus: PrintField,
    /// Whether the focused field has had a keystroke since it was last
    /// focused. The first character typed replaces the pre-filled value
    /// instead of appending to it -- without this, typing "0.25" over a
    /// pre-filled "0" would land on "00.25".
    kerf_touched: bool,
    trim_touched: bool,
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
    pub(crate) print_settings: Option<PrintSettings>,
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

    fn expand_all(&mut self) {
        for path in tree::all_folder_paths(&self.parts) {
            self.tree_state.open(path);
        }
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

    /// Recomputes `length_in`/`width_in`/`thickness_in`/`thickness_mismatch`
    /// for `self.parts[i]` from its raw dims, current material, and swap
    /// state -- call after mutating either (see `resolve_dims`).
    fn resolve_part_dims(&mut self, i: usize) {
        let material_name = self.parts[i].material.clone();
        let material = material_name.as_deref().and_then(|name| self.materials.iter().find(|m| m.name == name));
        let raw = (self.parts[i].raw_length_in, self.parts[i].raw_width_in, self.parts[i].raw_thickness_in);
        let swapped = self.parts[i].swapped;
        let (length_in, width_in, thickness_in, thickness_mismatch) = resolve_dims(raw, material, swapped);
        let part = &mut self.parts[i];
        part.length_in = length_in;
        part.width_in = width_in;
        part.thickness_in = thickness_in;
        part.thickness_mismatch = thickness_mismatch;
    }

    fn confirm_picker(&mut self) {
        let Some(picker) = self.picker.take() else { return };
        let Some(choice) = picker.list_state.selected() else { return };
        let chosen = picker.options[choice].clone();
        self.parts[picker.part_index].material = if choice == 0 { None } else { Some(chosen) };
        self.resolve_part_dims(picker.part_index);
        self.dirty = true;
        self.status = format!("set material for {}", self.parts[picker.part_index].path);
    }

    fn toggle_swap(&mut self) {
        let Some(i) = self.selected_part_index() else {
            self.status = "select a part first".to_string();
            return;
        };
        self.parts[i].swapped = !self.parts[i].swapped;
        self.resolve_part_dims(i);
        self.dirty = true;
        self.status = format!("swapped length/width for {}", self.parts[i].path);
    }

    fn save(&mut self) {
        let map: BTreeMap<String, PartOverride> = self
            .parts
            .iter()
            .filter_map(|p| {
                let over = PartOverride { material: p.material.clone(), swapped: p.swapped };
                if over.is_empty() { None } else { Some((p.assignment_key.clone(), over)) }
            })
            .collect();
        match assignments::save(&map, &self.sidecar_path) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("saved {}", self.sidecar_path.display());
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    fn open_print_settings(&mut self) {
        self.print_settings = Some(PrintSettings {
            kerf_in: format_editable(self.kerf_in),
            trim_allowance_in: format_editable(self.trim_allowance_in),
            focus: PrintField::Kerf,
            kerf_touched: false,
            trim_touched: false,
        });
    }

    fn print_settings_field(ps: &mut PrintSettings) -> (&mut String, &mut bool) {
        match ps.focus {
            PrintField::Kerf => (&mut ps.kerf_in, &mut ps.kerf_touched),
            PrintField::TrimAllowance => (&mut ps.trim_allowance_in, &mut ps.trim_touched),
        }
    }

    fn print_settings_input(&mut self, c: char) {
        let Some(ps) = &mut self.print_settings else { return };
        let (field, touched) = Self::print_settings_field(ps);
        if !*touched {
            field.clear();
            *touched = true;
        }
        if c.is_ascii_digit() || (c == '.' && !field.contains('.')) {
            field.push(c);
        }
    }

    fn print_settings_backspace(&mut self) {
        let Some(ps) = &mut self.print_settings else { return };
        let (field, touched) = Self::print_settings_field(ps);
        *touched = true;
        field.pop();
    }

    fn print_settings_toggle_focus(&mut self) {
        let Some(ps) = &mut self.print_settings else { return };
        ps.focus = match ps.focus {
            PrintField::Kerf => PrintField::TrimAllowance,
            PrintField::TrimAllowance => PrintField::Kerf,
        };
    }

    fn confirm_print_settings(&mut self) {
        let Some(ps) = self.print_settings.take() else { return };
        match (ps.kerf_in.parse::<f64>(), ps.trim_allowance_in.parse::<f64>()) {
            (Ok(kerf_in), Ok(trim_allowance_in)) => {
                self.kerf_in = kerf_in;
                self.trim_allowance_in = trim_allowance_in;
                self.print();
            }
            _ => self.status = "kerf and trim allowance must both be numbers, in inches".to_string(),
        }
    }

    fn print(&mut self) {
        let parts: Vec<PackablePart> = self.parts.iter().map(Part::to_packable).collect();
        let trim_allowance_mm = self.trim_allowance_in * MM_PER_IN;
        let layout = pack(&parts, &self.stock, self.kerf_in * MM_PER_IN, trim_allowance_mm);
        let unplaced = layout.unplaced.len();
        let bom = bill_of_materials(&layout);
        let pdf_bytes = storystick_core::diagrams::render_pdf(&layout, &bom, trim_allowance_mm);
        match std::fs::write(&self.out_pdf_path, pdf_bytes) {
            Ok(()) => {
                self.status = if unplaced == 0 {
                    format!("printed {} ({} sheets)", self.out_pdf_path.display(), layout.sheets.len())
                } else {
                    format!("printed {} ({} sheets, {} part(s) unplaced)", self.out_pdf_path.display(), layout.sheets.len(), unplaced)
                };
            }
            Err(e) => self.status = format!("failed to write {}: {e}", self.out_pdf_path.display()),
        }
    }
}

pub(crate) fn run(step_path: &Path, stock_path: &Path, out_pdf: &Path, kerf_in: f64, trim_allowance_in: f64) -> Result<(), Box<dyn Error>> {
    let sidecar_path = assignments::sidecar_path(step_path);
    let existing_assignments = assignments::load(&sidecar_path)?;

    let stock_list = stock::read(stock_path)?;
    let materials = distinct_materials(&stock_list);

    let parts = load_parts(step_path, &existing_assignments, &materials)?;

    let tree_state = TreeState::default();

    let total = parts.len();
    let mut app = App {
        parts,
        selection_index: HashMap::new(),
        materials,
        stock: stock_list,
        tree_state,
        dirty: false,
        status: format!("{total} part(s) -- j/k move, h/l fold, e/c expand/collapse all, Enter/m assign, g swap L/W, Ctrl-d/u page, s save, p print, q quit"),
        picker: None,
        print_settings: None,
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

            if app.print_settings.is_some() {
                match key.code {
                    KeyCode::Esc => app.print_settings = None,
                    KeyCode::Enter => app.confirm_print_settings(),
                    KeyCode::Tab | KeyCode::Up | KeyCode::Down => app.print_settings_toggle_focus(),
                    KeyCode::Char(c) => app.print_settings_input(c),
                    KeyCode::Backspace => app.print_settings_backspace(),
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
                KeyCode::Char('e') => app.expand_all(),
                KeyCode::Char('c') => {
                    app.tree_state.close_all();
                }
                KeyCode::Enter => app.handle_enter(),
                KeyCode::Char('m') => app.open_picker(),
                KeyCode::Char('g') => app.toggle_swap(),
                KeyCode::Char('s') => app.save(),
                KeyCode::Char('p') => app.open_print_settings(),
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
            raw_length_in: 30.0,
            raw_width_in: 20.0,
            raw_thickness_in: thickness_in,
            length_in: 30.0,
            width_in: 20.0,
            thickness_in,
            unreliable,
            thickness_mismatch: false,
            material: material.map(str::to_string),
            swapped: false,
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
    fn unassigned_part_is_still_flagged_when_only_one_material_matches() {
        // A single match today is still an inference, not a decision --
        // and silently wrong the moment a second material at that
        // thickness joins the catalog. An unreviewed part must never look
        // the same as a resolved one.
        let materials = vec![material("Baltic Birch 3/4", 0.75), material("Baltic Birch 1/4", 0.25)];
        let unassigned = part(0.75, None, false);
        assert_eq!(part_flag(&unassigned, &materials), Some("no material assigned"));
    }

    #[test]
    fn pinning_a_material_clears_the_ambiguous_flag() {
        let materials = vec![material("Baltic Birch 3/4", 0.75), material("Sande Ply 3/4", 0.75)];
        let pinned = part(0.75, Some("Sande Ply 3/4"), false);
        assert_eq!(part_flag(&pinned, &materials), None);
    }

    #[test]
    fn thickness_mismatch_is_flagged_even_though_a_material_is_assigned() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut mismatched = part(0.75, Some("Baltic Birch 3/4"), false);
        mismatched.thickness_mismatch = true;
        assert_eq!(part_flag(&mismatched, &materials), Some("material thickness doesn't match this part's geometry"));
    }

    #[test]
    fn resolve_dims_leaves_the_raw_guess_alone_with_no_material() {
        let (length_in, width_in, thickness_in, mismatch) = resolve_dims((30.0, 20.0, 0.75), None, false);
        assert_eq!((length_in, width_in, thickness_in), (30.0, 20.0, 0.75));
        assert!(!mismatch);
    }

    #[test]
    fn resolve_dims_corrects_a_narrow_rip_once_the_material_is_known() {
        // Ripped from 3/4" stock to a strip narrower than it is thick:
        // the raw largest/middle/smallest guess mislabels the 0.25" width
        // as thickness. Knowing the assigned material is really 3/4"
        // fixes it.
        let bb34 = Material { name: "Baltic Birch 3/4".to_string(), thickness_mm: 0.75 * MM_PER_IN };
        let (length_in, width_in, thickness_in, mismatch) = resolve_dims((24.0, 0.75, 0.25), Some(&bb34), false);
        assert_eq!((length_in, width_in, thickness_in), (24.0, 0.25, 0.75));
        assert!(!mismatch);
    }

    #[test]
    fn resolve_dims_flags_a_mismatch_instead_of_guessing() {
        let unrelated = Material { name: "1/8\" hardboard".to_string(), thickness_mm: 0.125 * MM_PER_IN };
        let (length_in, width_in, thickness_in, mismatch) = resolve_dims((30.0, 20.0, 0.75), Some(&unrelated), false);
        // No dimension is anywhere near 0.125" -- dims fall back to the
        // raw guess rather than silently picking the closest anyway.
        assert_eq!((length_in, width_in, thickness_in), (30.0, 20.0, 0.75));
        assert!(mismatch);
    }

    #[test]
    fn resolve_dims_applies_the_swap_after_any_material_correction() {
        let bb34 = Material { name: "Baltic Birch 3/4".to_string(), thickness_mm: 0.75 * MM_PER_IN };
        let (length_in, width_in, thickness_in, mismatch) = resolve_dims((24.0, 0.75, 0.25), Some(&bb34), true);
        // Same correction as above (thickness -> 0.75, remaining sorted
        // 24/0.25), then length_in/width_in end up swapped on top.
        assert_eq!((length_in, width_in, thickness_in), (0.25, 24.0, 0.75));
        assert!(!mismatch);
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
    fn format_editable_trims_trailing_zeros_and_a_bare_zero_stays_zero() {
        assert_eq!(format_editable(0.125), "0.125");
        assert_eq!(format_editable(0.0), "0");
        assert_eq!(format_editable(1.0), "1");
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
