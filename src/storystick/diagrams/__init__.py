"""Render a nesting.Layout + BOM as a single printable PDF -- the workbench
artifact: a bill-of-materials page, then one page per stock sheet showing
every cut on it (labeled with its own dimensions, ready to cross out with
a marker as you go), then a Parts Index page mapping each short on-sheet
code back to the part's full CAD assembly path. No on-screen/SVG output --
nothing in this tool is meant to be viewed at a computer, everything ends
up printed.

Placement.part_label carries a part's full CAD path end to end (parts.csv
has one row per physical body, `path` column, and the CLI passes that
straight through as each PackablePart's label) -- full paths are the real
identity a placement should carry, so a specific cut can always be traced
back to its body in the CAD assembly. But a path like "Console A / Carcasses / Upper /
Upper Cabinet Carcass (Imported) (1) / [Panel] Right (2)" doesn't fit on a
small rectangle, so *this module* -- the one place that knows about
"printable" -- compresses each distinct label to a short code (P001, P002,
...) for on-page display and reference, and prints the code -> full path
mapping as an index page. Nothing upstream of here ever sees the
abbreviation.

This is the one package that directly depends on other modules' types
(storystick.nesting.Layout, BomLine) -- rendering them is its whole job,
so that's a real domain dependency, not a convenience shortcut.
"""

from __future__ import annotations

from fpdf import FPDF

from storystick.nesting import BomLine, Layout, SheetLayout
from storystick.units import format_mm_in

__all__ = ["render_pdf"]

PAGE_FORMAT = "Letter"
MARGIN_MM = 10


def _leaf(full_path: str) -> str:
    return full_path.split(" / ")[-1]


def _assign_codes(layout: Layout) -> "tuple[dict[str, str], dict[str, tuple[float, float]]]":
    """Map each distinct Placement.part_label (a full CAD path) to a short
    on-page code, assigned in first-appearance order across sheets, and
    each code to its (length_mm, width_mm) -- so the Parts Index can show
    dimensions too, letting the on-sheet label fall back to just the code
    for a part too small to carry its own dimension text legibly.
    """
    codes: "dict[str, str]" = {}
    dims: "dict[str, tuple[float, float]]" = {}
    for sheet in layout.sheets:
        for placement in sheet.placements:
            if placement.part_label not in codes:
                code = f"P{len(codes) + 1:03d}"
                codes[placement.part_label] = code
                dims[code] = (placement.length_mm, placement.width_mm)
    return codes, dims


def _add_table_page(
    pdf: FPDF, title: str, headers, compute_widths, rows, *, font_size: int = 11, row_h: int = 8, orientation: str = "P"
) -> None:
    """A titled table that starts a fresh page and paginates itself --
    reprinting the header row -- if `rows` runs past the bottom margin.
    Used for both the BOM (small, rarely paginates) and the Parts Index
    (routinely hundreds of rows).

    `compute_widths(pdf) -> list[float]` is called only *after* this page
    exists, not before -- `pdf.w`/`pdf.get_string_width()` depend on the
    current page's orientation, which is only correct once add_page() has
    actually run. Computing widths first and adding the page after (the
    previous design) silently sized columns against whatever orientation
    the *last* page happened to leave `pdf.w` at -- e.g. every real sheet
    page here is landscape, so the Parts Index columns were being sized
    for a 279mm-wide page and then drawn on a 216mm-wide one, pushing the
    last column off the visible page entirely.
    """
    pdf.add_page(orientation=orientation)
    widths = compute_widths(pdf)

    def draw_header():
        pdf.set_font("Helvetica", "B", font_size)
        for header, w in zip(headers, widths):
            pdf.cell(w, row_h, header, border=1)
        pdf.ln()
        pdf.set_font("Helvetica", "", font_size)

    pdf.set_font("Helvetica", "B", 16)
    pdf.cell(0, 10, title, new_x="LMARGIN", new_y="NEXT")
    pdf.ln(4)
    draw_header()

    if not rows:
        pdf.cell(sum(widths), row_h, "(none)", border=1)
        return

    bottom = pdf.h - MARGIN_MM
    for row in rows:
        if pdf.get_y() + row_h > bottom:
            pdf.add_page(orientation=orientation)
            draw_header()
        for value, w in zip(row, widths):
            pdf.cell(w, row_h, str(value), border=1)
        pdf.ln()


def _add_bom_page(pdf: FPDF, bom: "list[BomLine]") -> None:
    rows = [
        (
            line.qty,
            line.stock.material.name,
            format_mm_in(line.stock.length_mm),
            format_mm_in(line.stock.width_mm),
            format_mm_in(line.stock.thickness_mm),
        )
        for line in bom
    ]
    _add_table_page(
        pdf,
        "Bill of Materials",
        ["Qty", "Label", "Length", "Width", "Thickness"],
        lambda pdf: [20, 90, 25, 25, 25],
        rows,
    )


def _add_index_page(
    pdf: FPDF, codes: "dict[str, str]", dims: "dict[str, tuple[float, float]]", *, trim_allowance_mm: float = 0.0
) -> None:
    """Every part's dimensions live here too, not just on the sheet page --
    so a part too small to carry its own dimension text on the diagram
    (see _on_sheet_label) can still be looked up by code alone. Landscape,
    not portrait: full CAD paths are long and unpredictable in length, and
    a wide page is what actually gives them room rather than a
    hand-guessed column width that breaks the next time a path is longer.
    """

    def dim_str(code: str) -> str:
        length_mm, width_mm = dims[code]
        final = f"{format_mm_in(length_mm)} x {format_mm_in(width_mm)}"
        if not trim_allowance_mm:
            return final
        rough = f"{format_mm_in(length_mm + trim_allowance_mm)} x {format_mm_in(width_mm + trim_allowance_mm)}"
        return f"{rough} -> {final}"

    rows = sorted(((code, label, dim_str(code)) for label, code in codes.items()), key=lambda r: r[0])

    def compute_widths(pdf: FPDF):
        pdf.set_font("Helvetica", "", 9)
        code_w = max([pdf.get_string_width("Code")] + [pdf.get_string_width(r[0]) for r in rows], default=16) + 4
        dim_w = max([pdf.get_string_width("Dimensions")] + [pdf.get_string_width(r[2]) for r in rows], default=30) + 4
        path_w = pdf.w - 2 * MARGIN_MM - code_w - dim_w
        return [code_w, path_w, dim_w]

    _add_table_page(
        pdf, "Parts Index", ["Code", "Full Path", "Dimensions"], compute_widths, rows, font_size=9, orientation="L"
    )


def _mark_reference_corner(pdf: FPDF, x: float, y: float, size: float = 5) -> None:
    """A small right-angle bracket at the sheet's trusted corner -- the one
    external fact the two-cut (rough + trim) scheme depends on: every
    part's rough allowance extends away from here, inheriting squareness
    down the guillotine cut tree rather than needing its own mark.

    Deliberately just an icon, no caption -- explained once, elsewhere
    (not on every sheet page), and small enough that a part legitimately
    placed right at this corner (drawn on top, see _add_sheet_page) won't
    have its own label fought over by this mark.
    """
    pdf.set_draw_color(200, 0, 0)
    pdf.set_line_width(0.6)
    pdf.line(x, y, x + size, y)
    pdf.line(x, y, x, y + size)
    pdf.set_line_width(0.2)
    pdf.set_draw_color(0, 0, 0)


# Below this, a full 3-line label wouldn't reliably fit -- fall back to
# just the code. Set just under the 4"-wide nailer's ~10.8mm scaled height
# on a 96x48 sheet (confirmed legible, if tight, by eye) so that
# already-validated case keeps its full label; only genuinely smaller
# parts (a small cleat, etc.) fall back.
LABEL_MIN_HEIGHT_MM = 9


def _on_sheet_label(code: str, leaf: str, height_mm: float, dims_line: str) -> str:
    if height_mm < LABEL_MIN_HEIGHT_MM:
        return code
    return f"{code} {leaf}\n{dims_line}"


def _add_sheet_page(
    pdf: FPDF, sheet: SheetLayout, codes: "dict[str, str]", *, trim_allowance_mm: float = 0.0
) -> None:
    stock = sheet.stock
    orientation = "L" if stock.length_mm >= stock.width_mm else "P"
    pdf.add_page(orientation=orientation)

    title_h = 12
    usable_w = pdf.w - 2 * MARGIN_MM
    usable_h = pdf.h - 2 * MARGIN_MM - title_h
    scale = min(usable_w / stock.length_mm, usable_h / stock.width_mm)

    origin_x = MARGIN_MM
    origin_y = MARGIN_MM + title_h

    pdf.set_xy(MARGIN_MM, MARGIN_MM)
    pdf.set_font("Helvetica", "B", 14)
    pdf.cell(
        0,
        8,
        f"{stock.material.name} #{sheet.sheet_index + 1}  "
        f"({format_mm_in(stock.length_mm)} x {format_mm_in(stock.width_mm)} x {format_mm_in(stock.thickness_mm)})",
    )
    pdf.set_fill_color(238, 238, 238)
    pdf.set_draw_color(153, 153, 153)
    pdf.rect(origin_x, origin_y, stock.length_mm * scale, stock.width_mm * scale, style="DF")

    for placement in sheet.placements:
        x = origin_x + placement.x_mm * scale
        y = origin_y + placement.y_mm * scale
        final_w = placement.length_mm * scale
        final_h = placement.width_mm * scale
        code = codes[placement.part_label]
        leaf = _leaf(placement.part_label)

        if trim_allowance_mm:
            rough_w = (placement.length_mm + trim_allowance_mm) * scale
            rough_h = (placement.width_mm + trim_allowance_mm) * scale
            rough_len_in = format_mm_in(placement.length_mm + trim_allowance_mm)
            rough_wid_in = format_mm_in(placement.width_mm + trim_allowance_mm)

            pdf.set_fill_color(255, 255, 255)
            pdf.set_draw_color(0, 0, 0)
            pdf.set_dash_pattern(dash=1, gap=1)
            pdf.rect(x, y, rough_w, rough_h, style="DF")
            pdf.set_dash_pattern()

            pdf.set_draw_color(0, 0, 0)
            pdf.rect(x, y, final_w, final_h, style="D")

            pdf.set_font("Helvetica", "", 7)
            pdf.set_xy(x + 1, y + 1)
            dims_line = (
                f"rough {rough_len_in} x {rough_wid_in}\n"
                f"final {format_mm_in(placement.length_mm)} x {format_mm_in(placement.width_mm)}"
            )
            pdf.multi_cell(max(final_w - 2, 1), 3.2, _on_sheet_label(code, leaf, final_h, dims_line))
        else:
            pdf.set_fill_color(255, 255, 255)
            pdf.set_draw_color(0, 0, 0)
            pdf.rect(x, y, final_w, final_h, style="DF")

            pdf.set_font("Helvetica", "", 8)
            pdf.set_xy(x + 1, y + 1)
            dims_line = f"{format_mm_in(placement.length_mm)} x {format_mm_in(placement.width_mm)}"
            pdf.multi_cell(max(final_w - 2, 1), 4, _on_sheet_label(code, leaf, final_h, dims_line))

    _mark_reference_corner(pdf, origin_x, origin_y)


def render_pdf(layout: Layout, bom: "list[BomLine]", *, trim_allowance_mm: float = 0.0) -> bytes:
    """One PDF: a bill-of-materials page, one page per sheet with every
    cut on it drawn to scale and labeled with a short code + its own
    dimensions, and a closing Parts Index mapping each code back to the
    full CAD assembly path it was cut from.

    trim_allowance_mm, when nonzero, draws a second, dashed rough-cut
    outline around each placement (final dims + trim_allowance_mm in each
    direction, extending from the placement's own origin -- see
    _mark_reference_corner) alongside the solid final outline, and labels
    both. Must match whatever trim_allowance_mm was passed to pack() for
    this same Layout -- this only draws the rough outline, it doesn't
    derive it from anything in Layout itself.
    """
    pdf = FPDF(unit="mm", format=PAGE_FORMAT)
    pdf.set_auto_page_break(auto=False)

    codes, dims = _assign_codes(layout)

    _add_bom_page(pdf, bom)
    for sheet in layout.sheets:
        _add_sheet_page(pdf, sheet, codes, trim_allowance_mm=trim_allowance_mm)
    _add_index_page(pdf, codes, dims, trim_allowance_mm=trim_allowance_mm)

    return bytes(pdf.output())
