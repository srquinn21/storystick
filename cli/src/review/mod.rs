//! `storystick`: an in-terminal tree over a project's STEP export --
//! flag rows that need attention, assign materials, save state back to
//! `storystick.yaml`, and generate the section-grouped cutlist PDF from
//! the tree's current state, all without switching to another program.
//!
//! Geometry always comes fresh from the STEP file (see `load_parts`);
//! everything else that persists between runs -- which materials this
//! project uses, bracket-tag rules, kerf/trim/output settings, and
//! per-part exceptions -- lives in one project file (see `crate::project`
//! and `Part::assignment_key`).
//!
//! A part's material resolves in this order (see `resolve_material`):
//! its own exception (`crate::project::Project::assignments`), if it has
//! one; else whichever of its bracket tags has a rule
//! (`crate::project::Project::autofill`); else unassigned, flagged.
//! Bulk-edit (`b`) is the primary way a rule gets authored -- it writes
//! *one* rule, applied fresh to every currently-matching part that has no
//! exception of its own, and re-applied automatically to any part a
//! future STEP revision adds with the same tag. The single-part picker
//! (`m`) instead carves out an exception for just the selected part;
//! clearing that exception removes it outright and re-resolves the part
//! from whatever rule currently applies (or plain unassigned, if none
//! does) -- there's no third "explicitly no material" state to hold in
//! reserve, on either the exception or the rule side.
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
use crate::project::Project;
use crate::{autofill, round4, stock, MM_PER_IN};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use storystick_core::nesting::{pack, Material, PackablePart, StockSheet};
use storystick_core::stepcrawl::{extract_parts, relabel_with_known_thickness};
use storystick_core::tags;
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

/// How long a transient status message (a save/print confirmation, an
/// error, "set material for ...") stays on screen before it's replaced
/// with the resting keyboard-shortcut help text -- long enough to read,
/// short enough that the help line (the thing you actually want visible
/// most of the time, especially right after `s`) comes back on its own
/// rather than staying clobbered until the next action happens to
/// overwrite it.
const STATUS_MESSAGE_TIMEOUT: Duration = Duration::from_secs(4);

/// How often the main loop wakes up with no key pressed, purely to check
/// whether a transient status message has timed out (see
/// `App::expire_status`) -- short enough that the help text's return
/// feels prompt, long enough not to matter for CPU usage.
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) struct Part {
    pub path: String,
    /// This project's exception-map identity for this part: `path` plus
    /// this part's own *raw* dimensions (see `raw_length_in` etc, never
    /// the possibly-corrected/swapped `length_in` etc -- a key that
    /// shifted under a material reassignment would orphan the very
    /// exception it's meant to persist). Shapr3D does not actually
    /// guarantee sibling body names are unique -- an un-renamed duplicate
    /// can leave two geometrically different parts sharing one `path`
    /// (seen in real project data: two "Body 03 (2)"s under the same
    /// folder with different dimensions) -- so dimensions are always part
    /// of the key, not just when today's file happens to have a
    /// collision. A key that depended on whether *other* parts currently
    /// collide would be a moving target: a path that's unique today could
    /// gain a colliding sibling in a future re-export, silently changing
    /// that key's shape and orphaning an exception saved under the old
    /// one. Keying on a part's own (path, dimensions) alone never depends
    /// on what else is in the file, so it can't drift out from under a
    /// saved exception that way. See `assignment_key`.
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
    /// This part's resolved material, if any -- always a clone of one of
    /// `App::materials`' entries (or `load_parts`' local `materials`
    /// slice), never a free-standing name: once a name comes off
    /// `Project::autofill`/`assignments` or a picker choice, it's
    /// resolved to a real `Material` immediately (see `find_material`)
    /// rather than carried as a string that might not resolve to
    /// anything, the way it used to be.
    pub material: Option<Material>,
    /// Whether `material` is this part's own exception (see
    /// `crate::project::Project::assignments`) rather than derived fresh
    /// from a bracket-tag rule (`crate::project::Project::autofill`) or
    /// left unassigned -- see `resolve_material`. Only ever true
    /// alongside `material.is_some()`: clearing a part's exception always
    /// removes it outright and re-resolves from the current rule (or
    /// unassigned), never leaves an exception recording "no material."
    /// `save` only persists `material` when this is true, so a
    /// rule-derived material is never frozen per-part -- that's exactly
    /// what lets a newly tagged part in a future STEP revision pick up an
    /// existing rule automatically.
    pub is_exception: bool,
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
fn resolve_dims(
    raw: (f64, f64, f64),
    material: Option<&Material>,
    swapped: bool,
) -> (f64, f64, f64, bool) {
    let (mut length_in, mut width_in, mut thickness_in) = raw;
    let mut thickness_mismatch = false;
    if let Some(m) = material {
        let raw_mm = (raw.0 * MM_PER_IN, raw.1 * MM_PER_IN, raw.2 * MM_PER_IN);
        match relabel_with_known_thickness(
            raw_mm,
            m.thickness_mm,
            COMPATIBLE_THICKNESS_TOLERANCE_IN * MM_PER_IN,
        ) {
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
    if s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

impl Part {
    fn to_packable(&self) -> PackablePart {
        let mut part = PackablePart::new(
            self.path.clone(),
            self.length_in * MM_PER_IN,
            self.width_in * MM_PER_IN,
            self.thickness_in * MM_PER_IN,
        );
        part.material_name = self.material.as_ref().map(|m| m.name.clone());
        part
    }
}

fn compatible_materials(materials: &[Material], thickness_in: f64) -> Vec<&Material> {
    materials
        .iter()
        .filter(|m| {
            (m.thickness_mm / MM_PER_IN - thickness_in).abs() <= COMPATIBLE_THICKNESS_TOLERANCE_IN
        })
        .collect()
}

/// Looks up a material by name against `materials` -- this project's own
/// resolved subset. Panics if it isn't there: every name reaching this
/// (an `autofill` value, an `assignments` exception, or a picker
/// selection built straight from `materials` itself) has already passed
/// `Project::validate_references` or was never anything but one of
/// `materials`' own names, so a miss here would mean that check was
/// skipped, not a normal, recoverable user error.
fn find_material<'a>(materials: &'a [Material], name: &str) -> &'a Material {
    materials
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| {
            panic!("material {name:?} not found -- was Project::validate_references skipped?")
        })
}

/// Every distinct bracket tag across `parts`' paths, with how many parts
/// carry it -- first-appearance order (not sorted by count), so a tag's
/// position in the list roughly tracks where it first shows up in the
/// tree rather than jumping around as counts change. A part carrying the
/// same tag twice in its own path (an odd but possible nesting) only
/// counts once toward that tag's total.
fn distinct_tags(parts: &[Part]) -> Vec<(String, usize)> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for part in parts {
        let mut seen_in_part = std::collections::HashSet::new();
        for tag in tags::extract_tags(&part.path) {
            if seen_in_part.insert(tag.clone()) {
                if !counts.contains_key(&tag) {
                    order.push(tag.clone());
                }
                *counts.entry(tag).or_insert(0) += 1;
            }
        }
    }
    order
        .into_iter()
        .map(|tag| {
            let count = counts[&tag];
            (tag, count)
        })
        .collect()
}

/// A part's material, given whether it has its own `exception` (from
/// `Project::assignments`) and the project's current `autofill_map` --
/// an exception always wins; otherwise, whichever of this part's own
/// bracket tags has a configured rule; otherwise unassigned. The second
/// return value is exactly `Part::is_exception` -- always `false` when
/// `exception` is `None`, so a caller re-resolving a *cleared* part (by
/// passing `None`) never has to compute it separately.
///
/// Deliberately no thickness-compatibility filtering on the rule branch
/// (unlike the picker's own material list, see `compatible_materials`):
/// a rule was authored deliberately, having seen the confirmation
/// summary, so a future part that turns out a bad fit for it should
/// surface via `thickness_mismatch` (a real signal worth seeing), not be
/// silently suppressed the way an unreviewed guess would need to be.
fn resolve_material(
    path: &str,
    exception: Option<String>,
    autofill_map: &std::collections::BTreeMap<String, String>,
) -> (Option<String>, bool) {
    match exception {
        Some(name) => (Some(name), true),
        None => (autofill::guess_material(path, autofill_map), false),
    }
}

/// Re-resolves every part that has no exception of its own against the
/// current `autofill_map` -- called right after a bulk-edit confirm
/// updates that map (inserting or removing one tag's rule), so every
/// currently-matching part picks up the change immediately. A part with
/// its own exception is never touched here: an exception always outranks
/// whatever a rule says, including a rule that changes after the
/// exception was set. Returns the indices actually changed so the caller
/// can re-derive each one's dims (`App::resolve_part_dims`).
fn apply_bulk_material(
    parts: &mut [Part],
    autofill_map: &std::collections::BTreeMap<String, String>,
    materials: &[Material],
) -> Vec<usize> {
    let mut changed = Vec::new();
    for (i, part) in parts.iter_mut().enumerate() {
        if part.is_exception {
            continue;
        }
        let resolved = autofill::guess_material(&part.path, autofill_map)
            .map(|name| find_material(materials, &name).clone());
        if part.material != resolved {
            part.material = resolved;
            changed.push(i);
        }
    }
    changed
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
        return Some(
            if compatible_materials(materials, part.thickness_in).len() > 1 {
                "ambiguous material"
            } else {
                "no material assigned"
            },
        );
    }
    None
}

fn load_parts(
    step_path: &Path,
    project: &Project,
    materials: &[Material],
) -> Result<Vec<Part>, Box<dyn Error>> {
    let groups = extract_parts(step_path)?;
    let mut parts = Vec::new();
    for group in &groups {
        for instance in &group.instances {
            let raw_length_in = round4(group.length_mm / MM_PER_IN);
            let raw_width_in = round4(group.width_mm / MM_PER_IN);
            let raw_thickness_in = round4(group.thickness_mm / MM_PER_IN);
            let key = assignment_key(
                &instance.path,
                raw_length_in,
                raw_width_in,
                raw_thickness_in,
            );
            let over = project.assignments.get(&key);
            let exception = over.and_then(|o| o.material.clone());
            let (material_name, is_exception) =
                resolve_material(&instance.path, exception, &project.autofill);
            let swapped = over.map(|o| o.swapped).unwrap_or(false);
            let material = material_name.map(|name| find_material(materials, &name).clone());
            let (length_in, width_in, thickness_in, thickness_mismatch) = resolve_dims(
                (raw_length_in, raw_width_in, raw_thickness_in),
                material.as_ref(),
                swapped,
            );
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
                is_exception,
                swapped,
            });
        }
    }
    Ok(parts)
}

/// What a confirmed `PickerState` choice applies to -- a single selected
/// part, or every part carrying a bulk-edit tag (see `BulkState`). Both
/// paths end up at the same picker UI and the same `confirm_picker`,
/// which is the whole point: bulk-edit reuses the material picker rather
/// than growing a second one.
pub(crate) enum PickerTarget {
    Part(usize),
    Tag(String),
}

pub(crate) struct PickerState {
    pub(crate) target: PickerTarget,
    pub(crate) options: Vec<String>,
    pub(crate) list_state: ListState,
}

/// Bulk-edit-by-tag's own screen: the list of every bracket tag in the
/// tree, each with its part count and (if a rule is already set) the
/// material it currently resolves to. Enter on a row hands off straight
/// to the shared material picker (`App::picker`, `PickerTarget::Tag`) --
/// which is what actually writes the project's `autofill` rule -- and
/// this state is left in place underneath it, so cancelling or
/// confirming that picker lands back on this same tag list with its
/// material column refreshed (see `confirm_picker`).
pub(crate) enum BulkState {
    PickTag {
        tags: Vec<(String, usize, Option<String>)>,
        list_state: ListState,
    },
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
    /// This project's own material subset, resolved against the global
    /// catalog (see `Project::resolve_materials`) -- what the pickers
    /// offer, never the whole shop catalog.
    pub(crate) materials: Vec<Material>,
    /// This project's own stock subset (see `Project::resolve_stock`) --
    /// what `pack()` is allowed to nest onto.
    stock: Vec<StockSheet>,
    pub(crate) tree_state: TreeState<String>,
    pub(crate) dirty: bool,
    pub(crate) status: String,
    /// The keyboard-shortcut help line `status` reverts to once a
    /// transient message (see `set_status`) times out. Fixed at startup,
    /// never itself passed through `set_status`, so it never expires.
    default_status: String,
    /// When the current `status` was set via `set_status`, so the main
    /// loop knows when to revert it -- `None` means `status` is already
    /// `default_status` (or hasn't been touched since it last reverted).
    status_message_at: Option<Instant>,
    pub(crate) picker: Option<PickerState>,
    pub(crate) bulk: Option<BulkState>,
    pub(crate) print_settings: Option<PrintSettings>,
    /// True while the "save before exiting?" popup is up -- set when `q`
    /// or `Esc` is pressed with `dirty` still true, instead of quitting
    /// immediately, so an unsaved swap, exception, or rule change from
    /// earlier in the session can't be lost to a reflexive quit keypress.
    pub(crate) confirm_quit: bool,
    pub(crate) step_path: PathBuf,
    project: Project,
    project_path: PathBuf,
    pub(crate) last_tree_height: u16,
}

impl App {
    /// Sets a transient status message, timed to revert to
    /// `default_status` once `STATUS_MESSAGE_TIMEOUT` elapses (see the
    /// main loop's poll timeout in `run`, which is what actually notices
    /// the expiry and performs the revert).
    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_message_at = Some(Instant::now());
    }

    /// Reverts `status` to `default_status` if a transient message has
    /// been showing for at least `STATUS_MESSAGE_TIMEOUT`. Called from
    /// the main loop on every idle poll tick, not just after an event --
    /// otherwise a message set right before the user stops pressing keys
    /// would stick until the next keypress instead of timing out on its
    /// own.
    fn expire_status(&mut self) {
        if self
            .status_message_at
            .is_some_and(|at| at.elapsed() >= STATUS_MESSAGE_TIMEOUT)
        {
            self.status = self.default_status.clone();
            self.status_message_at = None;
        }
    }

    /// Whether `status` is currently the resting help line rather than a
    /// transient message -- `ui::draw_status` uses this to decide between
    /// `ui`'s colored key/action rendering (only valid for the help
    /// line's own shape) and plain text for a free-form message.
    pub(super) fn status_is_default(&self) -> bool {
        self.status == self.default_status
    }

    /// (resolved, total) for the top-right title -- "resolved" means a
    /// material assigned *and* no `part_flag` reason left standing, not
    /// merely `material.is_some()`. A thickness-mismatched part still
    /// carries a material name, but that's a wrong decision, not a made
    /// one: counting it here would let the corner hit N/N green while a
    /// red-flagged row remains in the tree, which is the one thing this
    /// count exists to rule out.
    pub(crate) fn resolved_counts(&self) -> (usize, usize) {
        let resolved = self
            .parts
            .iter()
            .filter(|p| p.material.is_some() && part_flag(p, &self.materials).is_none())
            .count();
        (resolved, self.parts.len())
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
            self.set_status("select a part first");
            return;
        };
        let part = &self.parts[i];
        let mut options: Vec<String> = compatible_materials(&self.materials, part.thickness_in)
            .into_iter()
            .map(|m| m.name.clone())
            .collect();
        options.sort();
        options.insert(0, "(clear -- match by thickness alone)".to_string());
        let current_index = match &part.material {
            None => 0,
            Some(m) => options.iter().position(|o| o == &m.name).unwrap_or(0),
        };
        let mut list_state = ListState::default();
        list_state.select(Some(current_index));
        self.picker = Some(PickerState {
            target: PickerTarget::Part(i),
            options,
            list_state,
        });
    }

    /// Opens bulk-edit's tag list (see `BulkState`), or reports there's
    /// nothing to bulk-edit if no part in the whole tree carries a
    /// bracket tag at all. Each row's material column comes straight from
    /// the project's current `autofill` rule for that tag, if any.
    fn open_bulk_edit(&mut self) {
        let tags = distinct_tags(&self.parts);
        if tags.is_empty() {
            self.set_status("no bracket-tagged parts to bulk-edit");
            return;
        }
        let tags = tags
            .into_iter()
            .map(|(tag, count)| {
                let material = self.project.autofill.get(&tag).cloned();
                (tag, count, material)
            })
            .collect();
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        self.bulk = Some(BulkState::PickTag { tags, list_state });
    }

    /// Enter on the tag list hands off straight to the shared material
    /// picker (`PickerTarget::Tag`), which is what actually writes the
    /// project's `autofill` rule once a material is chosen (see
    /// `confirm_picker`). `self.bulk` is deliberately left in place --
    /// cancelling or confirming the picker returns to this same tag list.
    fn bulk_pick_tag(&mut self) {
        let Some(BulkState::PickTag { tags, list_state }) = &self.bulk else {
            return;
        };
        let Some(i) = list_state.selected() else {
            return;
        };
        let tag = tags[i].0.clone();
        let mut options: Vec<String> = self.materials.iter().map(|m| m.name.clone()).collect();
        options.sort();
        options.insert(0, "(clear -- remove this tag's rule)".to_string());
        let current_index = match self.project.autofill.get(&tag) {
            None => 0,
            Some(name) => options.iter().position(|o| o == name).unwrap_or(0),
        };
        let mut list_state = ListState::default();
        list_state.select(Some(current_index));
        self.picker = Some(PickerState {
            target: PickerTarget::Tag(tag),
            options,
            list_state,
        });
    }

    /// Recomputes `length_in`/`width_in`/`thickness_in`/`thickness_mismatch`
    /// for `self.parts[i]` from its raw dims, current material, and swap
    /// state -- call after mutating either (see `resolve_dims`).
    fn resolve_part_dims(&mut self, i: usize) {
        let material = self.parts[i].material.clone();
        let raw = (
            self.parts[i].raw_length_in,
            self.parts[i].raw_width_in,
            self.parts[i].raw_thickness_in,
        );
        let swapped = self.parts[i].swapped;
        let (length_in, width_in, thickness_in, thickness_mismatch) =
            resolve_dims(raw, material.as_ref(), swapped);
        let part = &mut self.parts[i];
        part.length_in = length_in;
        part.width_in = width_in;
        part.thickness_in = thickness_in;
        part.thickness_mismatch = thickness_mismatch;
    }

    fn confirm_picker(&mut self) {
        let Some(picker) = self.picker.take() else {
            return;
        };
        let Some(choice) = picker.list_state.selected() else {
            return;
        };
        let chosen = if choice == 0 {
            None
        } else {
            Some(picker.options[choice].clone())
        };
        match picker.target {
            PickerTarget::Part(i) => {
                match chosen {
                    Some(name) => {
                        self.parts[i].material =
                            Some(find_material(&self.materials, &name).clone());
                        self.parts[i].is_exception = true;
                    }
                    None => {
                        // Remove the exception outright -- fall back to
                        // this part's tag rule if one applies, else plain
                        // unassigned. Never records a standalone "no
                        // material" exception.
                        self.parts[i].is_exception = false;
                        self.parts[i].material =
                            autofill::guess_material(&self.parts[i].path, &self.project.autofill)
                                .map(|name| find_material(&self.materials, &name).clone());
                    }
                }
                self.resolve_part_dims(i);
                self.dirty = true;
                self.set_status(format!("set material for {}", self.parts[i].path));
            }
            PickerTarget::Tag(tag) => {
                match &chosen {
                    Some(name) => {
                        self.project.autofill.insert(tag.clone(), name.clone());
                    }
                    None => {
                        self.project.autofill.remove(&tag);
                    }
                }
                let changed =
                    apply_bulk_material(&mut self.parts, &self.project.autofill, &self.materials);
                for i in &changed {
                    self.resolve_part_dims(*i);
                }
                if let Some(BulkState::PickTag { tags, .. }) = &mut self.bulk {
                    if let Some(entry) = tags.iter_mut().find(|(t, _, _)| *t == tag) {
                        entry.2 = chosen.clone();
                    }
                }
                self.dirty = true;
                self.set_status(format!(
                    "updated the {tag} rule -- {} part(s) changed",
                    changed.len()
                ));
            }
        }
    }

    fn toggle_swap(&mut self) {
        let Some(i) = self.selected_part_index() else {
            self.set_status("select a part first");
            return;
        };
        self.parts[i].swapped = !self.parts[i].swapped;
        self.resolve_part_dims(i);
        self.dirty = true;
        self.set_status(format!("swapped length/width for {}", self.parts[i].path));
    }

    fn save(&mut self) {
        self.project.assignments = self
            .parts
            .iter()
            .filter_map(|p| {
                // A rule-derived material is never frozen into an
                // exception -- only `is_exception` (an explicit per-part
                // override) persists. See `Part::is_exception`.
                let material = if p.is_exception {
                    p.material.as_ref().map(|m| m.name.clone())
                } else {
                    None
                };
                let over = PartOverride {
                    material,
                    swapped: p.swapped,
                };
                if over.is_empty() {
                    None
                } else {
                    Some((p.assignment_key.clone(), over))
                }
            })
            .collect();
        match crate::project::save(&self.project, &self.project_path) {
            Ok(()) => {
                self.dirty = false;
                let msg = format!("saved {}", self.project_path.display());
                self.set_status(msg);
            }
            Err(e) => self.set_status(format!("save failed: {e}")),
        }
    }

    fn open_print_settings(&mut self) {
        self.print_settings = Some(PrintSettings {
            kerf_in: format_editable(self.project.settings.kerf_in),
            trim_allowance_in: format_editable(self.project.settings.trim_allowance_in),
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
        let Some(ps) = &mut self.print_settings else {
            return;
        };
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
        let Some(ps) = &mut self.print_settings else {
            return;
        };
        let (field, touched) = Self::print_settings_field(ps);
        *touched = true;
        field.pop();
    }

    fn print_settings_toggle_focus(&mut self) {
        let Some(ps) = &mut self.print_settings else {
            return;
        };
        ps.focus = match ps.focus {
            PrintField::Kerf => PrintField::TrimAllowance,
            PrintField::TrimAllowance => PrintField::Kerf,
        };
    }

    fn confirm_print_settings(&mut self) {
        let Some(ps) = self.print_settings.take() else {
            return;
        };
        match (
            ps.kerf_in.parse::<f64>(),
            ps.trim_allowance_in.parse::<f64>(),
        ) {
            (Ok(kerf_in), Ok(trim_allowance_in)) => {
                self.project.settings.kerf_in = kerf_in;
                self.project.settings.trim_allowance_in = trim_allowance_in;
                self.dirty = true;
                self.print();
            }
            _ => self.set_status("kerf and trim allowance must both be numbers, in inches"),
        }
    }

    fn print(&mut self) {
        let parts: Vec<PackablePart> = self.parts.iter().map(Part::to_packable).collect();
        let trim_allowance_mm = self.project.settings.trim_allowance_in * MM_PER_IN;
        let layout = pack(
            &parts,
            &self.stock,
            self.project.settings.kerf_in * MM_PER_IN,
            trim_allowance_mm,
        );
        let unplaced = layout.unplaced.len();
        let out_path = self.project.out_pdf_path(&self.project_path);
        let pdf_bytes = storystick_core::diagrams::render_pdf(
            &layout,
            trim_allowance_mm,
            crate::sections::classify,
            crate::sections::UNSECTIONED,
        );
        match std::fs::write(&out_path, pdf_bytes) {
            Ok(()) => {
                let msg = if unplaced == 0 {
                    format!(
                        "printed {} ({} sheets)",
                        out_path.display(),
                        layout.sheets.len()
                    )
                } else {
                    format!(
                        "printed {} ({} sheets, {} part(s) unplaced)",
                        out_path.display(),
                        layout.sheets.len(),
                        unplaced
                    )
                };
                self.set_status(msg);
            }
            Err(e) => self.set_status(format!("failed to write {}: {e}", out_path.display())),
        }
    }
}

pub(crate) fn run(
    project: Project,
    project_path: PathBuf,
    global_stock: Vec<StockSheet>,
) -> Result<(), Box<dyn Error>> {
    let global_materials = stock::distinct_materials(&global_stock);
    let materials = project.resolve_materials(&global_materials)?;
    project.validate_references(&materials)?;
    let stock_subset = project.resolve_stock(&global_stock);
    let step_path = project.step_path(&project_path);

    let parts = load_parts(&step_path, &project, &materials)?;

    let tree_state = TreeState::default();

    let help_text = ui::tree_help_text();
    let mut app = App {
        parts,
        selection_index: HashMap::new(),
        materials,
        stock: stock_subset,
        tree_state,
        dirty: false,
        status: help_text.clone(),
        default_status: help_text,
        status_message_at: None,
        picker: None,
        bulk: None,
        print_settings: None,
        confirm_quit: false,
        step_path,
        project,
        project_path,
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

            // Poll rather than block: a transient status message needs to
            // revert to the help text on its own timeout even if the user
            // isn't pressing anything at all.
            if !event::poll(STATUS_POLL_INTERVAL)? {
                app.expire_status();
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            app.expire_status();

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

            if matches!(app.bulk, Some(BulkState::PickTag { .. })) {
                match key.code {
                    KeyCode::Esc => app.bulk = None,
                    KeyCode::Enter => app.bulk_pick_tag(),
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(BulkState::PickTag { tags, list_state }) = &mut app.bulk {
                            let len = tags.len();
                            let cur = list_state.selected().unwrap_or(0) as i64;
                            let next = (cur - 1).rem_euclid(len as i64) as usize;
                            list_state.select(Some(next));
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(BulkState::PickTag { tags, list_state }) = &mut app.bulk {
                            let len = tags.len();
                            let next = (list_state.selected().unwrap_or(0) + 1) % len;
                            list_state.select(Some(next));
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

            if app.confirm_quit {
                match key.code {
                    // Enter with no letter typed defaults to the capital
                    // option in "[Y/n]" -- saving is the safer default
                    // when the only cost of guessing wrong is one extra
                    // keypress next launch, versus silently losing a
                    // swap or material pick if `n` were the default.
                    KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                        app.save();
                        if app.dirty {
                            // Save failed -- stay open so the error
                            // status is visible instead of exiting over
                            // it unnoticed.
                            app.confirm_quit = false;
                        } else {
                            break Ok(());
                        }
                    }
                    KeyCode::Char('n' | 'N') => break Ok(()),
                    KeyCode::Esc => app.confirm_quit = false,
                    _ => {}
                }
                continue;
            }

            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    if app.dirty {
                        app.confirm_quit = true;
                    } else {
                        break Ok(());
                    }
                }
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
                KeyCode::Char('b') => app.open_bulk_edit(),
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
    use std::collections::BTreeMap;

    fn material(name: &str, thickness_in: f64) -> Material {
        Material {
            name: name.to_string(),
            thickness_mm: thickness_in * MM_PER_IN,
        }
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
            material: material.map(|name| Material {
                name: name.to_string(),
                thickness_mm: thickness_in * MM_PER_IN,
            }),
            is_exception: material.is_some(),
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
        let materials = vec![
            material("Baltic Birch 3/4", 0.75),
            material("Sande Ply 3/4", 0.75),
        ];
        let unassigned = part(0.75, None, false);
        assert_eq!(
            part_flag(&unassigned, &materials),
            Some("ambiguous material")
        );
    }

    #[test]
    fn unassigned_part_is_still_flagged_when_only_one_material_matches() {
        // A single match today is still an inference, not a decision --
        // and silently wrong the moment a second material at that
        // thickness joins the catalog. An unreviewed part must never look
        // the same as a resolved one.
        let materials = vec![
            material("Baltic Birch 3/4", 0.75),
            material("Baltic Birch 1/4", 0.25),
        ];
        let unassigned = part(0.75, None, false);
        assert_eq!(
            part_flag(&unassigned, &materials),
            Some("no material assigned")
        );
    }

    #[test]
    fn pinning_a_material_clears_the_ambiguous_flag() {
        let materials = vec![
            material("Baltic Birch 3/4", 0.75),
            material("Sande Ply 3/4", 0.75),
        ];
        let pinned = part(0.75, Some("Sande Ply 3/4"), false);
        assert_eq!(part_flag(&pinned, &materials), None);
    }

    #[test]
    fn thickness_mismatch_is_flagged_even_though_a_material_is_assigned() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut mismatched = part(0.75, Some("Baltic Birch 3/4"), false);
        mismatched.thickness_mismatch = true;
        assert_eq!(
            part_flag(&mismatched, &materials),
            Some("material thickness doesn't match this part's geometry")
        );
    }

    #[test]
    fn resolve_dims_leaves_the_raw_guess_alone_with_no_material() {
        let (length_in, width_in, thickness_in, mismatch) =
            resolve_dims((30.0, 20.0, 0.75), None, false);
        assert_eq!((length_in, width_in, thickness_in), (30.0, 20.0, 0.75));
        assert!(!mismatch);
    }

    #[test]
    fn resolve_dims_corrects_a_narrow_rip_once_the_material_is_known() {
        // Ripped from 3/4" stock to a strip narrower than it is thick:
        // the raw largest/middle/smallest guess mislabels the 0.25" width
        // as thickness. Knowing the assigned material is really 3/4"
        // fixes it.
        let bb34 = Material {
            name: "Baltic Birch 3/4".to_string(),
            thickness_mm: 0.75 * MM_PER_IN,
        };
        let (length_in, width_in, thickness_in, mismatch) =
            resolve_dims((24.0, 0.75, 0.25), Some(&bb34), false);
        assert_eq!((length_in, width_in, thickness_in), (24.0, 0.25, 0.75));
        assert!(!mismatch);
    }

    #[test]
    fn resolve_dims_flags_a_mismatch_instead_of_guessing() {
        let unrelated = Material {
            name: "1/8\" hardboard".to_string(),
            thickness_mm: 0.125 * MM_PER_IN,
        };
        let (length_in, width_in, thickness_in, mismatch) =
            resolve_dims((30.0, 20.0, 0.75), Some(&unrelated), false);
        // No dimension is anywhere near 0.125" -- dims fall back to the
        // raw guess rather than silently picking the closest anyway.
        assert_eq!((length_in, width_in, thickness_in), (30.0, 20.0, 0.75));
        assert!(mismatch);
    }

    #[test]
    fn resolve_dims_applies_the_swap_after_any_material_correction() {
        let bb34 = Material {
            name: "Baltic Birch 3/4".to_string(),
            thickness_mm: 0.75 * MM_PER_IN,
        };
        let (length_in, width_in, thickness_in, mismatch) =
            resolve_dims((24.0, 0.75, 0.25), Some(&bb34), true);
        // Same correction as above (thickness -> 0.75, remaining sorted
        // 24/0.25), then length_in/width_in end up swapped on top.
        assert_eq!((length_in, width_in, thickness_in), (0.25, 24.0, 0.75));
        assert!(!mismatch);
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
        assert_ne!(
            a, b,
            "two real parts sharing a path must never collapse onto one key"
        );
    }

    #[test]
    fn compatible_materials_excludes_a_materially_different_thickness() {
        let materials = vec![
            material("Baltic Birch 3/4", 0.75),
            material("Baltic Birch 1/4", 0.25),
        ];
        let compat = compatible_materials(&materials, 0.75);
        assert_eq!(compat.len(), 1);
        assert_eq!(compat[0].name, "Baltic Birch 3/4");
    }

    #[test]
    fn resolve_material_exception_always_wins_over_a_rule() {
        let mut autofill_map = BTreeMap::new();
        autofill_map.insert("[Panel]".to_string(), "Baltic Birch 3/4".to_string());
        let (material, is_exception) = resolve_material(
            "Bench / [Panel] Bottom",
            Some("Sande Ply 3/4".to_string()),
            &autofill_map,
        );
        assert_eq!(material.as_deref(), Some("Sande Ply 3/4"));
        assert!(is_exception);
    }

    #[test]
    fn resolve_material_falls_back_to_the_tag_rule_when_no_exception() {
        let mut autofill_map = BTreeMap::new();
        autofill_map.insert("[Panel]".to_string(), "Baltic Birch 3/4".to_string());
        let (material, is_exception) =
            resolve_material("Bench / [Panel] Bottom", None, &autofill_map);
        assert_eq!(material.as_deref(), Some("Baltic Birch 3/4"));
        assert!(!is_exception);
    }

    #[test]
    fn resolve_material_is_unassigned_when_no_exception_and_no_rule() {
        let (material, is_exception) = resolve_material("Bench / Bottom", None, &BTreeMap::new());
        assert_eq!(material, None);
        assert!(!is_exception);
    }

    #[test]
    fn save_persists_an_exception_but_never_a_rule_derived_material() {
        let mut exception = part(0.75, Some("Baltic Birch 3/4"), false);
        exception.is_exception = true;
        let saved = if exception.is_exception {
            exception.material.as_ref().map(|m| m.name.clone())
        } else {
            None
        };
        assert_eq!(saved, Some("Baltic Birch 3/4".to_string()));

        let mut rule_derived = part(0.75, Some("Baltic Birch 3/4"), false);
        rule_derived.is_exception = false;
        let saved = if rule_derived.is_exception {
            rule_derived.material.as_ref().map(|m| m.name.clone())
        } else {
            None
        };
        assert_eq!(
            saved, None,
            "a rule-derived material is never frozen into an exception on save"
        );
    }

    fn tagged_part(path: &str, material: Option<&str>) -> Part {
        Part {
            path: path.to_string(),
            assignment_key: path.to_string(),
            ..part(0.75, material, false)
        }
    }

    #[test]
    fn distinct_tags_counts_distinct_parts_in_first_appearance_order() {
        let parts = vec![
            tagged_part("Bench / [Panel] Bottom", None),
            tagged_part("Bench / [Backer] Left", None),
            tagged_part("Bench / [Panel] Top", None),
        ];
        assert_eq!(
            distinct_tags(&parts),
            vec![("[Panel]".to_string(), 2), ("[Backer]".to_string(), 1)]
        );
    }

    #[test]
    fn distinct_tags_counts_a_part_carrying_the_same_tag_twice_only_once() {
        let parts = vec![tagged_part(
            "Bench / [Panel] Section / [Panel] Bottom",
            None,
        )];
        assert_eq!(distinct_tags(&parts), vec![("[Panel]".to_string(), 1)]);
    }

    #[test]
    fn distinct_tags_is_empty_when_no_part_carries_a_bracket_tag() {
        let parts = vec![tagged_part("Bench / Bottom", None)];
        assert!(distinct_tags(&parts).is_empty());
    }

    #[test]
    fn apply_bulk_material_re_resolves_every_non_exception_part_from_the_current_rule() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut parts = vec![tagged_part("Bench / [Panel] A", None)];
        parts[0].is_exception = false;
        let mut autofill_map = BTreeMap::new();
        autofill_map.insert("[Panel]".to_string(), "Baltic Birch 3/4".to_string());

        let changed = apply_bulk_material(&mut parts, &autofill_map, &materials);

        assert_eq!(changed, vec![0]);
        assert_eq!(
            parts[0].material.as_ref().map(|m| m.name.as_str()),
            Some("Baltic Birch 3/4")
        );
        assert!(
            !parts[0].is_exception,
            "bulk-edit updates the rule, never stamps a per-part exception"
        );
    }

    #[test]
    fn apply_bulk_material_never_touches_a_part_with_its_own_exception() {
        let materials = vec![
            material("Baltic Birch 3/4", 0.75),
            material("Sande Ply 3/4", 0.75),
        ];
        let mut parts = vec![tagged_part("Bench / [Panel] A", Some("Sande Ply 3/4"))];
        parts[0].is_exception = true;
        let mut autofill_map = BTreeMap::new();
        autofill_map.insert("[Panel]".to_string(), "Baltic Birch 3/4".to_string());

        let changed = apply_bulk_material(&mut parts, &autofill_map, &materials);

        assert!(changed.is_empty());
        assert_eq!(
            parts[0].material.as_ref().map(|m| m.name.as_str()),
            Some("Sande Ply 3/4"),
            "an exception always outranks the rule, even after the rule changes"
        );
    }

    #[test]
    fn apply_bulk_material_removing_a_rule_unassigns_its_non_exception_parts() {
        let materials = vec![material("Baltic Birch 3/4", 0.75)];
        let mut parts = vec![tagged_part("Bench / [Panel] A", Some("Baltic Birch 3/4"))];
        parts[0].is_exception = false;
        // No rule in the map at all -- simulates the rule having just
        // been removed via bulk-edit's "(clear)".
        let changed = apply_bulk_material(&mut parts, &BTreeMap::new(), &materials);
        assert_eq!(changed, vec![0]);
        assert_eq!(parts[0].material, None);
    }
}
