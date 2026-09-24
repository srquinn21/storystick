"""Proves the three domain packages communicate end-to-end through their
public APIs alone -- a PartGroup list shaped like stepcrawl's real output
is packable by nesting, and nesting's output is renderable by diagrams.
Deliberately builds PartGroups by hand rather than via
storystick.stepcrawl.extract_parts, so this suite doesn't depend on a real
or synthetic STEP file -- STEP-parsing correctness is stepcrawl's own
concern, not the pipeline-wiring concern this file tests.
"""

import re

from storystick.diagrams import render_pdf
from storystick.nesting import Material, PackablePart, StockSheet, bill_of_materials, pack
from storystick.stepcrawl import PartGroup, PartInstance

THREE_QUARTER = Material(name="3/4 Baltic Birch", thickness_mm=19.05)
QUARTER = Material(name="1/4 Baltic Birch", thickness_mm=6.35)

STOCK = [
    StockSheet(material=THREE_QUARTER, length_mm=2438.4, width_mm=1219.2),
    StockSheet(material=QUARTER, length_mm=2438.4, width_mm=1219.2),
]

FIXTURE_PARTS = [
    PartGroup(
        top_folder="Bench",
        length_mm=765.175,  # 30.125"
        width_mm=406.4,  # 16"
        thickness_mm=19.05,  # 0.75"
        instances=(
            PartInstance(path="Bench / Carcasses / Carcass A / [Panel] Bottom"),
            PartInstance(path="Bench / Carcasses / Carcass B / [Panel] Bottom (1)"),
        ),
    ),
    PartGroup(
        top_folder="Bench",
        length_mm=787.4,  # 31"
        width_mm=431.8,  # 17"
        thickness_mm=6.35,  # 0.25"
        instances=(PartInstance(path="Bench / Carcasses / Carcass A / [Backer]"),),
    ),
]


def _to_packable(parts):
    return [
        PackablePart(label=g.top_folder, length_mm=g.length_mm, width_mm=g.width_mm, thickness_mm=g.thickness_mm, qty=g.qty)
        for g in parts
    ]


def test_pack_buckets_by_thickness_not_just_size():
    layout = pack(_to_packable(FIXTURE_PARTS), STOCK)

    assert not layout.unplaced, "every fixture part's thickness has a matching stock sheet"
    used_thicknesses = {s.stock.thickness_mm for s in layout.sheets}
    assert used_thicknesses == {19.05, 6.35}, "0.75in and 0.25in parts must land on their own stock, never mixed"


def test_render_pdf_has_one_page_per_sheet_plus_a_bom_page():
    layout = pack(_to_packable(FIXTURE_PARTS), STOCK)
    bom = bill_of_materials(layout)

    pdf_bytes = render_pdf(layout, bom)

    assert pdf_bytes[:4] == b"%PDF"
    page_count = len(re.findall(rb"/Type\s*/Page\b", pdf_bytes))
    assert page_count == len(layout.sheets) + 2, "one BOM page, one page per sheet, one Parts Index page"


def test_pack_reports_unplaced_when_no_matching_stock():
    orphan = PackablePart(label="Mystery Panel", length_mm=500, width_mm=300, thickness_mm=12.7, qty=1)

    layout = pack([orphan], STOCK)

    assert layout.unplaced == (orphan,)
    assert layout.sheets == ()


def test_bill_of_materials_counts_sheets_per_stock_item():
    layout = pack(_to_packable(FIXTURE_PARTS), STOCK)

    bom = bill_of_materials(layout)

    by_label = {line.stock.material.name: line.qty for line in bom}
    assert by_label["3/4 Baltic Birch"] == 1, "both 30x16in pieces easily fit one 96x48in sheet with real packing"
    assert by_label["1/4 Baltic Birch"] == 1
    assert sum(line.qty for line in bom) == len(layout.sheets)


def test_pack_places_multiple_parts_per_sheet_without_overlap():
    # Four 30x16in pieces: comfortably gang onto one 96x48in sheet two-up,
    # two rows -- real packing should use one sheet, not four.
    parts = [PackablePart(label=f"panel-{i}", length_mm=762, width_mm=406.4, thickness_mm=19.05, qty=1) for i in range(4)]

    layout = pack(parts, STOCK)

    assert not layout.unplaced
    assert len(layout.sheets) == 1, "four 30x16in panels should nest onto a single 96x48in sheet"
    placements = layout.sheets[0].placements
    assert len(placements) == 4
    for i, a in enumerate(placements):
        for b in placements[i + 1 :]:
            x_overlap = a.x_mm < b.x_mm + b.length_mm and b.x_mm < a.x_mm + a.length_mm
            y_overlap = a.y_mm < b.y_mm + b.width_mm and b.y_mm < a.y_mm + a.width_mm
            assert not (x_overlap and y_overlap), f"placements overlap: {a} vs {b}"


def test_new_strip_spans_the_sheets_full_length():
    """The one safety guarantee that actually matters: a strip's free
    space is seeded to span the sheet's full length_mm the moment it's
    created -- that's the rip, made once, on an otherwise untouched
    sheet. Checked directly (white-box) by placing a single part that
    doesn't use the whole length, then confirming the leftover free rect
    accounts for exactly the rest of that same full length -- not some
    shorter, already-subdivided extent.
    """
    from storystick.nesting import _place_on_candidate

    part = PackablePart(label="wide-a", length_mm=1200, width_mm=600, thickness_mm=19.05, qty=1)

    sheets, placed_ids = _place_on_candidate([(0, part)], STOCK[0], allowance_mm=0.0)

    assert placed_ids == [0]
    strip = sheets[0].strips[0]
    assert len(strip.free_rects) == 1
    fx, _fy, fw, fh = strip.free_rects[0]
    assert fx + fw == STOCK[0].length_mm, "leftover after one placement should reach exactly the sheet's full length"
    assert fh == 600, "leftover should keep the strip's full height available, not just the placed part's row"


def test_material_name_pins_a_part_to_one_stock_option_even_at_shared_thickness():
    """The whole point of Material: a hidden, cheap-material part must
    never land on the same sheet as a show-face part just because they
    happen to share a thickness.
    """
    finished = Material(name="Baltic Birch 3/4 (finished)", thickness_mm=19.05)
    utility = Material(name="Sande Ply 3/4 (utility)", thickness_mm=19.05)
    stock = [
        StockSheet(material=finished, length_mm=2438.4, width_mm=1219.2),
        StockSheet(material=utility, length_mm=2438.4, width_mm=1219.2),
    ]
    panel = PackablePart(label="show-face panel", length_mm=700, width_mm=400, thickness_mm=19.05, qty=1, material_name=finished.name)
    stretcher = PackablePart(label="hidden stretcher", length_mm=700, width_mm=100, thickness_mm=19.05, qty=1, material_name=utility.name)

    layout = pack([panel, stretcher], stock)

    assert not layout.unplaced
    materials_used = {sheet.stock.material.name for sheet in layout.sheets}
    assert materials_used == {finished.name, utility.name}, "each part must land on its own assigned material, never the other"
    for sheet in layout.sheets:
        labels = {p.part_label for p in sheet.placements}
        if sheet.stock.material.name == finished.name:
            assert labels == {"show-face panel"}
        else:
            assert labels == {"hidden stretcher"}


def test_unassigned_part_still_matches_by_thickness_alone():
    """No material set on a part -- the common case -- keeps working
    exactly as before: any stock at the part's own thickness is fair
    game, chosen by pack()'s usual largest-candidate-first rule.
    """
    part = PackablePart(label="whatever", length_mm=700, width_mm=400, thickness_mm=19.05, qty=1)

    layout = pack([part], STOCK)

    assert not layout.unplaced
    assert layout.sheets[0].stock.material.name == THREE_QUARTER.name


def test_short_part_reuses_leftover_height_within_a_strip():
    """This is the actual fix: a strip's height is set by whichever part
    creates it, but a second, narrower part placed beside it can leave
    leftover height above itself -- and a third, short part should be
    able to stack directly into that leftover, inside the *same* strip,
    rather than being forced to open a brand new one. That reuse is what
    was missing when strips only accepted same-height parts.
    """
    from storystick.nesting import _place_on_candidate

    stock = StockSheet(material=Material(name="test", thickness_mm=19), length_mm=1000, width_mm=500)
    parts = [
        (0, PackablePart(label="tall", length_mm=200, width_mm=300, thickness_mm=19, qty=1)),
        (1, PackablePart(label="medium", length_mm=300, width_mm=120, thickness_mm=19, qty=1)),
        (2, PackablePart(label="short", length_mm=250, width_mm=90, thickness_mm=19, qty=1)),
    ]

    sheets, placed_ids = _place_on_candidate(parts, stock, allowance_mm=0.0)

    assert set(placed_ids) == {0, 1, 2}
    assert len(sheets) == 1
    assert len(sheets[0].strips) == 1, "medium and short should both reuse tall's strip, not open new ones"
    by_label = {p.part_label: p for p in sheets[0].strips[0].placements}
    assert by_label["medium"].y_mm == 0.0
    assert by_label["short"].y_mm == by_label["medium"].width_mm, "short should stack directly above medium"
