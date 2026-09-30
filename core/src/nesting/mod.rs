//! Sheet-goods nesting.
//!
//! Packs PackablePart rectangles onto StockSheet panels via a rip-first
//! guillotine packer, bucketed by matching thickness -- or by a specific
//! named Material, when a part pins one -- so a part is only ever
//! considered against stock it could actually come from. Matching by
//! thickness alone was the actual defect this replaces
//! cutlistoptimizer.com for (it pools every thickness onto the same
//! virtual sheet regardless of the Material column); matching by name is
//! what additionally lets two materials share a thickness (a cheap
//! utility ply and a show-face ply, say) without a part ever landing on
//! the wrong one by accident.
//!
//! Deliberately has no dependency on `crate::stepcrawl`: its input is this
//! module's own minimal `PackablePart` type, not a `stepcrawl::PartGroup`,
//! so it stays usable against any source of parts, not just STEP files.
//!
//! Design decisions settled by hand:
//!   - Rip-first guillotine, not generic guillotine bin-packing. Any
//!     guillotine cut sequence is guaranteed *achievable*, but a generic
//!     solver is free to choose a crosscut-first split wherever its
//!     heuristic finds it locally denser -- unsafe on a full, untouched
//!     sheet (a full sheet on a table saw is dangerous; a track saw isn't
//!     accurate enough for the repeated, matching cuts real parts need).
//!     The constraint only really applies to the *first* cut on a fresh
//!     sheet, though: once a strip is isolated by that rip, it's already
//!     small and manageable, so anything further within it is a normal,
//!     unconstrained 2D guillotine problem. So each strip (`Strip`) always
//!     spans the sheet's full length_mm -- that's the one rip, made once
//!     per strip -- but *within* a strip, parts are packed via ordinary
//!     free-rectangle guillotine splitting (either orientation, leftover
//!     space reused), not forced into a single uniform row. Real cost,
//!     still: a strip's height is fixed by its first (widest) member, so
//!     it packs somewhat less densely than a fully unconstrained solver
//!     working across the whole sheet at once -- the trade for a cut
//!     sequence that's actually safe and accurate to execute.
//!   - rotation: never. Nothing here rotates a part 90 degrees, ever, no
//!     per-part exception. Grain runs along a stock sheet's length_mm by
//!     convention, and identical parts land in a consistent orientation
//!     for repeatable fence cuts -- both satisfied for free by simply
//!     never rotating. Deciding which of a part's two in-plane
//!     dimensions should be called length_mm (and therefore run with the
//!     grain) is the caller's job, made once before a `PackablePart` is
//!     even built -- not something this module has an opinion on.
//!   - Bucketed by (material name or thickness, then stock size): each
//!     candidate StockSheet size within a bucket is tried largest-first,
//!     with whatever doesn't fit carried over to the next candidate. For
//!     the common case (one stock size per thickness, no material
//!     pinned) this is just a single pass.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

pub const DEFAULT_KERF_MM: f64 = 3.2; // ~1/8"

/// A named material, looked up by name -- e.g. "Baltic Birch 3/4
/// (finished 2 sides)" vs "Sande Ply 3/4 (utility)". Two materials can
/// share a thickness_mm while being genuinely different stock, bought and
/// used for different reasons (a hidden stretcher doesn't need show-face
/// plywood). Thickness alone was never a strong enough key for "what
/// should this part be cut from" -- that's what this type is for.
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub thickness_mm: f64,
}

// f64 isn't Eq/Hash (NaN), so these are implemented by hand comparing bit
// patterns -- safe here because every comparison is between values that
// trace back to the same original input (a Material/StockSheet cloned
// through the packing pipeline), never two independently computed floats
// that might differ by an ULP.
impl PartialEq for Material {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.thickness_mm.to_bits() == other.thickness_mm.to_bits()
    }
}
impl Eq for Material {}
impl Hash for Material {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.thickness_mm.to_bits().hash(state);
    }
}

/// One part to be nested. `qty` copies are nested independently (each
/// gets its own placement), not as one rectangle labeled "x3" -- `pack`
/// doesn't assume they end up anywhere near each other.
///
/// length_mm is always the dimension that runs with the stock sheet's
/// grain (grain runs along a sheet's length_mm by convention -- see this
/// module's docs); a caller that wants a part's *other* dimension running
/// with the grain instead swaps length_mm/width_mm before constructing
/// this, since `pack` itself never rotates a part.
///
/// `material_name = None` (the default) means "any material at this
/// part's own thickness_mm is fine" -- `pack` matches purely by
/// thickness, as it always has. Set it to pin this part to one specific
/// Material by name, e.g. to keep a hidden part off the good plywood even
/// though it happens to share a thickness with it.
#[derive(Debug, Clone, PartialEq)]
pub struct PackablePart {
    pub label: String,
    pub length_mm: f64,
    pub width_mm: f64,
    pub thickness_mm: f64,
    pub qty: u32,
    pub material_name: Option<String>,
}

impl PackablePart {
    pub fn new(label: impl Into<String>, length_mm: f64, width_mm: f64, thickness_mm: f64) -> Self {
        Self {
            label: label.into(),
            length_mm,
            width_mm,
            thickness_mm,
            qty: 1,
            material_name: None,
        }
    }
}

/// One purchasable sheet size of a given Material -- what you *can* buy,
/// not an inventory count of what you have. There's deliberately no
/// qty-on-hand field: `pack` always assumes you can buy as many of a
/// given StockSheet as needed, and `bill_of_materials` tells you how many
/// that turned out to be. If you already have some material on hand,
/// that's a manual adjustment you make on the BOM afterward, not
/// something this catalog tracks.
#[derive(Debug, Clone)]
pub struct StockSheet {
    pub material: Material,
    pub length_mm: f64,
    pub width_mm: f64,
}

impl StockSheet {
    pub fn thickness_mm(&self) -> f64 {
        self.material.thickness_mm
    }
}

impl PartialEq for StockSheet {
    fn eq(&self, other: &Self) -> bool {
        self.material == other.material
            && self.length_mm.to_bits() == other.length_mm.to_bits()
            && self.width_mm.to_bits() == other.width_mm.to_bits()
    }
}
impl Eq for StockSheet {}
impl Hash for StockSheet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.material.hash(state);
        self.length_mm.to_bits().hash(state);
        self.width_mm.to_bits().hash(state);
    }
}

/// Where one physical piece landed on its sheet. length_mm/width_mm are
/// the part's true final size -- never the kerf/trim-inflated footprint
/// `pack` actually feeds the packing algorithm.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub part_label: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub length_mm: f64,
    pub width_mm: f64,
    pub rotated: bool,
}

/// A cut holds one coordinate fixed and runs across the other -- `Rip`
/// fixes a y_mm (position across the sheet's width) and runs along
/// length_mm, same as the strip-defining rip this module is named for;
/// `Crosscut` fixes an x_mm (position along length_mm) and runs along
/// width_mm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutKind {
    Rip,
    Crosscut,
}

/// One straight guillotine cut. `cuts` on `SheetLayout` lists these in
/// the order they must actually be made: each cut only ever divides a
/// board that the cuts before it already produced (see `pack`'s docs),
/// so working through the list in order is always physically executable,
/// never "cut a piece that isn't isolated yet."
///
/// `position_mm` and `span_start_mm`/`span_end_mm` are measured from the
/// same reference corner every `Placement.x_mm`/`y_mm` already is (see
/// `crate::diagrams::mark_reference_corner`) -- exactly right for marking
/// every cut line on the whole, still-intact sheet before making a
/// single cut (see `crate::diagrams::render_rip_dimensions`), the same
/// way a placement's own coordinates are right for a diagram of the
/// finished layout. It is *not* a claim that this corner stays
/// physically reachable cut after cut: the first cut that removes
/// material between the corner and a later cut line takes that
/// reachability with it. `cut_steps` turns this sheet-wide coordinate
/// into the distance-from-the-piece-in-hand a woodworker actually needs
/// for a step-by-step breakdown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cut {
    pub kind: CutKind,
    /// The fixed coordinate: y_mm for a `Rip`, x_mm for a `Crosscut`.
    pub position_mm: f64,
    /// The cut's extent along the other axis: x_mm range for a `Rip`,
    /// y_mm range for a `Crosscut`.
    pub span_start_mm: f64,
    pub span_end_mm: f64,
}

impl Cut {
    fn rip(position_mm: f64, span_start_mm: f64, span_end_mm: f64) -> Self {
        Self {
            kind: CutKind::Rip,
            position_mm,
            span_start_mm,
            span_end_mm,
        }
    }

    fn crosscut(position_mm: f64, span_start_mm: f64, span_end_mm: f64) -> Self {
        Self {
            kind: CutKind::Crosscut,
            position_mm,
            span_start_mm,
            span_end_mm,
        }
    }
}

/// One physical sheet (the `sheet_index`-th copy of `stock` used) and
/// everything placed on it.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetLayout {
    pub stock: StockSheet,
    pub sheet_index: usize,
    pub placements: Vec<Placement>,
    /// Every cut needed to break this sheet down into `placements`, in
    /// execution order -- see `Cut`'s own docs.
    pub cuts: Vec<Cut>,
}

/// The full result of a `pack` call: every sheet used, plus any parts
/// that couldn't be placed at all -- which, with no qty-on-hand concept,
/// only ever means no stock in the catalog at this thickness was big
/// enough for this part.
#[derive(Debug, Clone)]
pub struct Layout {
    pub sheets: Vec<SheetLayout>,
    pub unplaced: Vec<PackablePart>,
}

/// One purchasing line: buy `qty` of `stock`.
#[derive(Debug, Clone)]
pub struct BomLine {
    pub stock: StockSheet,
    pub qty: usize,
}

/// (x, y, length, width) in a strip's local frame.
type FreeRect = (f64, f64, f64, f64);

/// One rip-defined strip: it always spans the sheet's full length_mm --
/// that's the rip, made once, when the strip is created -- and its
/// height (`width_mm`, fixed at creation by whichever part is first
/// placed into it) is never exceeded. That's the *only* constraint
/// inherited from being carved out of a full, unwieldy sheet.
///
/// Everything else about what happens *inside* a strip is a normal,
/// unconstrained 2D guillotine packing problem (`free_rects`, split
/// freely in either orientation): once a strip is isolated by its rip,
/// it's already a small, manageable piece, not the original sheet, so
/// there's no safety reason left to restrict cut order within it. This is
/// what lets a short part reuse the leftover height above another part in
/// the same strip instead of that space just going to waste.
#[derive(Debug, Clone)]
struct Strip {
    #[allow(dead_code)] // kept for parity/debuggability; only free_rects/placements drive packing
    y_mm: f64,
    #[allow(dead_code)] // kept for parity/debuggability; strip height lives in free_rects too
    width_mm: f64,
    free_rects: Vec<FreeRect>,
    placements: Vec<Placement>,
}

#[derive(Debug, Clone)]
struct SheetInProgress {
    #[allow(dead_code)] // assigned at construction, superseded by pack()'s own indexing on emit
    sheet_index: usize,
    used_width_mm: f64,
    strips: Vec<Strip>,
    /// Every cut made on this sheet so far, across all its strips, in the
    /// order `place_on_candidate` made them -- see `Cut`'s own docs for
    /// why call order alone is already a valid execution order.
    cuts: Vec<Cut>,
}

/// Index of the smallest-area free rect (within one strip) that fits
/// (length, width) without rotation, or None.
fn best_fit_free_rect(free_rects: &[FreeRect], length: f64, width: f64) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for (i, &(_x, _y, fw, fh)) in free_rects.iter().enumerate() {
        if length <= fw && width <= fh {
            let area = fw * fh;
            if best.is_none_or(|(a, _)| area < a) {
                best = Some((area, i));
            }
        }
    }
    best.map(|(_, i)| i)
}

/// Place a (length, width) piece into free_rects[index]'s own corner,
/// replacing it with up to two leftover rects via a guillotine split
/// (shorter-leftover-axis rule: whichever leftover side is smaller stays
/// attached to the placed piece's row/column, the larger one becomes its
/// own free rect). Safe in either split orientation -- this free rect
/// already belongs to an isolated strip, never the original full sheet.
///
/// Appends the 0-2 cuts this split actually requires to `cuts`, in the
/// order they must be made: first the cut that separates the placed
/// piece's whole row/column (piece + its attached leftover) from the
/// other, standalone leftover -- that one spans the free rect's full
/// original extent, since nothing has been cut from it yet -- then, only
/// if there is an attached leftover at all, the second cut that splits
/// the placed piece off it. A piece that exactly fills its free rect
/// needs neither.
///
/// Returns the piece's placement origin (x, y).
fn split_free_rect(
    free_rects: &mut Vec<FreeRect>,
    index: usize,
    length: f64,
    width: f64,
    cuts: &mut Vec<Cut>,
) -> (f64, f64) {
    let (fx, fy, fw, fh) = free_rects.remove(index);
    let right_w = fw - length;
    let top_h = fh - width;
    if right_w <= top_h {
        // Row (piece + right leftover) vs. the standalone top leftover.
        if top_h > 1e-6 {
            cuts.push(Cut::rip(fy + width, fx, fx + fw));
        }
        if right_w > 1e-6 {
            cuts.push(Cut::crosscut(fx + length, fy, fy + width));
            free_rects.push((fx + length, fy, right_w, width));
        }
        if top_h > 1e-6 {
            free_rects.push((fx, fy + width, fw, top_h));
        }
    } else {
        // Column (piece + top leftover) vs. the standalone right leftover.
        if right_w > 1e-6 {
            cuts.push(Cut::crosscut(fx + length, fy, fy + fh));
        }
        if top_h > 1e-6 {
            cuts.push(Cut::rip(fy + width, fx, fx + length));
            free_rects.push((fx, fy + width, length, top_h));
        }
        if right_w > 1e-6 {
            free_rects.push((fx + length, fy, right_w, fh));
        }
    }
    (fx, fy)
}

/// Pack `pieces` onto as many copies of `candidate` as needed. Parts are
/// tried width-descending (classic strip/FFDH heuristic: the widest parts
/// define strip heights first, narrower parts fill in behind them). For
/// each part: best-fit into whatever open strip has the tightest-fitting
/// free space -- any height, no matter how much taller than the part
/// itself, so long as it's tall enough -- else open a new strip on any
/// open sheet with enough remaining width, else open a new sheet.
///
/// Deliberately no preference for keeping same-size parts in one strip
/// over reusing a taller foreign one: ganging a part into an existing,
/// taller strip costs nothing extra to cut (it's one rip either way,
/// already made), while forcing it to wait for a strip of its own size
/// risks an entire additional sheet, or stranding a smaller remainder
/// sheet nothing else fits on -- far more expensive than a few inches of
/// wasted rip height. A same-size group can still end up split across
/// two strips this way when the pieces before it happened to leave
/// room for only some of it; if that split lands on an already-open
/// sheet (the common case -- best-fit tries every existing strip before
/// ever opening a new sheet), it costs nothing, just two labeled
/// rectangles instead of one contiguous block on the cut diagram.
///
/// Returns the in-progress sheets touched and the ids of pieces that got
/// placed -- the caller carries over whatever's left to the next
/// candidate stock size.
fn place_on_candidate(
    pieces: &[(usize, PackablePart)],
    candidate: &StockSheet,
    allowance_mm: f64,
) -> (Vec<SheetInProgress>, Vec<usize>) {
    let mut ordered: Vec<&(usize, PackablePart)> = pieces.iter().collect();
    ordered.sort_by(|a, b| b.1.width_mm.partial_cmp(&a.1.width_mm).unwrap());

    let mut sheets: Vec<SheetInProgress> = Vec::new();
    let mut placed_ids: Vec<usize> = Vec::new();

    for &(piece_id, ref part) in ordered {
        let (part_length, part_width) = (part.length_mm, part.width_mm);
        let length = part_length + allowance_mm;
        let width = part_width + allowance_mm;
        if length > candidate.length_mm || width > candidate.width_mm {
            continue; // too big for this stock size at all, regardless of sheet count
        }

        let mut found: Option<(usize, usize, usize)> = None; // (sheet_idx, strip_idx, free_rect_idx)
        'outer: for (si, sheet) in sheets.iter().enumerate() {
            for (ti, strip) in sheet.strips.iter().enumerate() {
                if let Some(fi) = best_fit_free_rect(&strip.free_rects, length, width) {
                    found = Some((si, ti, fi));
                    break 'outer;
                }
            }
        }

        let (si, ti, fi) = match found {
            Some(t) => t,
            None => {
                let si = match sheets
                    .iter()
                    .position(|s| s.used_width_mm + width <= candidate.width_mm)
                {
                    Some(i) => i,
                    None => {
                        sheets.push(SheetInProgress {
                            sheet_index: sheets.len(),
                            used_width_mm: 0.0,
                            strips: Vec::new(),
                            cuts: Vec::new(),
                        });
                        sheets.len() - 1
                    }
                };
                let y_mm = sheets[si].used_width_mm;
                let strip_top = y_mm + width;
                // The rip that frees this new strip from whatever sheet
                // remains above it -- skipped when the strip reaches the
                // sheet's own top edge, since there's no remainder left
                // to separate.
                if strip_top < candidate.width_mm - 1e-6 {
                    sheets[si]
                        .cuts
                        .push(Cut::rip(strip_top, 0.0, candidate.length_mm));
                }
                sheets[si].strips.push(Strip {
                    y_mm,
                    width_mm: width,
                    free_rects: vec![(0.0, y_mm, candidate.length_mm, width)],
                    placements: Vec::new(),
                });
                sheets[si].used_width_mm += width;
                (si, sheets[si].strips.len() - 1, 0)
            }
        };

        let sheet = &mut sheets[si];
        let (x, y) = split_free_rect(
            &mut sheet.strips[ti].free_rects,
            fi,
            length,
            width,
            &mut sheet.cuts,
        );
        sheet.strips[ti].placements.push(Placement {
            part_label: part.label.clone(),
            x_mm: x,
            y_mm: y,
            length_mm: part_length,
            width_mm: part_width,
            rotated: false,
        });
        placed_ids.push(piece_id);
    }

    (sheets, placed_ids)
}

#[derive(PartialEq, Eq, Hash, Clone)]
enum Bucket {
    Material(String),
    Thickness(u64), // f64 bits
}

/// Nest parts onto stock sheets via rip-first guillotine packing.
///
/// Bucketed by (material_name, thickness) when a part pins a specific
/// Material by name, else by thickness_mm alone (any material at that
/// thickness is fair game) -- see `PackablePart::material_name`. Each
/// part's packed footprint is inflated by `trim_allowance_mm + kerf_mm`
/// in both dimensions before packing -- kerf reserves the blade's own
/// width between two adjacent cuts, and trim_allowance (if nonzero)
/// reserves extra rough-cut margin on top of that for a later, separate
/// finishing pass (see `crate::diagrams`). Reported Placement sizes are
/// always the true final dimensions, never the inflated packing
/// footprint.
pub fn pack(
    parts: &[PackablePart],
    stock: &[StockSheet],
    kerf_mm: f64,
    trim_allowance_mm: f64,
) -> Layout {
    let allowance_mm = kerf_mm + trim_allowance_mm;

    let mut pieces_by_bucket: HashMap<Bucket, Vec<(usize, PackablePart)>> = HashMap::new();
    let mut order: Vec<Bucket> = Vec::new();
    let mut next_id = 0usize;
    for part in parts {
        let bucket = match &part.material_name {
            Some(name) if !name.is_empty() => Bucket::Material(name.clone()),
            _ => Bucket::Thickness(part.thickness_mm.to_bits()),
        };
        if !pieces_by_bucket.contains_key(&bucket) {
            pieces_by_bucket.insert(bucket.clone(), Vec::new());
            order.push(bucket.clone());
        }
        let entry = pieces_by_bucket.get_mut(&bucket).unwrap();
        for _ in 0..part.qty {
            entry.push((next_id, part.clone()));
            next_id += 1;
        }
    }

    let mut sheets: Vec<SheetLayout> = Vec::new();
    let mut unplaced: Vec<PackablePart> = Vec::new();
    let mut next_sheet_index: HashMap<StockSheet, usize> = HashMap::new();

    for bucket in &order {
        let pieces = &pieces_by_bucket[bucket];
        let mut candidates: Vec<&StockSheet> = match bucket {
            Bucket::Material(name) => stock.iter().filter(|s| &s.material.name == name).collect(),
            Bucket::Thickness(bits) => stock
                .iter()
                .filter(|s| s.thickness_mm().to_bits() == *bits)
                .collect(),
        };
        candidates.sort_by(|a, b| {
            (b.length_mm * b.width_mm)
                .partial_cmp(&(a.length_mm * a.width_mm))
                .unwrap()
        });

        let mut remaining: Vec<(usize, PackablePart)> = pieces.clone();
        for &candidate in &candidates {
            if remaining.is_empty() {
                break;
            }
            let (in_progress, placed_ids) = place_on_candidate(&remaining, candidate, allowance_mm);
            for sheet in in_progress {
                let placements: Vec<Placement> = sheet
                    .strips
                    .into_iter()
                    .flat_map(|s| s.placements)
                    .collect();
                if placements.is_empty() {
                    continue;
                }
                let idx = *next_sheet_index.get(candidate).unwrap_or(&0);
                next_sheet_index.insert(candidate.clone(), idx + 1);
                sheets.push(SheetLayout {
                    stock: candidate.clone(),
                    sheet_index: idx,
                    placements,
                    cuts: sheet.cuts,
                });
            }

            let placed_set: HashSet<usize> = placed_ids.into_iter().collect();
            remaining.retain(|(id, _)| !placed_set.contains(id));
        }

        unplaced.extend(remaining.into_iter().map(|(_, part)| part));
    }

    Layout { sheets, unplaced }
}

/// Roll a set of sheets up into purchasing lines: how many of each
/// StockSheet got used. Purely a count of `sheets` grouped by stock --
/// there's no on-hand quantity to net out, since StockSheet is a catalog
/// entry, not an inventory count (see `StockSheet`'s docs). Sorted by
/// material name then thickness for a stable, readable BOM.
///
/// Takes any borrowed-`SheetLayout` iterator rather than a whole
/// `&Layout`, so the exact same function produces both a whole-project
/// BOM (`bill_of_materials(&layout.sheets)`) and a per-section BOM
/// (`bill_of_materials(section_sheets)`, `section_sheets: Vec<&SheetLayout>`
/// from `crate::diagrams::group_sheets_by_section`) with no duplicated
/// counting logic -- see `crate::diagrams::render_pdf`.
pub fn bill_of_materials<'a>(sheets: impl IntoIterator<Item = &'a SheetLayout>) -> Vec<BomLine> {
    let mut counts: HashMap<StockSheet, usize> = HashMap::new();
    let mut order: Vec<StockSheet> = Vec::new();
    for sheet in sheets {
        if !counts.contains_key(&sheet.stock) {
            counts.insert(sheet.stock.clone(), 0);
            order.push(sheet.stock.clone());
        }
        *counts.get_mut(&sheet.stock).unwrap() += 1;
    }
    let mut lines: Vec<BomLine> = order
        .into_iter()
        .map(|stock| {
            let qty = counts[&stock];
            BomLine { stock, qty }
        })
        .collect();
    lines.sort_by(|a, b| {
        a.stock.material.name.cmp(&b.stock.material.name).then(
            a.stock
                .thickness_mm()
                .partial_cmp(&b.stock.thickness_mm())
                .unwrap(),
        )
    });
    lines
}

/// Where a `CutStep`'s cut lands -- either the sheet itself (its very
/// first cut), or a piece an earlier step in the same breakdown left for
/// later (see `Outcome::Piece`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Sheet,
    Piece(usize),
}

/// What one side of a `CutStep`'s cut turns out to be. `Piece` ids are
/// only meaningful within one call's own result -- they number the
/// pieces that still need a further cut, in the order this breakdown
/// creates them, and every one is guaranteed to show up as some later
/// step's own `Source::Piece` (see `cut_steps`'s docs): nothing here is
/// numbered "to be cut later" and then never is.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Already a finished part -- nothing more to cut. Carries the same
    /// label as the matching `Placement.part_label`.
    Part(String),
    /// Needs at least one more cut; some later step cuts it as
    /// `Source::Piece(id)`.
    Piece(usize),
    /// Never cut again and never matches a placement -- scrap left over
    /// once every part is out.
    Offcut,
}

/// One cut, described the way it actually has to be executed: as a
/// distance from the edge of the specific piece being cut, not from the
/// sheet's own reference corner (see `Cut`'s own docs for why the two
/// aren't the same thing past the first cut).
#[derive(Debug, Clone, PartialEq)]
pub struct CutStep {
    pub cut: Cut,
    /// `cut.position_mm`'s distance from `source`'s own edge on the
    /// cut's axis (its y_mm edge for a `Rip`, x_mm edge for a
    /// `Crosscut`) -- always the edge every piece in this lineage traces
    /// back toward the sheet's reference corner through, whether that's
    /// the sheet's own true corner (the first cut made on a piece) or a
    /// fresh edge an earlier cut in the same lineage left behind. Either
    /// way, it's a real edge physically present on the piece in hand,
    /// and consistently the same side -- so which edge to measure from
    /// never depends on how deep into the breakdown a step is.
    pub offset_mm: f64,
    pub source: Source,
    /// The two pieces this cut produces: `near` is the side on
    /// `source`'s reference-corner-ward edge (what `offset_mm` cuts
    /// off), `far` is everything past it.
    pub near: Outcome,
    pub far: Outcome,
}

/// Replays `sheet.cuts` -- which only records *where* each cut falls in
/// the sheet's own fixed coordinates (see `Cut`'s docs) -- into the
/// sequence a woodworker can actually follow: for each cut, which piece
/// it lands on, how far from that piece's own edge, and what the cut
/// leaves behind.
///
/// A rect this replay produces is classified in a *second* pass, once
/// every cut has been replayed -- never inline, while replaying, even
/// though a piece's `Outcome` looks decidable the moment it's created.
/// The reason: a piece's corner (x0, y0) always exactly equals a
/// placement's own (`Placement.x_mm`/`y_mm`), since a part is always
/// placed flush with its free rect's own corner -- but that's just as
/// true of a piece several cuts away from being that placement (its far
/// edge is still whatever the *sheet* or an earlier strip left it,
/// nowhere near the placement's true size) as it is of the one cut that
/// actually finishes the job. Only "did any later cut ever need to
/// divide this piece further" tells the two apart -- a leaf that never
/// gets cut again either matches a placement's corner (a finished part)
/// or it doesn't (scrap) -- and that's only knowable after the whole
/// sheet's cuts have been replayed.
pub fn cut_steps(sheet: &SheetLayout) -> Vec<CutStep> {
    #[derive(Clone, Copy)]
    struct Rect {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    }

    struct OpenPiece {
        id: Option<usize>, // None only for the sheet itself
        rect: Rect,
    }

    /// One cut, replayed geometrically but not yet classified -- `near`/
    /// `far` are just the piece ids `cut_steps` assigned, before knowing
    /// whether each is a finished part, a piece needing more cuts, or
    /// scrap.
    struct RawStep {
        cut: Cut,
        offset_mm: f64,
        source_id: Option<usize>,
        near_id: usize,
        far_id: usize,
    }

    const EPS: f64 = 1e-6;

    let mut open = vec![OpenPiece {
        id: None,
        rect: Rect {
            x0: 0.0,
            y0: 0.0,
            x1: sheet.stock.length_mm,
            y1: sheet.stock.width_mm,
        },
    }];
    let mut rects: HashMap<usize, Rect> = HashMap::new();
    let mut cut_further: HashSet<usize> = HashSet::new();
    let mut next_id = 1usize;
    let mut raw_steps: Vec<RawStep> = Vec::with_capacity(sheet.cuts.len());

    for cut in &sheet.cuts {
        let idx = open
            .iter()
            .position(|p| match cut.kind {
                CutKind::Rip => {
                    cut.position_mm > p.rect.y0 + EPS
                        && cut.position_mm < p.rect.y1 - EPS
                        && (cut.span_start_mm - p.rect.x0).abs() < EPS
                        && (cut.span_end_mm - p.rect.x1).abs() < EPS
                }
                CutKind::Crosscut => {
                    cut.position_mm > p.rect.x0 + EPS
                        && cut.position_mm < p.rect.x1 - EPS
                        && (cut.span_start_mm - p.rect.y0).abs() < EPS
                        && (cut.span_end_mm - p.rect.y1).abs() < EPS
                }
            })
            .expect("a recorded cut should always divide a piece this breakdown already produced");
        let piece = open.remove(idx);
        if let Some(id) = piece.id {
            cut_further.insert(id);
        }

        let (offset_mm, near_rect, far_rect) = match cut.kind {
            CutKind::Rip => (
                cut.position_mm - piece.rect.y0,
                Rect {
                    y1: cut.position_mm,
                    ..piece.rect
                },
                Rect {
                    y0: cut.position_mm,
                    ..piece.rect
                },
            ),
            CutKind::Crosscut => (
                cut.position_mm - piece.rect.x0,
                Rect {
                    x1: cut.position_mm,
                    ..piece.rect
                },
                Rect {
                    x0: cut.position_mm,
                    ..piece.rect
                },
            ),
        };

        let mut alloc = |rect: Rect| -> usize {
            let id = next_id;
            next_id += 1;
            rects.insert(id, rect);
            open.push(OpenPiece { id: Some(id), rect });
            id
        };
        let near_id = alloc(near_rect);
        let far_id = alloc(far_rect);

        raw_steps.push(RawStep {
            cut: *cut,
            offset_mm,
            source_id: piece.id,
            near_id,
            far_id,
        });
    }

    // Now that every cut has been replayed, `cut_further` says which
    // ids were ever a later cut's source -- the only thing that tells a
    // finished part apart from a piece merely sharing its corner (see
    // this function's own docs).
    let classify = |id: usize| -> Outcome {
        if cut_further.contains(&id) {
            return Outcome::Piece(id);
        }
        let rect = rects[&id];
        match sheet
            .placements
            .iter()
            .find(|p| (p.x_mm - rect.x0).abs() < EPS && (p.y_mm - rect.y0).abs() < EPS)
        {
            Some(p) => Outcome::Part(p.part_label.clone()),
            None => Outcome::Offcut,
        }
    };

    raw_steps
        .into_iter()
        .map(|r| CutStep {
            cut: r.cut,
            offset_mm: r.offset_mm,
            source: r.source_id.map_or(Source::Sheet, Source::Piece),
            near: classify(r.near_id),
            far: classify(r.far_id),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three_quarter() -> Material {
        Material {
            name: "3/4 Baltic Birch".to_string(),
            thickness_mm: 19.05,
        }
    }
    fn quarter() -> Material {
        Material {
            name: "1/4 Baltic Birch".to_string(),
            thickness_mm: 6.35,
        }
    }
    fn stock() -> Vec<StockSheet> {
        vec![
            StockSheet {
                material: three_quarter(),
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
            StockSheet {
                material: quarter(),
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
        ]
    }

    #[test]
    fn pack_never_rotates_a_part_to_rescue_a_fit() {
        // As given (width_mm=1300), this doesn't fit the sheet's 1219.2mm
        // width -- even though swapping length_mm/width_mm would fit it
        // fine (1300 <= the sheet's 2438.4mm length, 800 <= its 1219.2mm
        // width). `pack` must report it unplaced rather than silently
        // rotating it to make it fit: deciding which of a part's
        // dimensions runs with the sheet's grain is the caller's job (see
        // `PackablePart`'s docs), never something `pack` chooses on its
        // own.
        let sheet = &stock()[0];
        let rotated_would_fit = PackablePart::new("panel", 1300.0, 800.0, 19.05);
        assert!(
            pack(&[rotated_would_fit], &stock(), DEFAULT_KERF_MM, 0.0)
                .unplaced
                .is_empty(),
            "sanity: the swapped footprint does fit"
        );
        assert!(
            1300.0 > sheet.width_mm,
            "test assumption: as-given width alone shouldn't fit"
        );

        let as_given = PackablePart::new("panel", 800.0, 1300.0, 19.05);
        let layout = pack(&[as_given], &stock(), DEFAULT_KERF_MM, 0.0);
        assert_eq!(layout.unplaced.len(), 1);
        assert!(layout.sheets.is_empty());
    }

    #[test]
    fn pack_buckets_by_thickness_not_just_size() {
        let parts = vec![
            PackablePart {
                qty: 2,
                ..PackablePart::new("panel", 765.175, 406.4, 19.05)
            },
            PackablePart::new("backer", 787.4, 431.8, 6.35),
        ];
        let layout = pack(&parts, &stock(), DEFAULT_KERF_MM, 0.0);

        assert!(layout.unplaced.is_empty());
        let thicknesses: HashSet<u64> = layout
            .sheets
            .iter()
            .map(|s| s.stock.thickness_mm().to_bits())
            .collect();
        assert_eq!(
            thicknesses,
            [19.05f64.to_bits(), 6.35f64.to_bits()]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn pack_reports_unplaced_when_no_matching_stock() {
        let orphan = PackablePart::new("mystery", 500.0, 300.0, 12.7);
        let layout = pack(
            std::slice::from_ref(&orphan),
            &stock(),
            DEFAULT_KERF_MM,
            0.0,
        );

        assert_eq!(layout.unplaced, vec![orphan]);
        assert!(layout.sheets.is_empty());
    }

    #[test]
    fn bill_of_materials_counts_sheets_per_stock_item() {
        let parts = vec![
            PackablePart {
                qty: 2,
                ..PackablePart::new("panel", 765.175, 406.4, 19.05)
            },
            PackablePart::new("backer", 787.4, 431.8, 6.35),
        ];
        let layout = pack(&parts, &stock(), DEFAULT_KERF_MM, 0.0);
        let bom = bill_of_materials(&layout.sheets);

        let by_name: HashMap<&str, usize> = bom
            .iter()
            .map(|l| (l.stock.material.name.as_str(), l.qty))
            .collect();
        assert_eq!(
            by_name["3/4 Baltic Birch"], 1,
            "both 30x16in pieces fit one 96x48in sheet with real packing"
        );
        assert_eq!(by_name["1/4 Baltic Birch"], 1);
        assert_eq!(
            bom.iter().map(|l| l.qty).sum::<usize>(),
            layout.sheets.len()
        );
    }

    #[test]
    fn pack_places_multiple_parts_per_sheet_without_overlap() {
        let parts: Vec<PackablePart> = (0..4)
            .map(|i| PackablePart::new(format!("panel-{i}"), 762.0, 406.4, 19.05))
            .collect();
        let layout = pack(&parts, &stock(), DEFAULT_KERF_MM, 0.0);

        assert!(layout.unplaced.is_empty());
        assert_eq!(
            layout.sheets.len(),
            1,
            "four 30x16in panels should nest onto a single 96x48in sheet"
        );
        let placements = &layout.sheets[0].placements;
        assert_eq!(placements.len(), 4);
        for (i, a) in placements.iter().enumerate() {
            for b in &placements[i + 1..] {
                let x_overlap = a.x_mm < b.x_mm + b.length_mm && b.x_mm < a.x_mm + a.length_mm;
                let y_overlap = a.y_mm < b.y_mm + b.width_mm && b.y_mm < a.y_mm + a.width_mm;
                assert!(
                    !(x_overlap && y_overlap),
                    "placements overlap: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn material_name_pins_a_part_even_at_shared_thickness() {
        let finished = Material {
            name: "Baltic Birch 3/4 (finished)".to_string(),
            thickness_mm: 19.05,
        };
        let utility = Material {
            name: "Sande Ply 3/4 (utility)".to_string(),
            thickness_mm: 19.05,
        };
        let stock = vec![
            StockSheet {
                material: finished.clone(),
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
            StockSheet {
                material: utility.clone(),
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
        ];
        let panel = PackablePart {
            material_name: Some(finished.name.clone()),
            ..PackablePart::new("show-face panel", 700.0, 400.0, 19.05)
        };
        let stretcher = PackablePart {
            material_name: Some(utility.name.clone()),
            ..PackablePart::new("hidden stretcher", 700.0, 100.0, 19.05)
        };

        let layout = pack(&[panel, stretcher], &stock, DEFAULT_KERF_MM, 0.0);

        assert!(layout.unplaced.is_empty());
        let materials_used: HashSet<&str> = layout
            .sheets
            .iter()
            .map(|s| s.stock.material.name.as_str())
            .collect();
        assert_eq!(
            materials_used,
            [finished.name.as_str(), utility.name.as_str()]
                .into_iter()
                .collect()
        );
        for sheet in &layout.sheets {
            let labels: HashSet<&str> = sheet
                .placements
                .iter()
                .map(|p| p.part_label.as_str())
                .collect();
            if sheet.stock.material.name == finished.name {
                assert_eq!(labels, ["show-face panel"].into_iter().collect());
            } else {
                assert_eq!(labels, ["hidden stretcher"].into_iter().collect());
            }
        }
    }

    #[test]
    fn unassigned_part_still_matches_by_thickness_alone() {
        let part = PackablePart::new("whatever", 700.0, 400.0, 19.05);
        let layout = pack(&[part], &stock(), DEFAULT_KERF_MM, 0.0);

        assert!(layout.unplaced.is_empty());
        assert_eq!(layout.sheets[0].stock.material.name, "3/4 Baltic Birch");
    }

    #[test]
    fn new_strip_spans_the_sheets_full_length() {
        let part = (0usize, PackablePart::new("wide-a", 1200.0, 600.0, 19.05));
        let (sheets, placed_ids) = place_on_candidate(&[part], &stock()[0], 0.0);

        assert_eq!(placed_ids, vec![0]);
        let strip = &sheets[0].strips[0];
        assert_eq!(strip.free_rects.len(), 1);
        let (fx, _fy, fw, fh) = strip.free_rects[0];
        assert_eq!(
            fx + fw,
            stock()[0].length_mm,
            "leftover after one placement should reach exactly the sheet's full length"
        );
        assert_eq!(fh, 600.0, "leftover should keep the strip's full height available, not just the placed part's row");
    }

    #[test]
    fn short_part_reuses_leftover_height_within_a_strip() {
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 1000.0,
            width_mm: 500.0,
        };
        let parts = vec![
            (0usize, PackablePart::new("tall", 200.0, 300.0, 19.0)),
            (1usize, PackablePart::new("medium", 300.0, 120.0, 19.0)),
            (2usize, PackablePart::new("short", 250.0, 90.0, 19.0)),
        ];

        let (sheets, placed_ids) = place_on_candidate(&parts, &test_stock, 0.0);

        assert_eq!(placed_ids.len(), 3);
        assert_eq!(sheets.len(), 1);
        assert_eq!(
            sheets[0].strips.len(),
            1,
            "medium and short should both reuse tall's strip, not open new ones"
        );
        let by_label: HashMap<&str, &Placement> = sheets[0].strips[0]
            .placements
            .iter()
            .map(|p| (p.part_label.as_str(), p))
            .collect();
        assert_eq!(by_label["medium"].y_mm, 0.0);
        assert_eq!(
            by_label["short"].y_mm, by_label["medium"].width_mm,
            "short should stack directly above medium"
        );
    }

    #[test]
    fn a_same_size_group_may_split_across_strips_but_never_strands_an_avoidable_sheet() {
        // Scaled from real project data: two ~290mm-wide parts fill most
        // of a strip on sheet 1; a third, slightly narrower part
        // (288.75mm) can't join them, so it opens sheet 2. Three
        // 170mm-wide parts come next -- narrow enough that some of them
        // fit into the leftover pockets those first two strips left
        // behind, on sheets that already exist. A same-size group
        // splitting across strips like this costs nothing (no rip goes
        // uncut, no sheet goes unused) and is preferable to forcing every
        // member to wait for a dedicated strip of its own height, which
        // can cost a whole extra sheet -- so this only asserts none of
        // that: every part placed, and no more sheets used than pieces
        // that can't share space at all actually require.
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 960.0,
            width_mm: 480.0,
        };
        let parts = vec![
            (0usize, PackablePart::new("wide-a", 331.25, 290.0, 19.0)),
            (1usize, PackablePart::new("wide-b", 331.25, 290.0, 19.0)),
            (2usize, PackablePart::new("mid-a", 290.0, 288.75, 19.0)),
            (3usize, PackablePart::new("mid-b", 290.0, 288.75, 19.0)),
            (4usize, PackablePart::new("short-a", 310.0, 170.0, 19.0)),
            (5usize, PackablePart::new("short-b", 310.0, 170.0, 19.0)),
            (6usize, PackablePart::new("short-c", 310.0, 170.0, 19.0)),
        ];

        let (sheets, placed_ids) = place_on_candidate(&parts, &test_stock, 0.0);

        assert_eq!(placed_ids.len(), 7);
        let short_placements: usize = sheets
            .iter()
            .flat_map(|s| &s.strips)
            .flat_map(|strip| &strip.placements)
            .filter(|p| p.part_label.starts_with("short-"))
            .count();
        assert_eq!(short_placements, 3);
        // 2 sheets already exist (wide's and mid's) by the time the short
        // group is placed; all three short parts fit into leftover space
        // on those two, so no third sheet should ever open.
        assert_eq!(
            sheets.len(),
            2,
            "the short group should reuse the two sheets already open, not strand a third"
        );
    }

    #[test]
    fn opening_a_strip_records_the_rip_that_frees_it_from_whatever_sheet_remains_above() {
        // a's leftover (700..1000 long, 200 tall) is too short for b's
        // 400mm length, so b can't reuse it and opens its own strip --
        // each strip's own opening then records the rip that frees it
        // from the sheet remaining above it at the time.
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 1000.0,
            width_mm: 500.0,
        };
        let parts = vec![
            (0usize, PackablePart::new("a", 700.0, 200.0, 19.0)),
            (1usize, PackablePart::new("b", 400.0, 150.0, 19.0)),
        ];

        let (sheets, placed_ids) = place_on_candidate(&parts, &test_stock, 0.0);

        assert_eq!(placed_ids.len(), 2);
        assert_eq!(
            sheets[0].strips.len(),
            2,
            "b can't reuse a's leftover, so it opens its own strip"
        );
        assert_eq!(
            sheets[0].cuts,
            vec![
                Cut::rip(200.0, 0.0, 1000.0),
                Cut::crosscut(700.0, 0.0, 200.0),
                Cut::rip(350.0, 0.0, 1000.0),
                Cut::crosscut(400.0, 200.0, 350.0),
            ],
            "each strip's opening rip is followed by the crosscut that trims its part off the strip's own leftover"
        );
    }

    #[test]
    fn a_strip_reaching_the_sheets_top_edge_needs_no_closing_rip() {
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 1000.0,
            width_mm: 500.0,
        };
        let part = (0usize, PackablePart::new("full-sheet", 1000.0, 500.0, 19.0));

        let (sheets, _) = place_on_candidate(&[part], &test_stock, 0.0);

        assert!(
            sheets[0].cuts.is_empty(),
            "a part exactly matching the sheet needs neither a strip-opening rip nor a split cut"
        );
    }

    #[test]
    fn a_part_exactly_filling_its_free_rect_needs_no_cut() {
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 400.0,
            width_mm: 200.0,
        };
        let part = (0usize, PackablePart::new("exact", 400.0, 200.0, 19.0));

        let (sheets, _) = place_on_candidate(&[part], &test_stock, 0.0);

        assert!(
            sheets[0].cuts.is_empty(),
            "a part matching the whole sheet needs no cut to isolate it"
        );
    }

    /// Replays `sheet.cuts` against a simulated full sheet -- starting
    /// from one board the size of the whole sheet, each cut must divide
    /// some board already on the table into two, in guillotine fashion
    /// (edge to edge across whichever board it lands in) -- then checks
    /// that every placement ends up as its own standalone board. This is
    /// the real correctness property `cuts` needs: not just "the right
    /// positions," but "actually executable, in this order, and
    /// sufficient to free every part." Callers must pack with zero
    /// kerf/trim allowance so a placement's true dims match its isolated
    /// board's dims exactly.
    fn assert_cuts_isolate_every_placement(sheet: &SheetLayout) {
        #[derive(Clone, Copy, Debug)]
        struct Board {
            x0: f64,
            y0: f64,
            x1: f64,
            y1: f64,
        }
        const EPS: f64 = 1e-6;

        let mut boards = vec![Board {
            x0: 0.0,
            y0: 0.0,
            x1: sheet.stock.length_mm,
            y1: sheet.stock.width_mm,
        }];
        for cut in &sheet.cuts {
            let idx = boards
                .iter()
                .position(|b| match cut.kind {
                    CutKind::Rip => {
                        cut.position_mm > b.y0 + EPS
                            && cut.position_mm < b.y1 - EPS
                            && (cut.span_start_mm - b.x0).abs() < EPS
                            && (cut.span_end_mm - b.x1).abs() < EPS
                    }
                    CutKind::Crosscut => {
                        cut.position_mm > b.x0 + EPS
                            && cut.position_mm < b.x1 - EPS
                            && (cut.span_start_mm - b.y0).abs() < EPS
                            && (cut.span_end_mm - b.y1).abs() < EPS
                    }
                })
                .unwrap_or_else(|| {
                    panic!("{cut:?} doesn't divide any board on the table: {boards:?}")
                });
            let b = boards.remove(idx);
            match cut.kind {
                CutKind::Rip => {
                    boards.push(Board {
                        y1: cut.position_mm,
                        ..b
                    });
                    boards.push(Board {
                        y0: cut.position_mm,
                        ..b
                    });
                }
                CutKind::Crosscut => {
                    boards.push(Board {
                        x1: cut.position_mm,
                        ..b
                    });
                    boards.push(Board {
                        x0: cut.position_mm,
                        ..b
                    });
                }
            }
        }

        for p in &sheet.placements {
            let isolated = boards.iter().any(|b| {
                (b.x0 - p.x_mm).abs() < EPS
                    && (b.y0 - p.y_mm).abs() < EPS
                    && (b.x1 - (p.x_mm + p.length_mm)).abs() < EPS
                    && (b.y1 - (p.y_mm + p.width_mm)).abs() < EPS
            });
            assert!(
                isolated,
                "placement {p:?} was never isolated as its own board by replaying cuts in order"
            );
        }
    }

    #[test]
    fn pack_produces_a_cut_sequence_that_isolates_every_placement() {
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 960.0,
            width_mm: 480.0,
        };
        let parts = vec![
            PackablePart::new("wide-a", 331.25, 290.0, 19.0),
            PackablePart::new("wide-b", 331.25, 290.0, 19.0),
            PackablePart::new("mid-a", 290.0, 288.75, 19.0),
            PackablePart::new("mid-b", 290.0, 288.75, 19.0),
            PackablePart::new("short-a", 310.0, 170.0, 19.0),
            PackablePart::new("short-b", 310.0, 170.0, 19.0),
            PackablePart::new("short-c", 310.0, 170.0, 19.0),
        ];

        let layout = pack(&parts, &[test_stock], 0.0, 0.0);

        assert!(layout.unplaced.is_empty());
        for sheet in &layout.sheets {
            assert_cuts_isolate_every_placement(sheet);
        }
    }

    #[test]
    fn cut_steps_describes_each_cut_relative_to_the_piece_it_lands_on() {
        // Same scenario as
        // opening_a_strip_records_the_rip_that_frees_it_from_whatever_sheet_remains_above:
        // a's leftover is too short for b, so b opens its own strip.
        // Once b's strip is freed, the *sheet's* reference corner is
        // gone from it -- its own rip, and a's own crosscut, must be
        // described relative to the piece each one actually lands on.
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 1000.0,
            width_mm: 500.0,
        };
        let parts = vec![
            PackablePart::new("a", 700.0, 200.0, 19.0),
            PackablePart::new("b", 400.0, 150.0, 19.0),
        ];
        let layout = pack(&parts, &[test_stock], 0.0, 0.0);
        assert!(layout.unplaced.is_empty());
        let sheet = &layout.sheets[0];

        let steps = cut_steps(sheet);
        assert_eq!(steps.len(), 4);

        assert_eq!(steps[0].source, Source::Sheet);
        assert_eq!(steps[0].offset_mm, 200.0);
        assert_eq!(steps[0].near, Outcome::Piece(1));
        assert_eq!(steps[0].far, Outcome::Piece(2));

        assert_eq!(
            steps[1].source,
            Source::Piece(1),
            "a's crosscut lands on the piece step 0 cut off, not the sheet"
        );
        assert_eq!(steps[1].offset_mm, 700.0);
        assert_eq!(steps[1].near, Outcome::Part("a".to_string()));
        assert_eq!(steps[1].far, Outcome::Offcut);

        assert_eq!(
            steps[2].source,
            Source::Piece(2),
            "b's strip is ripped from the piece step 0 left over, not the sheet"
        );
        assert_eq!(
            steps[2].offset_mm, 150.0,
            "150mm from that piece's own edge, not 350mm from the sheet's corner"
        );
        assert_eq!(steps[2].near, Outcome::Piece(5));
        assert_eq!(steps[2].far, Outcome::Offcut);

        assert_eq!(steps[3].source, Source::Piece(5));
        assert_eq!(
            steps[3].offset_mm, 400.0,
            "b's own length, from the edge of the piece it's actually cut from"
        );
        assert_eq!(steps[3].near, Outcome::Part("b".to_string()));
        assert_eq!(steps[3].far, Outcome::Offcut);
    }

    #[test]
    fn cut_steps_still_recognizes_a_finished_part_when_kerf_inflates_its_footprint() {
        // A placement's *true* far edge sits kerf_mm short of the free
        // rect it was actually carved from -- the gap is blade waste,
        // never drawn as part of any piece (see `Cut`'s docs). Matching
        // a leaf to a placement by full-rect equality (comparing the
        // leaf's far edge too) would never fire once kerf_mm > 0; only
        // matching by the corner they share is correct.
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 1000.0,
            width_mm: 500.0,
        };
        let parts = vec![
            PackablePart::new("a", 700.0, 200.0, 19.0),
            PackablePart::new("b", 400.0, 150.0, 19.0),
        ];
        let layout = pack(&parts, &[test_stock], 3.2, 0.0);
        assert!(layout.unplaced.is_empty());
        let sheet = &layout.sheets[0];

        let steps = cut_steps(sheet);
        let parts_seen: Vec<&str> = steps
            .iter()
            .flat_map(|s| [&s.near, &s.far])
            .filter_map(|o| match o {
                Outcome::Part(label) => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            parts_seen,
            vec!["a", "b"],
            "both parts should still be recognized as finished, despite kerf inflating every free rect's far edge"
        );
    }

    #[test]
    fn cut_steps_every_numbered_piece_is_addressed_by_a_later_step() {
        let test_stock = StockSheet {
            material: Material {
                name: "test".to_string(),
                thickness_mm: 19.0,
            },
            length_mm: 960.0,
            width_mm: 480.0,
        };
        let parts = vec![
            PackablePart::new("wide-a", 331.25, 290.0, 19.0),
            PackablePart::new("wide-b", 331.25, 290.0, 19.0),
            PackablePart::new("mid-a", 290.0, 288.75, 19.0),
            PackablePart::new("mid-b", 290.0, 288.75, 19.0),
            PackablePart::new("short-a", 310.0, 170.0, 19.0),
            PackablePart::new("short-b", 310.0, 170.0, 19.0),
            PackablePart::new("short-c", 310.0, 170.0, 19.0),
        ];
        let layout = pack(&parts, &[test_stock], 0.0, 0.0);
        assert!(layout.unplaced.is_empty());

        for sheet in &layout.sheets {
            let steps = cut_steps(sheet);
            for (i, step) in steps.iter().enumerate() {
                for outcome in [&step.near, &step.far] {
                    if let Outcome::Piece(id) = outcome {
                        let addressed_later = steps[i + 1..]
                            .iter()
                            .any(|later| later.source == Source::Piece(*id));
                        assert!(
                            addressed_later,
                            "piece {id} from step {i} is never cut again"
                        );
                    }
                }
            }
            for placement in &sheet.placements {
                let found = steps.iter().any(|step| {
                    matches!(&step.near, Outcome::Part(l) if l == &placement.part_label)
                        || matches!(&step.far, Outcome::Part(l) if l == &placement.part_label)
                });
                assert!(
                    found,
                    "{} never appears as a cut outcome",
                    placement.part_label
                );
            }
        }
    }
}
