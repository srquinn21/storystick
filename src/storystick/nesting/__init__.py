"""Sheet-goods nesting.

Packs PackablePart rectangles onto StockSheet panels via a rip-first
guillotine packer, bucketed by matching thickness -- or by a specific
named Material, when a part pins one -- so a part is only ever
considered against stock it could actually come from. Matching by
thickness alone was the actual defect this replaces cutlistoptimizer.com
for (it pools every thickness onto the same virtual sheet regardless of
the Material column); matching by name is what additionally lets two
materials share a thickness (a cheap utility ply and a show-face ply,
say) without a part ever landing on the wrong one by accident.

Deliberately has no import dependency on storystick.stepcrawl: its input is
this package's own minimal PackablePart type, not stepcrawl.PartGroup, so
it stays usable against any source of parts, not just STEP files. The two
domains only meet in the CLI glue (storystick.cli).

Design decisions settled by hand:
  - Rip-first guillotine, not generic guillotine bin-packing (rectpack's
    Guillotine* algorithms, tried first). Any guillotine cut sequence is
    guaranteed *achievable*, but a generic solver is free to choose a
    crosscut-first split wherever its heuristic finds it locally denser --
    unsafe on a full, untouched sheet (a full sheet on a table saw is
    dangerous; a track saw isn't accurate enough for the repeated,
    matching cuts real parts need). The constraint only really applies to
    the *first* cut on a fresh sheet, though: once a strip is isolated by
    that rip, it's already small and manageable, so anything further
    within it is a normal, unconstrained 2D guillotine problem. So each
    strip (_Strip) always spans the sheet's full length_mm -- that's the
    one rip, made once per strip -- but *within* a strip, parts are
    packed via ordinary free-rectangle guillotine splitting (either
    orientation, leftover space reused), not forced into a single uniform
    row. Real cost, still: a strip's height is fixed by its first
    (widest) member, so it packs somewhat less densely than a fully
    unconstrained solver working across the whole sheet at once -- the
    trade for a cut sequence that's actually safe and accurate to execute.
  - rotation: never. Nothing here rotates a part 90 degrees, ever, no
    per-part exception. Grain runs along a stock sheet's length_mm by
    convention, and identical parts land in a consistent orientation for
    repeatable fence cuts -- both satisfied for free by simply never
    rotating, no per-part annotation needed.
  - Bucketed by (material name or thickness, then stock size): each
    candidate StockSheet size within a bucket is tried largest-first, with
    whatever doesn't fit carried over to the next candidate. For the
    common case (one stock size per thickness, no material pinned) this
    is just a single pass.
"""

from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Optional, Tuple

__all__ = [
    "Material",
    "PackablePart",
    "StockSheet",
    "Placement",
    "SheetLayout",
    "Layout",
    "BomLine",
    "pack",
    "bill_of_materials",
]

DEFAULT_KERF_MM = 3.2  # ~1/8"


@dataclass(frozen=True)
class Material:
    """A named material, looked up by name -- e.g. "Baltic Birch 3/4
    (finished 2 sides)" vs "Sande Ply 3/4 (utility)". Two materials can
    share a thickness_mm while being genuinely different stock, bought
    and used for different reasons (a hidden stretcher doesn't need
    show-face plywood). Thickness alone was never a strong enough key for
    "what should this part be cut from" -- that's what this type is for.
    """

    name: str
    thickness_mm: float


@dataclass(frozen=True)
class PackablePart:
    """One part to be nested. qty copies are nested independently (each
    gets its own placement), not as one rectangle labeled "x3" -- pack()
    doesn't assume they end up anywhere near each other.

    material_name=None (the default) means "any material at this part's
    own thickness_mm is fine" -- pack() matches purely by thickness, as
    it always has. Set it to pin this part to one specific Material by
    name, e.g. to keep a hidden part off the good plywood even though it
    happens to share a thickness with it.
    """

    label: str
    length_mm: float
    width_mm: float
    thickness_mm: float
    qty: int = 1
    material_name: Optional[str] = None


@dataclass(frozen=True)
class StockSheet:
    """One purchasable sheet size of a given Material -- what you *can*
    buy, not an inventory count of what you have. There's deliberately no
    qty-on-hand field: pack() always assumes you can buy as many of a
    given StockSheet as needed, and bill_of_materials() tells you how many
    that turned out to be. If you already have some material on hand,
    that's a manual adjustment you make on the BOM afterward, not
    something this catalog tracks."""

    material: Material
    length_mm: float
    width_mm: float

    @property
    def thickness_mm(self) -> float:
        return self.material.thickness_mm


@dataclass(frozen=True)
class Placement:
    """Where one physical piece landed on its sheet. length_mm/width_mm
    are the part's true final size -- never the kerf/trim-inflated
    footprint pack() actually feeds the packing algorithm."""

    part_label: str
    x_mm: float
    y_mm: float
    length_mm: float
    width_mm: float
    rotated: bool


@dataclass(frozen=True)
class SheetLayout:
    """One physical sheet (the sheet_index-th copy of `stock` used) and
    everything placed on it."""

    stock: StockSheet
    sheet_index: int
    placements: Tuple[Placement, ...]


@dataclass(frozen=True)
class Layout:
    """The full result of a pack() call: every sheet used, plus any parts
    that couldn't be placed at all -- which, with no qty-on-hand concept,
    only ever means no stock in the catalog at this thickness was big
    enough for this part."""

    sheets: Tuple[SheetLayout, ...]
    unplaced: Tuple[PackablePart, ...]


@dataclass(frozen=True)
class BomLine:
    """One purchasing line: buy `qty` of `stock`."""

    stock: StockSheet
    qty: int


FreeRect = Tuple[float, float, float, float]  # (x, y, length, width) in a strip's local frame


@dataclass
class _Strip:
    """One rip-defined strip: it always spans the sheet's full length_mm --
    that's the rip, made once, when the strip is created -- and its
    height (`width_mm`, fixed at creation by whichever part is first
    placed into it) is never exceeded. That's the *only* constraint
    inherited from being carved out of a full, unwieldy sheet.

    Everything else about what happens *inside* a strip is a normal,
    unconstrained 2D guillotine packing problem (`free_rects`, split
    freely in either orientation): once a strip is isolated by its rip,
    it's already a small, manageable piece, not the original sheet, so
    there's no safety reason left to restrict cut order within it. This
    is what lets a short part reuse the leftover height above another
    part in the same strip instead of that space just going to waste.
    """

    y_mm: float
    width_mm: float
    free_rects: List[FreeRect] = field(default_factory=list)
    placements: List[Placement] = field(default_factory=list)


@dataclass
class _SheetInProgress:
    sheet_index: int
    used_width_mm: float = 0.0
    strips: List[_Strip] = field(default_factory=list)


def _best_fit_free_rect(free_rects: List[FreeRect], length: float, width: float):
    """Index of the smallest-area free rect (within one strip) that fits
    (length, width) without rotation, or None."""
    best = None
    for i, (_x, _y, fw, fh) in enumerate(free_rects):
        if length <= fw and width <= fh:
            area = fw * fh
            if best is None or area < best[0]:
                best = (area, i)
    return best[1] if best is not None else None


def _split_free_rect(free_rects: List[FreeRect], index: int, length: float, width: float) -> Tuple[float, float]:
    """Place a (length, width) piece into free_rects[index]'s own corner,
    replacing it with up to two leftover rects via a guillotine split
    (shorter-leftover-axis rule: whichever leftover side is smaller stays
    attached to the placed piece's row/column, the larger one becomes its
    own free rect). Safe in either split orientation -- this free rect
    already belongs to an isolated strip, never the original full sheet.
    Returns the piece's placement origin (x, y).
    """
    fx, fy, fw, fh = free_rects.pop(index)
    right_w, top_h = fw - length, fh - width
    if right_w <= top_h:
        if right_w > 1e-6:
            free_rects.append((fx + length, fy, right_w, width))
        if top_h > 1e-6:
            free_rects.append((fx, fy + width, fw, top_h))
    else:
        if top_h > 1e-6:
            free_rects.append((fx, fy + width, length, top_h))
        if right_w > 1e-6:
            free_rects.append((fx + length, fy, right_w, fh))
    return fx, fy


def _place_on_candidate(
    pieces: List[Tuple[int, PackablePart]], candidate: StockSheet, *, allowance_mm: float
) -> Tuple[List[_SheetInProgress], List[int]]:
    """Pack `pieces` onto as many copies of `candidate` as needed. Parts
    are tried width-descending (classic strip/FFDH heuristic: the widest
    parts define strip heights first, narrower parts fill in behind
    them). For each part: best-fit into any open strip's free space; else
    open a new strip on any open sheet with enough remaining width; else
    open a new sheet.

    Returns the in-progress sheets touched and the ids of pieces that got
    placed -- the caller carries over whatever's left to the next
    candidate stock size.
    """
    ordered = sorted(pieces, key=lambda p: p[1].width_mm, reverse=True)

    sheets: List[_SheetInProgress] = []
    placed_ids: List[int] = []

    for piece_id, part in ordered:
        length = part.length_mm + allowance_mm
        width = part.width_mm + allowance_mm
        if length > candidate.length_mm or width > candidate.width_mm:
            continue  # too big for this stock size at all, regardless of sheet count

        strip = None
        free_index = None
        for sheet in sheets:
            for s in sheet.strips:
                idx = _best_fit_free_rect(s.free_rects, length, width)
                if idx is not None:
                    strip, free_index = s, idx
                    break
            if strip is not None:
                break

        if strip is None:
            sheet = next((s for s in sheets if s.used_width_mm + width <= candidate.width_mm), None)
            if sheet is None:
                sheet = _SheetInProgress(sheet_index=len(sheets))
                sheets.append(sheet)
            strip = _Strip(y_mm=sheet.used_width_mm, width_mm=width, free_rects=[(0.0, sheet.used_width_mm, candidate.length_mm, width)])
            sheet.strips.append(strip)
            sheet.used_width_mm += width
            free_index = 0

        x, y = _split_free_rect(strip.free_rects, free_index, length, width)
        strip.placements.append(
            Placement(part_label=part.label, x_mm=x, y_mm=y, length_mm=part.length_mm, width_mm=part.width_mm, rotated=False)
        )
        placed_ids.append(piece_id)

    return sheets, placed_ids


def pack(
    parts: Iterable[PackablePart],
    stock: Iterable[StockSheet],
    *,
    kerf_mm: float = DEFAULT_KERF_MM,
    trim_allowance_mm: float = 0.0,
) -> Layout:
    """Nest parts onto stock sheets via rip-first guillotine packing.

    Bucketed by (material_name, thickness) when a part pins a specific
    Material by name, else by thickness_mm alone (any material at that
    thickness is fair game) -- see PackablePart.material_name. Each
    part's packed footprint is inflated by trim_allowance_mm + kerf_mm in
    both dimensions before packing -- kerf reserves the blade's own width
    between two adjacent cuts, and trim_allowance (if nonzero) reserves
    extra rough-cut margin on top of that for a later, separate finishing
    pass (see storystick.diagrams). Reported Placement sizes are always
    the true final dimensions, never the inflated packing footprint.
    """
    allowance_mm = kerf_mm + trim_allowance_mm

    # Bucket key: the assigned material name if the part pins one,
    # otherwise its thickness_mm -- two different key "shapes" that never
    # collide since one's a str and the other's a float.
    pieces_by_bucket: Dict[object, List[Tuple[int, PackablePart]]] = defaultdict(list)
    next_id = 0
    for part in parts:
        bucket = part.material_name if part.material_name else part.thickness_mm
        for _ in range(part.qty):
            pieces_by_bucket[bucket].append((next_id, part))
            next_id += 1

    stock_list = list(stock)
    sheets: List[SheetLayout] = []
    unplaced: List[PackablePart] = []
    next_sheet_index: Dict[StockSheet, int] = {}

    for bucket, pieces in pieces_by_bucket.items():
        if isinstance(bucket, str):
            candidates_iter = (s for s in stock_list if s.material.name == bucket)
        else:
            candidates_iter = (s for s in stock_list if s.thickness_mm == bucket)
        candidates = sorted(candidates_iter, key=lambda s: s.length_mm * s.width_mm, reverse=True)

        remaining = pieces
        for candidate in candidates:
            if not remaining:
                break
            in_progress, placed_ids = _place_on_candidate(remaining, candidate, allowance_mm=allowance_mm)
            for sheet in in_progress:
                placements = [p for strip in sheet.strips for p in strip.placements]
                if not placements:
                    continue
                sheet_index = next_sheet_index.get(candidate, 0)
                next_sheet_index[candidate] = sheet_index + 1
                sheets.append(SheetLayout(stock=candidate, sheet_index=sheet_index, placements=tuple(placements)))

            placed_set = set(placed_ids)
            remaining = [(piece_id, part) for piece_id, part in remaining if piece_id not in placed_set]

        unplaced.extend(part for _piece_id, part in remaining)

    return Layout(sheets=tuple(sheets), unplaced=tuple(unplaced))


def bill_of_materials(layout: Layout) -> List[BomLine]:
    """Roll a Layout up into purchasing lines: how many of each StockSheet
    got used. Purely a count of layout.sheets grouped by stock -- there's
    no on-hand quantity to net out, since StockSheet is a catalog entry,
    not an inventory count (see StockSheet's docstring). Sorted by
    material name then thickness for a stable, readable BOM.
    """
    counts = Counter(s.stock for s in layout.sheets)
    lines = [BomLine(stock=stock, qty=qty) for stock, qty in counts.items()]
    lines.sort(key=lambda line: (line.stock.material.name, line.stock.thickness_mm))
    return lines
