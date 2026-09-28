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
//!     never rotating, no per-part annotation needed.
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
        Self { label: label.into(), length_mm, width_mm, thickness_mm, qty: 1, material_name: None }
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

/// One physical sheet (the `sheet_index`-th copy of `stock` used) and
/// everything placed on it.
#[derive(Debug, Clone)]
pub struct SheetLayout {
    pub stock: StockSheet,
    pub sheet_index: usize,
    pub placements: Vec<Placement>,
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
}

/// Index of the smallest-area free rect (within one strip) that fits
/// (length, width) without rotation, or None.
fn best_fit_free_rect(free_rects: &[FreeRect], length: f64, width: f64) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for (i, &(_x, _y, fw, fh)) in free_rects.iter().enumerate() {
        if length <= fw && width <= fh {
            let area = fw * fh;
            if best.map_or(true, |(a, _)| area < a) {
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
/// Returns the piece's placement origin (x, y).
fn split_free_rect(free_rects: &mut Vec<FreeRect>, index: usize, length: f64, width: f64) -> (f64, f64) {
    let (fx, fy, fw, fh) = free_rects.remove(index);
    let right_w = fw - length;
    let top_h = fh - width;
    if right_w <= top_h {
        if right_w > 1e-6 {
            free_rects.push((fx + length, fy, right_w, width));
        }
        if top_h > 1e-6 {
            free_rects.push((fx, fy + width, fw, top_h));
        }
    } else {
        if top_h > 1e-6 {
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
/// each part: best-fit into any open strip's free space; else open a new
/// strip on any open sheet with enough remaining width; else open a new
/// sheet.
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
        let length = part.length_mm + allowance_mm;
        let width = part.width_mm + allowance_mm;
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
                let si = match sheets.iter().position(|s| s.used_width_mm + width <= candidate.width_mm) {
                    Some(i) => i,
                    None => {
                        sheets.push(SheetInProgress { sheet_index: sheets.len(), used_width_mm: 0.0, strips: Vec::new() });
                        sheets.len() - 1
                    }
                };
                let y_mm = sheets[si].used_width_mm;
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

        let (x, y) = split_free_rect(&mut sheets[si].strips[ti].free_rects, fi, length, width);
        sheets[si].strips[ti].placements.push(Placement {
            part_label: part.label.clone(),
            x_mm: x,
            y_mm: y,
            length_mm: part.length_mm,
            width_mm: part.width_mm,
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
pub fn pack(parts: &[PackablePart], stock: &[StockSheet], kerf_mm: f64, trim_allowance_mm: f64) -> Layout {
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
            Bucket::Thickness(bits) => stock.iter().filter(|s| s.thickness_mm().to_bits() == *bits).collect(),
        };
        candidates.sort_by(|a, b| (b.length_mm * b.width_mm).partial_cmp(&(a.length_mm * a.width_mm)).unwrap());

        let mut remaining: Vec<(usize, PackablePart)> = pieces.clone();
        for &candidate in &candidates {
            if remaining.is_empty() {
                break;
            }
            let (in_progress, placed_ids) = place_on_candidate(&remaining, candidate, allowance_mm);
            for sheet in in_progress {
                let placements: Vec<Placement> = sheet.strips.into_iter().flat_map(|s| s.placements).collect();
                if placements.is_empty() {
                    continue;
                }
                let idx = *next_sheet_index.get(candidate).unwrap_or(&0);
                next_sheet_index.insert(candidate.clone(), idx + 1);
                sheets.push(SheetLayout { stock: candidate.clone(), sheet_index: idx, placements });
            }

            let placed_set: HashSet<usize> = placed_ids.into_iter().collect();
            remaining = remaining.into_iter().filter(|(id, _)| !placed_set.contains(id)).collect();
        }

        unplaced.extend(remaining.into_iter().map(|(_, part)| part));
    }

    Layout { sheets, unplaced }
}

/// Roll a Layout up into purchasing lines: how many of each StockSheet
/// got used. Purely a count of `layout.sheets` grouped by stock -- there's
/// no on-hand quantity to net out, since StockSheet is a catalog entry,
/// not an inventory count (see `StockSheet`'s docs). Sorted by material
/// name then thickness for a stable, readable BOM.
pub fn bill_of_materials(layout: &Layout) -> Vec<BomLine> {
    let mut counts: HashMap<StockSheet, usize> = HashMap::new();
    let mut order: Vec<StockSheet> = Vec::new();
    for sheet in &layout.sheets {
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
        a.stock
            .material
            .name
            .cmp(&b.stock.material.name)
            .then(a.stock.thickness_mm().partial_cmp(&b.stock.thickness_mm()).unwrap())
    });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three_quarter() -> Material {
        Material { name: "3/4 Baltic Birch".to_string(), thickness_mm: 19.05 }
    }
    fn quarter() -> Material {
        Material { name: "1/4 Baltic Birch".to_string(), thickness_mm: 6.35 }
    }
    fn stock() -> Vec<StockSheet> {
        vec![
            StockSheet { material: three_quarter(), length_mm: 2438.4, width_mm: 1219.2 },
            StockSheet { material: quarter(), length_mm: 2438.4, width_mm: 1219.2 },
        ]
    }

    #[test]
    fn pack_buckets_by_thickness_not_just_size() {
        let parts = vec![
            PackablePart { qty: 2, ..PackablePart::new("panel", 765.175, 406.4, 19.05) },
            PackablePart::new("backer", 787.4, 431.8, 6.35),
        ];
        let layout = pack(&parts, &stock(), DEFAULT_KERF_MM, 0.0);

        assert!(layout.unplaced.is_empty());
        let thicknesses: HashSet<u64> = layout.sheets.iter().map(|s| s.stock.thickness_mm().to_bits()).collect();
        assert_eq!(thicknesses, [19.05f64.to_bits(), 6.35f64.to_bits()].into_iter().collect());
    }

    #[test]
    fn pack_reports_unplaced_when_no_matching_stock() {
        let orphan = PackablePart::new("mystery", 500.0, 300.0, 12.7);
        let layout = pack(&[orphan.clone()], &stock(), DEFAULT_KERF_MM, 0.0);

        assert_eq!(layout.unplaced, vec![orphan]);
        assert!(layout.sheets.is_empty());
    }

    #[test]
    fn bill_of_materials_counts_sheets_per_stock_item() {
        let parts = vec![
            PackablePart { qty: 2, ..PackablePart::new("panel", 765.175, 406.4, 19.05) },
            PackablePart::new("backer", 787.4, 431.8, 6.35),
        ];
        let layout = pack(&parts, &stock(), DEFAULT_KERF_MM, 0.0);
        let bom = bill_of_materials(&layout);

        let by_name: HashMap<&str, usize> = bom.iter().map(|l| (l.stock.material.name.as_str(), l.qty)).collect();
        assert_eq!(by_name["3/4 Baltic Birch"], 1, "both 30x16in pieces fit one 96x48in sheet with real packing");
        assert_eq!(by_name["1/4 Baltic Birch"], 1);
        assert_eq!(bom.iter().map(|l| l.qty).sum::<usize>(), layout.sheets.len());
    }

    #[test]
    fn pack_places_multiple_parts_per_sheet_without_overlap() {
        let parts: Vec<PackablePart> = (0..4).map(|i| PackablePart::new(format!("panel-{i}"), 762.0, 406.4, 19.05)).collect();
        let layout = pack(&parts, &stock(), DEFAULT_KERF_MM, 0.0);

        assert!(layout.unplaced.is_empty());
        assert_eq!(layout.sheets.len(), 1, "four 30x16in panels should nest onto a single 96x48in sheet");
        let placements = &layout.sheets[0].placements;
        assert_eq!(placements.len(), 4);
        for (i, a) in placements.iter().enumerate() {
            for b in &placements[i + 1..] {
                let x_overlap = a.x_mm < b.x_mm + b.length_mm && b.x_mm < a.x_mm + a.length_mm;
                let y_overlap = a.y_mm < b.y_mm + b.width_mm && b.y_mm < a.y_mm + a.width_mm;
                assert!(!(x_overlap && y_overlap), "placements overlap: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn material_name_pins_a_part_even_at_shared_thickness() {
        let finished = Material { name: "Baltic Birch 3/4 (finished)".to_string(), thickness_mm: 19.05 };
        let utility = Material { name: "Sande Ply 3/4 (utility)".to_string(), thickness_mm: 19.05 };
        let stock = vec![
            StockSheet { material: finished.clone(), length_mm: 2438.4, width_mm: 1219.2 },
            StockSheet { material: utility.clone(), length_mm: 2438.4, width_mm: 1219.2 },
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
        let materials_used: HashSet<&str> = layout.sheets.iter().map(|s| s.stock.material.name.as_str()).collect();
        assert_eq!(materials_used, [finished.name.as_str(), utility.name.as_str()].into_iter().collect());
        for sheet in &layout.sheets {
            let labels: HashSet<&str> = sheet.placements.iter().map(|p| p.part_label.as_str()).collect();
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
        assert_eq!(fx + fw, stock()[0].length_mm, "leftover after one placement should reach exactly the sheet's full length");
        assert_eq!(fh, 600.0, "leftover should keep the strip's full height available, not just the placed part's row");
    }

    #[test]
    fn short_part_reuses_leftover_height_within_a_strip() {
        let test_stock = StockSheet { material: Material { name: "test".to_string(), thickness_mm: 19.0 }, length_mm: 1000.0, width_mm: 500.0 };
        let parts = vec![
            (0usize, PackablePart::new("tall", 200.0, 300.0, 19.0)),
            (1usize, PackablePart::new("medium", 300.0, 120.0, 19.0)),
            (2usize, PackablePart::new("short", 250.0, 90.0, 19.0)),
        ];

        let (sheets, placed_ids) = place_on_candidate(&parts, &test_stock, 0.0);

        assert_eq!(placed_ids.len(), 3);
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].strips.len(), 1, "medium and short should both reuse tall's strip, not open new ones");
        let by_label: HashMap<&str, &Placement> =
            sheets[0].strips[0].placements.iter().map(|p| (p.part_label.as_str(), p)).collect();
        assert_eq!(by_label["medium"].y_mm, 0.0);
        assert_eq!(by_label["short"].y_mm, by_label["medium"].width_mm, "short should stack directly above medium");
    }
}
