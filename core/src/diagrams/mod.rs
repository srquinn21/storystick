//! Render a nesting::Layout + BOM as a single printable PDF -- the workbench
//! artifact: a bill-of-materials page, then one page per stock sheet showing
//! every cut on it (labeled with its own dimensions, ready to cross out with
//! a marker as you go), then a Parts Index page mapping each short on-sheet
//! code back to the part's full CAD assembly path. No on-screen/SVG output --
//! nothing in this tool is meant to be viewed at a computer, everything ends
//! up printed.
//!
//! Placement::part_label carries a part's full CAD path end to end (parts.csv
//! has one row per physical body, `path` column, and the CLI passes that
//! straight through as each PackablePart's label) -- full paths are the real
//! identity a placement should carry, so a specific cut can always be traced
//! back to its body in the CAD assembly. But a path like "Console A / Carcasses / Upper /
//! Upper Cabinet Carcass (Imported) (1) / [Panel] Right (2)" doesn't fit on a
//! small rectangle, so *this module* -- the one place that knows about
//! "printable" -- compresses each distinct label to a short code (P001, P002,
//! ...) for on-page display and reference, and prints the code -> full path
//! mapping as an index page. Nothing upstream of here ever sees the
//! abbreviation.
//!
//! This is the one module that directly depends on `nesting`'s types
//! (Layout, BomLine) -- rendering them is its whole job, so that's a real
//! domain dependency, not a convenience shortcut.
//!
//! All internal layout math (`Page`) works in the same top-left-origin,
//! y-grows-downward coordinate system the original fpdf2-based renderer
//! used, converting to printpdf's native bottom-left/y-grows-upward system
//! only at the point of emitting an `Op`. This keeps every geometric
//! computation below identical in spirit to the Python original instead of
//! fighting two coordinate systems throughout.

use crate::nesting::{
    bill_of_materials, cut_steps, BomLine, CutKind, CutStep, Layout, Outcome, SheetLayout, Source,
};
use crate::units::format_mm_in;
use printpdf::{
    BuiltinFont, Color, Line, LineDashPattern, LinePoint, Op, ParsedFont, PdfDocument,
    PdfFontHandle, PdfPage, PdfSaveOptions, Point, Rect, Rgb, TextItem,
};
use std::collections::HashMap;

const MARGIN_MM: f64 = 10.0;
const LETTER_W_MM: f64 = 215.9;
const LETTER_H_MM: f64 = 279.4;
const MM_PER_PT: f64 = 25.4 / 72.0;

// Below this, a full 3-line label wouldn't reliably fit -- fall back to
// just the code. Set just under the 4"-wide nailer's ~10.8mm scaled height
// on a 96x48 sheet (confirmed legible, if tight, by eye) so that
// already-validated case keeps its full label; only genuinely smaller
// parts (a small cleat, etc.) fall back.
const LABEL_MIN_HEIGHT_MM: f64 = 9.0;

/// Extra room reserved along a sheet diagram's left edge (for rip
/// ticks) and bottom edge (for crosscut ticks) -- see `render_dim_marks`
/// -- only reserved on whichever side actually has a mark; a
/// single-strip, single-part sheet keeps the full page for its diagram,
/// same as before either gutter existed.
const DIM_GUTTER_MM: f64 = 14.0;

fn leaf(full_path: &str) -> &str {
    full_path.rsplit(" / ").next().unwrap_or(full_path)
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(Rgb {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        icc_profile: None,
    })
}

fn mm(v: f64) -> printpdf::Mm {
    printpdf::Mm(v as f32)
}

/// Text-width measurement for the two built-in fonts this module uses.
/// printpdf's builtin fonts have no one-line "measure this string" helper
/// (unlike fpdf2's `get_string_width`); this reconstructs one from glyph
/// widths, which is exact for a Latin-1 label/dimension string against a
/// standard-14 font.
struct Metrics {
    regular: ParsedFont,
    bold: ParsedFont,
}

impl Metrics {
    fn new() -> Self {
        Self {
            regular: BuiltinFont::Helvetica
                .get_parsed_font()
                .expect("builtin Helvetica should always parse"),
            bold: BuiltinFont::HelveticaBold
                .get_parsed_font()
                .expect("builtin Helvetica-Bold should always parse"),
        }
    }

    fn width_mm(&self, text: &str, size_pt: f64, bold: bool) -> f64 {
        let font = if bold { &self.bold } else { &self.regular };
        let units_per_em = font.font_metrics.units_per_em as f64;
        let units: f64 = text
            .chars()
            .map(|c| {
                let gid = font.lookup_glyph_index(c as u32).unwrap_or(0);
                font.get_horizontal_advance(gid) as f64
            })
            .sum();
        units / units_per_em * size_pt * MM_PER_PT
    }

    /// Greedy word-wrap of one logical line (no embedded '\n') to fit
    /// `max_w_mm`. A single word wider than `max_w_mm` is kept whole
    /// (never mid-word split) rather than overflowing the box invisibly.
    fn wrap(&self, line: &str, max_w_mm: f64, size_pt: f64) -> Vec<String> {
        if line.is_empty() {
            return vec![String::new()];
        }
        let mut lines = Vec::new();
        let mut current = String::new();
        for word in line.split(' ') {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if current.is_empty() || self.width_mm(&candidate, size_pt, false) <= max_w_mm {
                current = candidate;
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        lines
    }
}

/// One page under construction, in top-left-origin/y-down mm coordinates.
struct Page {
    width_mm: f64,
    height_mm: f64,
    ops: Vec<Op>,
}

impl Page {
    fn new(width_mm: f64, height_mm: f64) -> Self {
        Self {
            width_mm,
            height_mm,
            ops: vec![Op::SetOutlineThickness {
                pt: mm(0.2).into_pt(),
            }],
        }
    }

    fn finish(self) -> PdfPage {
        PdfPage::new(mm(self.width_mm), mm(self.height_mm), self.ops)
    }

    fn flip_y(&self, y_mm: f64) -> f64 {
        self.height_mm - y_mm
    }

    fn set_fill(&mut self, r: u8, g: u8, b: u8) {
        self.ops.push(Op::SetFillColor { col: rgb(r, g, b) });
    }

    fn set_stroke(&mut self, r: u8, g: u8, b: u8) {
        self.ops.push(Op::SetOutlineColor { col: rgb(r, g, b) });
    }

    fn set_line_width_mm(&mut self, w_mm: f64) {
        self.ops.push(Op::SetOutlineThickness {
            pt: mm(w_mm).into_pt(),
        });
    }

    fn set_dash_mm(&mut self, dash_mm: f64, gap_mm: f64) {
        let dash_pt = mm(dash_mm).into_pt().0;
        let gap_pt = mm(gap_mm).into_pt().0;
        self.ops.push(Op::SetLineDashPattern {
            dash: LineDashPattern::new(0.0, &[dash_pt, gap_pt]),
        });
    }

    fn clear_dash(&mut self) {
        self.ops.push(Op::SetLineDashPattern {
            dash: LineDashPattern::solid(),
        });
    }

    /// x_mm, y_mm name the box's top-left corner, matching the Python
    /// original throughout (fpdf2 is also top-left-origin).
    fn rect(&mut self, x_mm: f64, y_mm: f64, w_mm: f64, h_mm: f64, mode: printpdf::PaintMode) {
        let bottom_left_y = self.flip_y(y_mm + h_mm);
        self.ops.push(Op::DrawRectangle {
            rectangle: Rect {
                x: mm(x_mm).into_pt(),
                y: mm(bottom_left_y).into_pt(),
                width: mm(w_mm).into_pt(),
                height: mm(h_mm).into_pt(),
                mode: Some(mode),
                winding_order: None,
            },
        });
    }

    fn line(&mut self, x1_mm: f64, y1_mm: f64, x2_mm: f64, y2_mm: f64) {
        let p1 = Point::new(mm(x1_mm), mm(self.flip_y(y1_mm)));
        let p2 = Point::new(mm(x2_mm), mm(self.flip_y(y2_mm)));
        self.ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: p1,
                        bezier: false,
                    },
                    LinePoint {
                        p: p2,
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
    }

    /// x_mm/y_mm is the text baseline's start point, top-left-origin.
    fn text(&mut self, x_mm: f64, y_mm: f64, s: &str, bold: bool, size_pt: f64) {
        if s.is_empty() {
            return;
        }
        self.ops.push(Op::SetFillColor { col: rgb(0, 0, 0) });
        self.ops.push(Op::StartTextSection);
        self.ops.push(Op::SetFont {
            font: PdfFontHandle::Builtin(if bold {
                BuiltinFont::HelveticaBold
            } else {
                BuiltinFont::Helvetica
            }),
            size: printpdf::Pt(size_pt as f32),
        });
        self.ops.push(Op::SetTextCursor {
            pos: Point::new(mm(x_mm), mm(self.flip_y(y_mm))),
        });
        self.ops.push(Op::ShowText {
            items: vec![TextItem::Text(s.to_string())],
        });
        self.ops.push(Op::EndTextSection);
    }

    /// Draws `text` (which may contain explicit '\n's, each independently
    /// word-wrapped to `max_w_mm`) starting with its first baseline
    /// `line_h_mm * 0.7` below `y_mm`, one line per `line_h_mm` after that --
    /// mirroring fpdf2's `multi_cell` as used here (explicit newlines for
    /// layout, wrapping only as a width safety net).
    #[allow(clippy::too_many_arguments)] // plain PDF-drawing params, not worth a bespoke struct for one private helper
    fn multiline(
        &mut self,
        x_mm: f64,
        y_mm: f64,
        max_w_mm: f64,
        line_h_mm: f64,
        text: &str,
        size_pt: f64,
        metrics: &Metrics,
    ) {
        let mut y = y_mm;
        for raw_line in text.split('\n') {
            for wrapped in metrics.wrap(raw_line, max_w_mm, size_pt) {
                self.text(x_mm, y + line_h_mm * 0.7, &wrapped, false, size_pt);
                y += line_h_mm;
            }
        }
    }
}

/// P001, P002, ... in first-appearance order across `sheets`. Takes any
/// borrowed-`SheetLayout` iterator, not a whole `&Layout`, so the same
/// function numbers both a whole-project index (`assign_codes(&layout.sheets)`)
/// and a per-section one (`assign_codes(section_sheets)`) -- the latter
/// restarting at P001 for each section, since `render_pdf` calls this
/// once per section rather than once globally (see that function's own
/// docs for why).
fn assign_codes<'a>(
    sheets: impl IntoIterator<Item = &'a SheetLayout>,
) -> (HashMap<String, String>, HashMap<String, (f64, f64)>) {
    let mut codes: HashMap<String, String> = HashMap::new();
    let mut dims: HashMap<String, (f64, f64)> = HashMap::new();
    for sheet in sheets {
        for placement in &sheet.placements {
            if !codes.contains_key(&placement.part_label) {
                let code = format!("P{:03}", codes.len() + 1);
                dims.insert(code.clone(), (placement.length_mm, placement.width_mm));
                codes.insert(placement.part_label.clone(), code);
            }
        }
    }
    (codes, dims)
}

/// Groups `sheets` into construction-stage sections for a PDF that lists
/// pages section by section (see docs/poc.md's "folder-name-based PDF
/// section grouping") -- pure grouping logic only, no page layout: what a
/// section's pages actually look like is `render_pdf`'s job once this
/// feeds it, not this function's.
///
/// A sheet's section is decided by its *first* placement's part_label --
/// the simplest deterministic rule, and the right one for the common case
/// nesting already tends toward (parts sharing a material/thickness bucket
/// tend to share a construction stage too). A sheet whose placements
/// actually span two sections is a real, if rare, edge case this doesn't
/// try to split -- worth revisiting once real full-project data shows
/// whether it matters in practice, per docs/poc.md's own "resolved:
/// deferred" pattern for open questions like this.
///
/// `classify` maps a placement's part_label (a full CAD path) to a section
/// label -- typically `crate::tags::classify_by_keyword` against a
/// construction-stage keyword list; `None` (including for a sheet with no
/// placements at all) files the sheet under `unsectioned_label`. Sections
/// come back in first-appearance order; sheets keep their original
/// relative order within a section.
pub fn group_sheets_by_section<'a>(
    sheets: &'a [SheetLayout],
    classify: impl Fn(&str) -> Option<String>,
    unsectioned_label: &str,
) -> Vec<(String, Vec<&'a SheetLayout>)> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&'a SheetLayout>> = HashMap::new();
    for sheet in sheets {
        let section = sheet
            .placements
            .first()
            .and_then(|p| classify(&p.part_label))
            .unwrap_or_else(|| unsectioned_label.to_string());
        if !groups.contains_key(&section) {
            order.push(section.clone());
            groups.insert(section.clone(), Vec::new());
        }
        groups.get_mut(&section).unwrap().push(sheet);
    }
    order
        .into_iter()
        .map(|label| (label.clone(), groups.remove(&label).unwrap()))
        .collect()
}

/// A titled table that starts a fresh page and paginates itself --
/// reprinting the header row -- if `rows` runs past the bottom margin. Used
/// for the BOM (small, rarely paginates), the Parts Index (routinely
/// hundreds of rows), and each section's front page (see `render_pdf`).
/// Column widths are computed once, up front, by the caller (never before
/// the page's own orientation/width is known -- see the Parts Index's
/// dynamic widths in `render_index_pages`).
struct TableSpec {
    title: String,
    headers: Vec<String>,
    widths: Vec<f64>,
    rows: Vec<Vec<String>>,
    font_size_pt: f64,
    row_h_mm: f64,
    landscape: bool,
    /// A ruled "Notes" box (see `render_notes_box`) filling whatever page
    /// space is left below the table -- only a section's front page wants
    /// this (these plans travel on a clipboard); the BOM and Parts Index
    /// never do.
    notes: bool,
}

/// How much vertical room a notes box needs to be worth drawing at all --
/// below this, it opens a fresh page instead of squeezing in under an
/// already-tall table (see `render_table_pages`). Small blank margin below
/// a table is otherwise left alone, same as before this existed.
const NOTES_MIN_HEIGHT_MM: f64 = 40.0;
const NOTES_LINE_SPACING_MM: f64 = 8.0;

/// A bordered, ruled area for handwritten notes, filling
/// `(x_mm, y_mm)` to `(x_mm + w_mm, y_mm + h_mm)`. "Notes" here means
/// literal blank space to write on -- these project plans go on a
/// clipboard in the shop -- not generated content; the ruled lines are
/// just a writing aid, spaced for pen-and-paper handwriting.
fn render_notes_box(page: &mut Page, x_mm: f64, y_mm: f64, w_mm: f64, h_mm: f64) {
    page.set_stroke(0, 0, 0);
    page.set_line_width_mm(0.3);
    page.rect(x_mm, y_mm, w_mm, h_mm, printpdf::PaintMode::Stroke);
    page.text(x_mm + 2.0, y_mm + 6.0, "Notes", true, 10.0);

    page.set_stroke(210, 210, 210);
    page.set_line_width_mm(0.15);
    let mut ruled_y = y_mm + 14.0;
    while ruled_y < y_mm + h_mm - 4.0 {
        page.line(x_mm + 2.0, ruled_y, x_mm + w_mm - 2.0, ruled_y);
        ruled_y += NOTES_LINE_SPACING_MM;
    }
    page.set_stroke(0, 0, 0);
    page.set_line_width_mm(0.2);
}

fn render_table_pages(spec: &TableSpec) -> Vec<Page> {
    let (page_w, page_h) = if spec.landscape {
        (LETTER_H_MM, LETTER_W_MM)
    } else {
        (LETTER_W_MM, LETTER_H_MM)
    };
    let bottom = page_h - MARGIN_MM;

    #[allow(clippy::too_many_arguments)] // plain PDF-drawing params, not worth a bespoke struct for one private helper
    fn draw_row(
        page: &mut Page,
        x0: f64,
        y_top: f64,
        widths: &[f64],
        values: &[String],
        row_h: f64,
        font_size_pt: f64,
        bold: bool,
    ) {
        let mut x = x0;
        for (value, w) in values.iter().zip(widths) {
            page.set_stroke(0, 0, 0);
            page.rect(x, y_top, *w, row_h, printpdf::PaintMode::Stroke);
            page.text(x + 1.0, y_top + row_h * 0.7, value, bold, font_size_pt);
            x += w;
        }
    }

    let mut pages = Vec::new();
    let mut page = Page::new(page_w, page_h);
    let mut y = MARGIN_MM;

    page.text(MARGIN_MM, y + 10.0 * 0.7, &spec.title, true, 16.0);
    y += 10.0 + 4.0;

    draw_row(
        &mut page,
        MARGIN_MM,
        y,
        &spec.widths,
        &spec.headers,
        spec.row_h_mm,
        spec.font_size_pt,
        true,
    );
    y += spec.row_h_mm;

    if spec.rows.is_empty() {
        let total_w: f64 = spec.widths.iter().sum();
        draw_row(
            &mut page,
            MARGIN_MM,
            y,
            &[total_w],
            &["(none)".to_string()],
            spec.row_h_mm,
            spec.font_size_pt,
            false,
        );
        y += spec.row_h_mm;
    } else {
        for row in &spec.rows {
            if y + spec.row_h_mm > bottom {
                pages.push(page);
                page = Page::new(page_w, page_h);
                y = MARGIN_MM;
                draw_row(
                    &mut page,
                    MARGIN_MM,
                    y,
                    &spec.widths,
                    &spec.headers,
                    spec.row_h_mm,
                    spec.font_size_pt,
                    true,
                );
                y += spec.row_h_mm;
            }
            draw_row(
                &mut page,
                MARGIN_MM,
                y,
                &spec.widths,
                row,
                spec.row_h_mm,
                spec.font_size_pt,
                false,
            );
            y += spec.row_h_mm;
        }
    }

    if spec.notes {
        if bottom - y < NOTES_MIN_HEIGHT_MM {
            pages.push(page);
            page = Page::new(page_w, page_h);
            y = MARGIN_MM;
        }
        render_notes_box(
            &mut page,
            MARGIN_MM,
            y,
            page_w - 2.0 * MARGIN_MM,
            bottom - y,
        );
    }

    pages.push(page);
    pages
}

fn bom_rows(bom: &[BomLine]) -> Vec<Vec<String>> {
    bom.iter()
        .map(|line| {
            vec![
                line.qty.to_string(),
                line.stock.material.name.clone(),
                format_mm_in(line.stock.length_mm),
                format_mm_in(line.stock.width_mm),
                format_mm_in(line.stock.thickness_mm()),
            ]
        })
        .collect()
}

/// A Bill-of-Materials table under `title` -- the whole-project BOM
/// (`notes: false`) and each section's own front-page BOM (`notes: true`,
/// see `render_pdf`) share this one function rather than growing two
/// near-identical table specs.
fn render_bom_table_pages(title: &str, bom: &[BomLine], notes: bool) -> Vec<Page> {
    render_table_pages(&TableSpec {
        title: title.to_string(),
        headers: vec![
            "Qty".to_string(),
            "Label".to_string(),
            "Length".to_string(),
            "Width".to_string(),
            "Thickness".to_string(),
        ],
        widths: vec![20.0, 90.0, 25.0, 25.0, 25.0],
        rows: bom_rows(bom),
        font_size_pt: 11.0,
        row_h_mm: 8.0,
        landscape: false,
        notes,
    })
}

/// Every part's dimensions live here too, not just on the sheet page -- so
/// a part too small to carry its own dimension text on the diagram (see
/// `on_sheet_label`) can still be looked up by code alone. Landscape, not
/// portrait: full CAD paths are long and unpredictable in length, and a
/// wide page is what actually gives them room rather than a hand-guessed
/// column width that breaks the next time a path is longer.
fn render_index_pages(
    codes: &HashMap<String, String>,
    dims: &HashMap<String, (f64, f64)>,
    trim_allowance_mm: f64,
    metrics: &Metrics,
    title: &str,
) -> Vec<Page> {
    fn dim_str(length_mm: f64, width_mm: f64, trim_allowance_mm: f64) -> String {
        let final_s = format!("{} x {}", format_mm_in(length_mm), format_mm_in(width_mm));
        if trim_allowance_mm == 0.0 {
            return final_s;
        }
        let rough = format!(
            "{} x {}",
            format_mm_in(length_mm + trim_allowance_mm),
            format_mm_in(width_mm + trim_allowance_mm)
        );
        format!("{rough} -> {final_s}")
    }

    let mut rows: Vec<(String, String, String)> = codes
        .iter()
        .map(|(label, code)| {
            let (length_mm, width_mm) = dims[code];
            (
                code.clone(),
                label.clone(),
                dim_str(length_mm, width_mm, trim_allowance_mm),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let font_size_pt = 9.0;
    let code_w = rows
        .iter()
        .map(|r| metrics.width_mm(&r.0, font_size_pt, false))
        .fold(metrics.width_mm("Code", font_size_pt, false), f64::max)
        + 4.0;
    let dim_w = rows
        .iter()
        .map(|r| metrics.width_mm(&r.2, font_size_pt, false))
        .fold(
            metrics.width_mm("Dimensions", font_size_pt, false),
            f64::max,
        )
        + 4.0;
    let path_w = LETTER_H_MM - 2.0 * MARGIN_MM - code_w - dim_w;

    render_table_pages(&TableSpec {
        title: title.to_string(),
        headers: vec![
            "Code".to_string(),
            "Full Path".to_string(),
            "Dimensions".to_string(),
        ],
        widths: vec![code_w, path_w, dim_w],
        rows: rows
            .into_iter()
            .map(|(code, label, dims)| vec![code, label, dims])
            .collect(),
        font_size_pt,
        row_h_mm: 8.0,
        landscape: true,
        notes: false,
    })
}

/// A small right-angle bracket at the sheet's trusted corner -- the one
/// external fact the two-cut (rough + trim) scheme depends on: every
/// part's rough allowance extends away from here, inheriting squareness
/// down the guillotine cut tree rather than needing its own mark.
///
/// Deliberately just an icon, no caption -- explained once, elsewhere (not
/// on every sheet page), and small enough that a part legitimately placed
/// right at this corner (drawn on top, see `render_sheet_page`) won't have
/// its own label fought over by this mark.
fn mark_reference_corner(page: &mut Page, x_mm: f64, y_mm: f64, size_mm: f64) {
    page.set_stroke(200, 0, 0);
    page.set_line_width_mm(0.6);
    page.line(x_mm, y_mm, x_mm + size_mm, y_mm);
    page.line(x_mm, y_mm, x_mm, y_mm + size_mm);
    page.set_line_width_mm(0.2);
    page.set_stroke(0, 0, 0);
}

fn on_sheet_label(code: &str, leaf: &str, height_mm: f64, dims_line: &str) -> String {
    if height_mm < LABEL_MIN_HEIGHT_MM {
        code.to_string()
    } else {
        format!("{code} {leaf}\n{dims_line}")
    }
}

/// One dimension mark on the diagram: a tick at `position_mm` (the
/// cut's sheet-wide coordinate, for placing the tick on the page) paired
/// with the *size* to label it with -- the actual width or length of
/// whatever this cut separates off, never the raw cumulative
/// `Cut.position_mm` a mark sits at. See `dim_marks`'s own docs for why
/// the two aren't the same number.
struct DimMark {
    position_mm: f64,
    length_mm: f64,
}

/// The dimension marks worth drawing directly on a sheet's diagram for
/// one `kind` of cut: `Rip`s (marked along the left gutter, one per
/// strip) restricted to the primary, full-length ones that actually
/// carve the sheet into strips -- the "important key cutlines" -- as
/// opposed to the narrower rips a strip's own internal packing can also
/// produce; `Crosscut`s (marked along the bottom gutter) taken
/// unrestricted, since a crosscut has no sheet-wide "primary" cut the
/// way a strip-defining rip does. Every cut of either kind still gets
/// its own line in the Cut Instructions list regardless
/// (`render_cut_instructions_pages`).
///
/// `length_mm` is `CutStep.offset_mm` -- already a segment size, not a
/// cumulative position, see that field's own docs -- with `kerf_mm`
/// subtracted back out: `offset_mm` is `(rough size) + kerf_mm`, the
/// blade-width gap `pack()` reserved past this piece's own edge to keep
/// it clear of its neighbor (see `Cut`'s docs), never material this
/// piece actually keeps. Marking the raw, kerf-inflated number would
/// have a woodworker rip or crosscut every single piece on the sheet a
/// hair oversized.
fn dim_marks(sheet: &SheetLayout, kerf_mm: f64, kind: CutKind, primary_only: bool) -> Vec<DimMark> {
    cut_steps(sheet)
        .into_iter()
        .filter(|step| step.cut.kind == kind)
        .filter(|step| {
            if !primary_only {
                return true;
            }
            let full_span = match kind {
                CutKind::Rip => sheet.stock.length_mm,
                CutKind::Crosscut => sheet.stock.width_mm,
            };
            step.cut.span_end_mm - step.cut.span_start_mm >= full_span - 1e-3
        })
        .map(|step| DimMark {
            position_mm: step.cut.position_mm,
            length_mm: step.offset_mm - kerf_mm,
        })
        .collect()
}

/// Dimension ticks along the left gutter (one per primary rip, anchored
/// on the sheet's top edge at `origin_y`) or the bottom gutter (one per
/// crosscut, anchored on its bottom edge at `bottom_y`) reserved by
/// `render_sheet_page`: a tick on the sheet's own edge, projecting into
/// the gutter, labeled with that cut's own width/length (see
/// `DimMark`'s docs) -- never a position, so every label is a size you
/// can check a board against directly, independent of any other mark.
#[allow(clippy::too_many_arguments)] // plain PDF-drawing params, not worth a bespoke struct for one private helper
fn render_dim_marks(
    page: &mut Page,
    origin_x: f64,
    origin_y: f64,
    bottom_y: f64,
    scale: f64,
    axis: CutKind,
    marks: &[DimMark],
    metrics: &Metrics,
) {
    const FONT_PT: f64 = 6.0;
    page.set_stroke(90, 90, 90);
    page.set_line_width_mm(0.2);
    for mark in marks {
        let label = format_mm_in(mark.length_mm);
        match axis {
            CutKind::Rip => {
                let y = origin_y + mark.position_mm * scale;
                let tick_x = origin_x - 3.0;
                page.line(tick_x, y, origin_x, y);
                let label_w = metrics.width_mm(&label, FONT_PT, false);
                page.text(
                    tick_x - 1.0 - label_w,
                    y + FONT_PT * 0.3,
                    &label,
                    false,
                    FONT_PT,
                );
            }
            CutKind::Crosscut => {
                let x = origin_x + mark.position_mm * scale;
                let tick_y = bottom_y + 3.0;
                page.line(x, bottom_y, x, tick_y);
                let label_w = metrics.width_mm(&label, FONT_PT, false);
                page.text(x - label_w / 2.0, tick_y + 4.5, &label, false, FONT_PT);
            }
        }
    }
    page.set_stroke(0, 0, 0);
    page.set_line_width_mm(0.2);
}

fn render_sheet_page(
    sheet: &SheetLayout,
    codes: &HashMap<String, String>,
    kerf_mm: f64,
    trim_allowance_mm: f64,
    metrics: &Metrics,
) -> Page {
    let stock = &sheet.stock;
    let landscape = stock.length_mm >= stock.width_mm;
    let (page_w, page_h) = if landscape {
        (LETTER_H_MM, LETTER_W_MM)
    } else {
        (LETTER_W_MM, LETTER_H_MM)
    };
    let mut page = Page::new(page_w, page_h);

    let rip_marks = dim_marks(sheet, kerf_mm, CutKind::Rip, true);
    let crosscut_marks = dim_marks(sheet, kerf_mm, CutKind::Crosscut, false);
    let left_gutter = if rip_marks.is_empty() {
        0.0
    } else {
        DIM_GUTTER_MM
    };
    let bottom_gutter = if crosscut_marks.is_empty() {
        0.0
    } else {
        DIM_GUTTER_MM
    };

    let title_h = 12.0;
    let usable_w = page_w - 2.0 * MARGIN_MM - left_gutter;
    let usable_h = page_h - 2.0 * MARGIN_MM - title_h - bottom_gutter;
    let scale = (usable_w / stock.length_mm).min(usable_h / stock.width_mm);

    let origin_x = MARGIN_MM + left_gutter;
    let origin_y = MARGIN_MM + title_h;
    let bottom_y = origin_y + stock.width_mm * scale;

    let title = format!(
        "{} #{}  ({} x {} x {})",
        stock.material.name,
        sheet.sheet_index + 1,
        format_mm_in(stock.length_mm),
        format_mm_in(stock.width_mm),
        format_mm_in(stock.thickness_mm()),
    );
    page.text(MARGIN_MM, MARGIN_MM + 8.0 * 0.7, &title, true, 14.0);

    page.set_fill(238, 238, 238);
    page.set_stroke(153, 153, 153);
    page.rect(
        origin_x,
        origin_y,
        stock.length_mm * scale,
        stock.width_mm * scale,
        printpdf::PaintMode::FillStroke,
    );

    for placement in &sheet.placements {
        let x = origin_x + placement.x_mm * scale;
        let y = origin_y + placement.y_mm * scale;
        let final_w = placement.length_mm * scale;
        let final_h = placement.width_mm * scale;
        let code = &codes[&placement.part_label];
        let leaf_name = leaf(&placement.part_label);

        if trim_allowance_mm > 0.0 {
            let rough_w = (placement.length_mm + trim_allowance_mm) * scale;
            let rough_h = (placement.width_mm + trim_allowance_mm) * scale;

            page.set_fill(255, 255, 255);
            page.set_stroke(0, 0, 0);
            page.set_dash_mm(1.0, 1.0);
            page.rect(x, y, rough_w, rough_h, printpdf::PaintMode::FillStroke);
            page.clear_dash();

            page.set_stroke(0, 0, 0);
            page.rect(x, y, final_w, final_h, printpdf::PaintMode::Stroke);

            let dims_line = format!(
                "rough {} x {}\nfinal {} x {}",
                format_mm_in(placement.length_mm + trim_allowance_mm),
                format_mm_in(placement.width_mm + trim_allowance_mm),
                format_mm_in(placement.length_mm),
                format_mm_in(placement.width_mm),
            );
            let label = on_sheet_label(code, leaf_name, final_h, &dims_line);
            page.multiline(
                x + 1.0,
                y + 1.0,
                (final_w - 2.0).max(1.0),
                3.2,
                &label,
                7.0,
                metrics,
            );
        } else {
            page.set_fill(255, 255, 255);
            page.set_stroke(0, 0, 0);
            page.rect(x, y, final_w, final_h, printpdf::PaintMode::FillStroke);

            let dims_line = format!(
                "{} x {}",
                format_mm_in(placement.length_mm),
                format_mm_in(placement.width_mm)
            );
            let label = on_sheet_label(code, leaf_name, final_h, &dims_line);
            page.multiline(
                x + 1.0,
                y + 1.0,
                (final_w - 2.0).max(1.0),
                4.0,
                &label,
                8.0,
                metrics,
            );
        }
    }

    mark_reference_corner(&mut page, origin_x, origin_y, 5.0);
    render_dim_marks(
        &mut page,
        origin_x,
        origin_y,
        bottom_y,
        scale,
        CutKind::Rip,
        &rip_marks,
        metrics,
    );
    render_dim_marks(
        &mut page,
        origin_x,
        origin_y,
        bottom_y,
        scale,
        CutKind::Crosscut,
        &crosscut_marks,
        metrics,
    );
    page
}

/// A piece a `CutStep` refers to, as a woodworker would point at it:
/// the full sheet, a still-pending numbered piece, a finished part (by
/// its Parts Index code), or scrap nothing further happens to.
fn describe_source(source: &Source) -> String {
    match source {
        Source::Sheet => "the full sheet".to_string(),
        Source::Piece(id) => format!("piece {id}"),
    }
}

fn describe_outcome(outcome: &Outcome, codes: &HashMap<String, String>) -> String {
    match outcome {
        Outcome::Part(label) => codes.get(label).cloned().unwrap_or_else(|| label.clone()),
        Outcome::Piece(id) => format!("piece {id}"),
        Outcome::Offcut => "an unused offcut".to_string(),
    }
}

/// One line of a sheet's Cut Instructions table. Deliberately *not* an
/// absolute measurement from the sheet's reference corner -- past the
/// very first cut, that corner is gone from most of the pieces it once
/// applied to (see `crate::nesting::CutStep`'s own docs). Every
/// measurement here is instead the distance from the edge of whichever
/// piece (`step.source`) this particular cut actually lands on, and
/// every piece a step mentions is named clearly enough (a Parts Index
/// code, a numbered piece, or "an unused offcut") that following the
/// list start to finish never requires re-reading the diagram.
///
/// `kerf_mm` is subtracted from `step.offset_mm` before it's shown, same
/// as `DimMark.length_mm` on the diagram (see that type's own docs) --
/// the raw offset includes the blade-width gap this cut also reserves
/// past the piece's true edge, which isn't material the piece keeps.
fn describe_cut_step(step: &CutStep, kerf_mm: f64, codes: &HashMap<String, String>) -> String {
    let source = describe_source(&step.source);
    let near = describe_outcome(&step.near, codes);
    let far = describe_outcome(&step.far, codes);
    let offset = format_mm_in(step.offset_mm - kerf_mm);
    let axis = match step.cut.kind {
        CutKind::Rip => "width",
        CutKind::Crosscut => "length",
    };
    let verb = match step.cut.kind {
        CutKind::Rip => "Rip",
        CutKind::Crosscut => "Crosscut",
    };
    format!(
        "On {source}: {verb} {offset} from its reference-corner edge, across the {axis} -- this cuts off {near}, leaving {far}."
    )
}

/// A step-by-step breakdown of one sheet, each step named for the piece
/// it actually cuts (see `describe_cut_step`) rather than the sheet's
/// own corner -- so the whole sheet can be broken down from this list
/// alone, piece in hand, without needing a measurement the previous cut
/// already made physically impossible. Skips sheets with no cuts at all
/// (a single part that already is the whole sheet): there's nothing to
/// instruct.
fn render_cut_instructions_pages(
    sheet: &SheetLayout,
    kerf_mm: f64,
    codes: &HashMap<String, String>,
    metrics: &Metrics,
) -> Vec<Page> {
    let steps = cut_steps(sheet);
    if steps.is_empty() {
        return Vec::new();
    }
    let title = format!(
        "Cut Sequence -- {} #{}",
        sheet.stock.material.name,
        sheet.sheet_index + 1
    );
    let rows: Vec<Vec<String>> = steps
        .iter()
        .enumerate()
        .map(|(i, step)| vec![(i + 1).to_string(), describe_cut_step(step, kerf_mm, codes)])
        .collect();
    let step_w = rows
        .iter()
        .map(|r| metrics.width_mm(&r[0], 11.0, false))
        .fold(metrics.width_mm("Step", 11.0, false), f64::max)
        + 4.0;
    let landscape = sheet.stock.length_mm >= sheet.stock.width_mm;
    let page_w = if landscape { LETTER_H_MM } else { LETTER_W_MM };

    render_table_pages(&TableSpec {
        title,
        headers: vec!["Step".to_string(), "Cut".to_string()],
        widths: vec![step_w, page_w - 2.0 * MARGIN_MM - step_w],
        rows,
        font_size_pt: 11.0,
        row_h_mm: 8.0,
        landscape,
        notes: false,
    })
}

/// One PDF, organized for shop assembly (one construction stage at a
/// time) rather than as a single flat cutlist:
///
/// 1. A whole-project Bill of Materials -- the shopping list.
/// 2. Per construction-stage section (see `group_sheets_by_section`), in
///    build order: a front page (section title, that section's own BOM,
///    and a blank ruled Notes area -- these plans travel on a clipboard
///    in the shop, so there's always room to write on one), then for
///    each of that section's sheets, its own cut-sheet diagram
///    immediately followed by its own Cut Instructions page (see
///    `render_cut_instructions_pages`), and finally that section's own
///    Parts Index. Parts Index codes (P001, P002, ...) restart at P001
///    within each section rather than counting up across the whole
///    project, since a section's index only ever needs to
///    cross-reference that same section's own sheet pages -- assembly
///    happens one section at a time, so there's no reason to search a
///    global list.
///
/// No dedicated section title page: the front page already carries the
/// title alongside content worth the paper (its own BOM), so a
/// title-only page would just be a blank sheet with a heading on it.
///
/// `classify`/`unsectioned_label` are forwarded straight to
/// `group_sheets_by_section` -- see that function's docs for how a
/// sheet's section is decided.
///
/// `kerf_mm`/`trim_allowance_mm` must match whatever was passed to
/// `pack()` for this same `Layout` -- neither is derived from `Layout`
/// itself, both only ever *drawn* from here: `kerf_mm` is subtracted
/// back out of every dimension mark and Cut Instructions measurement
/// (see `DimMark`'s docs for why); `trim_allowance_mm`, when nonzero,
/// draws a second, dashed rough-cut outline around each placement
/// (final dims + trim_allowance_mm in each direction, extending from the
/// placement's own origin -- see `mark_reference_corner`) alongside the
/// solid final outline, and labels both.
pub fn render_pdf(
    layout: &Layout,
    kerf_mm: f64,
    trim_allowance_mm: f64,
    classify: impl Fn(&str) -> Option<String>,
    unsectioned_label: &str,
) -> Vec<u8> {
    let pages = build_pages(
        layout,
        kerf_mm,
        trim_allowance_mm,
        classify,
        unsectioned_label,
    );
    let mut doc = PdfDocument::new("Story Stick Cutlist");
    doc.with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut Vec::new())
}

/// `render_pdf`'s actual page-building, split out so tests can assert on
/// page *count* (and therefore document structure) directly, without
/// parsing rendered PDF bytes back apart.
fn build_pages(
    layout: &Layout,
    kerf_mm: f64,
    trim_allowance_mm: f64,
    classify: impl Fn(&str) -> Option<String>,
    unsectioned_label: &str,
) -> Vec<PdfPage> {
    let metrics = Metrics::new();
    let mut pages: Vec<PdfPage> = Vec::new();

    let global_bom = bill_of_materials(&layout.sheets);
    pages.extend(
        render_bom_table_pages("Bill of Materials", &global_bom, false)
            .into_iter()
            .map(Page::finish),
    );

    let sections = group_sheets_by_section(&layout.sheets, classify, unsectioned_label);
    for (label, sheets) in sections {
        let section_bom = bill_of_materials(sheets.iter().copied());
        pages.extend(
            render_bom_table_pages(&label, &section_bom, true)
                .into_iter()
                .map(Page::finish),
        );

        let (codes, dims) = assign_codes(sheets.iter().copied());
        for sheet in &sheets {
            pages.push(
                render_sheet_page(sheet, &codes, kerf_mm, trim_allowance_mm, &metrics).finish(),
            );
            pages.extend(
                render_cut_instructions_pages(sheet, kerf_mm, &codes, &metrics)
                    .into_iter()
                    .map(Page::finish),
            );
        }
        let index_title = format!("Parts Index -- {label}");
        pages.extend(
            render_index_pages(&codes, &dims, trim_allowance_mm, &metrics, &index_title)
                .into_iter()
                .map(Page::finish),
        );
    }

    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nesting::{pack, Cut, Material, PackablePart, Placement, StockSheet};

    fn test_stock() -> StockSheet {
        StockSheet {
            material: Material {
                name: "Baltic Birch 3/4".to_string(),
                thickness_mm: 19.05,
            },
            length_mm: 2438.4,
            width_mm: 1219.2,
        }
    }

    fn sheet_layout(placements: Vec<Placement>) -> SheetLayout {
        SheetLayout {
            stock: test_stock(),
            sheet_index: 0,
            placements,
            cuts: Vec::new(),
        }
    }

    fn placement(label: &str, x: f64, y: f64, length: f64, width: f64) -> Placement {
        Placement {
            part_label: label.to_string(),
            x_mm: x,
            y_mm: y,
            length_mm: length,
            width_mm: width,
            rotated: false,
        }
    }

    #[test]
    fn assign_codes_orders_by_first_appearance_across_sheets() {
        let layout = Layout {
            sheets: vec![
                sheet_layout(vec![
                    placement("Bench / Left / Body A", 0.0, 0.0, 100.0, 50.0),
                    placement("Bench / Left / Body B", 100.0, 0.0, 100.0, 50.0),
                ]),
                sheet_layout(vec![placement(
                    "Bench / Left / Body A",
                    0.0,
                    0.0,
                    100.0,
                    50.0,
                )]),
            ],
            unplaced: vec![],
        };
        let (codes, dims) = assign_codes(&layout.sheets);
        assert_eq!(codes["Bench / Left / Body A"], "P001");
        assert_eq!(codes["Bench / Left / Body B"], "P002");
        assert_eq!(dims["P001"], (100.0, 50.0));
    }

    #[test]
    fn on_sheet_label_falls_back_to_code_below_min_height() {
        assert_eq!(on_sheet_label("P001", "Body A", 5.0, "10\" x 5\""), "P001");
        assert_eq!(
            on_sheet_label("P001", "Body A", 20.0, "10\" x 5\""),
            "P001 Body A\n10\" x 5\""
        );
    }

    #[test]
    fn metrics_measures_wider_text_as_wider() {
        let metrics = Metrics::new();
        let short = metrics.width_mm("P001", 9.0, false);
        let long = metrics.width_mm("P001 Living Room Built-In / Left Console", 9.0, false);
        assert!(long > short);
        assert!(short > 0.0);
    }

    #[test]
    fn wrap_keeps_a_single_overlong_word_whole() {
        let metrics = Metrics::new();
        let lines = metrics.wrap("Supercalifragilisticexpialidocious", 5.0, 9.0);
        assert_eq!(
            lines,
            vec!["Supercalifragilisticexpialidocious".to_string()]
        );
    }

    #[test]
    fn group_sheets_by_section_groups_in_first_appearance_order() {
        let sheets = vec![
            sheet_layout(vec![placement(
                "Bench / Left Carcass / Bottom",
                0.0,
                0.0,
                100.0,
                50.0,
            )]),
            sheet_layout(vec![placement(
                "Bench / Left Door / Panel",
                0.0,
                0.0,
                100.0,
                50.0,
            )]),
            sheet_layout(vec![placement(
                "Bench / Right Carcass / Bottom",
                0.0,
                0.0,
                100.0,
                50.0,
            )]),
        ];
        let rules = [("Carcass", "Carcasses"), ("Door", "Doors")];
        let classify = |path: &str| crate::tags::classify_by_keyword(path, &rules);

        let grouped = group_sheets_by_section(&sheets, classify, "Unsectioned");

        let labels: Vec<&str> = grouped.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Carcasses", "Doors"],
            "Doors first appears after Carcasses, so it sorts second"
        );
        assert_eq!(
            grouped[0].1.len(),
            2,
            "both carcass sheets land in the Carcasses group"
        );
        assert_eq!(grouped[1].1.len(), 1);
    }

    #[test]
    fn group_sheets_by_section_falls_back_to_unsectioned_label() {
        let sheets = vec![sheet_layout(vec![placement(
            "Bench / Face Frame / Rail",
            0.0,
            0.0,
            100.0,
            50.0,
        )])];
        let rules = [("Carcass", "Carcasses")];
        let classify = |path: &str| crate::tags::classify_by_keyword(path, &rules);

        let grouped = group_sheets_by_section(&sheets, classify, "Unsectioned");

        assert_eq!(grouped, vec![("Unsectioned".to_string(), vec![&sheets[0]])]);
    }

    #[test]
    fn group_sheets_by_section_falls_back_for_a_sheet_with_no_placements() {
        let sheets = vec![sheet_layout(vec![])];
        let grouped =
            group_sheets_by_section(&sheets, |_| Some("Carcasses".to_string()), "Unsectioned");

        assert_eq!(grouped, vec![("Unsectioned".to_string(), vec![&sheets[0]])]);
    }

    #[test]
    fn group_sheets_by_section_uses_only_the_first_placements_section() {
        // A sheet mixing two sections' parts is a known, deliberately
        // unhandled edge case -- it's filed entirely under whichever
        // section its first placement belongs to, not split.
        let mixed = sheet_layout(vec![
            placement("Bench / Left Carcass / Bottom", 0.0, 0.0, 100.0, 50.0),
            placement("Bench / Left Door / Panel", 100.0, 0.0, 100.0, 50.0),
        ]);
        let rules = [("Carcass", "Carcasses"), ("Door", "Doors")];
        let classify = |path: &str| crate::tags::classify_by_keyword(path, &rules);

        let grouped =
            group_sheets_by_section(std::slice::from_ref(&mixed), classify, "Unsectioned");

        assert_eq!(grouped, vec![("Carcasses".to_string(), vec![&mixed])]);
    }

    #[test]
    fn render_pdf_produces_a_nonempty_pdf() {
        let layout = Layout {
            sheets: vec![sheet_layout(vec![placement(
                "Bench / Left / Body A",
                0.0,
                0.0,
                762.0,
                438.0,
            )])],
            unplaced: vec![],
        };
        let bytes = render_pdf(&layout, 0.0, 0.0, |_| None, "Unsectioned");
        assert!(bytes.starts_with(b"%PDF"));
        assert!(bytes.len() > 500);
    }

    #[test]
    fn build_pages_groups_pages_by_section_with_a_front_page_and_scoped_index_each() {
        let layout = Layout {
            sheets: vec![
                sheet_layout(vec![placement(
                    "Bench / Left Carcass / Bottom",
                    0.0,
                    0.0,
                    762.0,
                    438.0,
                )]),
                sheet_layout(vec![placement(
                    "Bench / Left Door / Panel",
                    0.0,
                    0.0,
                    762.0,
                    438.0,
                )]),
            ],
            unplaced: vec![],
        };
        let rules = [("Carcass", "Carcasses"), ("Door", "Doors")];
        let classify = |path: &str| crate::tags::classify_by_keyword(path, &rules);

        let pages = build_pages(&layout, 0.0, 0.0, classify, "Unsectioned");

        // 1 global BOM page, then per section (Carcasses, Doors): 1 front
        // page (title + BOM + notes) + 1 sheet page + 1 Parts Index page.
        assert_eq!(
            pages.len(),
            1 + 2 * 3,
            "global BOM + 2 sections x (front + sheet + index)"
        );
    }

    #[test]
    fn render_table_pages_notes_box_fits_on_the_same_page_when_rows_are_short() {
        let spec = TableSpec {
            title: "Section".to_string(),
            headers: vec!["Qty".to_string()],
            widths: vec![100.0],
            rows: vec![vec!["1".to_string()]],
            font_size_pt: 11.0,
            row_h_mm: 8.0,
            landscape: false,
            notes: true,
        };
        let pages = render_table_pages(&spec);
        assert_eq!(
            pages.len(),
            1,
            "a short table plus its notes box should fit on one page"
        );
    }

    #[test]
    fn render_table_pages_notes_box_opens_a_fresh_page_when_the_table_leaves_no_room() {
        // 26 one-line rows leaves under NOTES_MIN_HEIGHT_MM of portrait
        // page space below the table -- not enough to fit the notes box
        // it inline, without the table's own row loop needing to
        // paginate first.
        let spec = TableSpec {
            title: "Section".to_string(),
            headers: vec!["Qty".to_string()],
            widths: vec![100.0],
            rows: vec![vec!["1".to_string()]; 26],
            font_size_pt: 11.0,
            row_h_mm: 8.0,
            landscape: false,
            notes: true,
        };
        let pages = render_table_pages(&spec);
        assert_eq!(
            pages.len(),
            2,
            "the notes box should open its own page rather than squeeze in"
        );
    }

    #[test]
    fn describe_cut_step_measures_from_the_source_pieces_own_edge_not_the_sheet() {
        // A step whose source is a numbered piece (not Source::Sheet)
        // must never read as an absolute measurement from the sheet's
        // corner -- that corner isn't on this piece anymore.
        let step = CutStep {
            cut: Cut {
                kind: CutKind::Rip,
                position_mm: 600.0,
                span_start_mm: 0.0,
                span_end_mm: 900.0,
            },
            offset_mm: 150.0,
            source: Source::Piece(2),
            near: Outcome::Piece(3),
            far: Outcome::Offcut,
        };
        let description = describe_cut_step(&step, 0.0, &HashMap::new());
        assert!(description.starts_with("On piece 2:"));
        assert!(description.contains(&format!(
            "Rip {} from its reference-corner edge",
            format_mm_in(150.0)
        )));
        assert!(description.contains("cuts off piece 3"));
        assert!(description.contains("leaving an unused offcut"));
    }

    #[test]
    fn describe_cut_step_subtracts_kerf_from_the_reported_measurement() {
        // offset_mm already includes the blade-width gap this cut
        // reserves past the piece's true edge (see `DimMark`'s docs) --
        // the number shown must have that kerf subtracted back out.
        let step = CutStep {
            cut: Cut {
                kind: CutKind::Rip,
                position_mm: 600.0,
                span_start_mm: 0.0,
                span_end_mm: 900.0,
            },
            offset_mm: 150.0,
            source: Source::Sheet,
            near: Outcome::Piece(1),
            far: Outcome::Offcut,
        };
        let description = describe_cut_step(&step, 3.2, &HashMap::new());
        assert!(description.contains(&format_mm_in(150.0 - 3.2)));
        assert!(!description.contains(&format_mm_in(150.0)));
    }

    #[test]
    fn describe_cut_step_reports_a_finished_part_by_its_code() {
        let step = CutStep {
            cut: Cut {
                kind: CutKind::Crosscut,
                position_mm: 700.0,
                span_start_mm: 0.0,
                span_end_mm: 200.0,
            },
            offset_mm: 700.0,
            source: Source::Sheet,
            near: Outcome::Part("Bench / Body A".to_string()),
            far: Outcome::Piece(1),
        };
        let mut codes = HashMap::new();
        codes.insert("Bench / Body A".to_string(), "P001".to_string());
        let description = describe_cut_step(&step, 0.0, &codes);
        assert!(description.starts_with("On the full sheet:"));
        assert!(description.contains("cuts off P001"));
    }

    #[test]
    fn render_cut_instructions_pages_is_empty_for_a_sheet_with_no_cuts() {
        let sheet = sheet_layout(vec![placement("Bench / Body A", 0.0, 0.0, 762.0, 438.0)]);
        let metrics = Metrics::new();
        assert!(render_cut_instructions_pages(&sheet, 0.0, &HashMap::new(), &metrics).is_empty());
    }

    #[test]
    fn render_cut_instructions_pages_produces_a_page_for_a_sheet_with_cuts() {
        let mut sheet = sheet_layout(vec![placement("Bench / Body A", 0.0, 0.0, 762.0, 438.0)]);
        sheet.cuts = vec![Cut {
            kind: CutKind::Rip,
            position_mm: 438.0,
            span_start_mm: 0.0,
            span_end_mm: sheet.stock.length_mm,
        }];
        let metrics = Metrics::new();
        assert_eq!(
            render_cut_instructions_pages(&sheet, 0.0, &HashMap::new(), &metrics).len(),
            1
        );
    }

    fn two_strip_sheet_with_kerf() -> SheetLayout {
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
        layout.sheets[0].clone()
    }

    #[test]
    fn dim_marks_reports_each_rips_own_width_with_kerf_subtracted_out() {
        let sheet = two_strip_sheet_with_kerf();
        let marks = dim_marks(&sheet, 3.2, CutKind::Rip, true);
        let lengths: Vec<f64> = marks.iter().map(|m| m.length_mm).collect();
        assert_eq!(
            lengths,
            vec![200.0, 150.0],
            "a's strip is 200mm wide and b's is 150mm, neither inflated by the kerf gap past it"
        );
    }

    #[test]
    fn dim_marks_reports_each_crosscuts_own_length_and_isnt_limited_to_primary_spans() {
        let sheet = two_strip_sheet_with_kerf();
        let marks = dim_marks(&sheet, 3.2, CutKind::Crosscut, false);
        let lengths: Vec<f64> = marks.iter().map(|m| m.length_mm).collect();
        assert_eq!(
            lengths,
            vec![700.0, 400.0],
            "a is 700mm long and b is 400mm, matching their true lengths, not the sheet's cumulative position"
        );
    }

    #[test]
    fn build_pages_adds_one_instructions_page_per_sheet_that_has_cuts() {
        let mut with_cuts = sheet_layout(vec![placement(
            "Bench / Left / Body A",
            0.0,
            0.0,
            762.0,
            438.0,
        )]);
        with_cuts.cuts = vec![Cut {
            kind: CutKind::Rip,
            position_mm: 438.0,
            span_start_mm: 0.0,
            span_end_mm: with_cuts.stock.length_mm,
        }];
        let without_cuts = sheet_layout(vec![placement(
            "Bench / Left / Body B",
            0.0,
            0.0,
            762.0,
            438.0,
        )]);
        let layout = Layout {
            sheets: vec![with_cuts, without_cuts],
            unplaced: vec![],
        };

        let pages = build_pages(&layout, 0.0, 0.0, |_| None, "Unsectioned");

        // global BOM + front page + (sheet + instructions) + sheet (no
        // instructions) + index.
        assert_eq!(pages.len(), 1 + 1 + 2 + 1 + 1);
    }
}
