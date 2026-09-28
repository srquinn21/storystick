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

use crate::nesting::{BomLine, Layout, SheetLayout};
use crate::units::format_mm_in;
use printpdf::{
    BuiltinFont, Color, Line, LineDashPattern, LinePoint, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Rect, Rgb, TextItem,
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

fn leaf(full_path: &str) -> &str {
    full_path.rsplit(" / ").next().unwrap_or(full_path)
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(Rgb { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, icc_profile: None })
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
            regular: BuiltinFont::Helvetica.get_parsed_font().expect("builtin Helvetica should always parse"),
            bold: BuiltinFont::HelveticaBold.get_parsed_font().expect("builtin Helvetica-Bold should always parse"),
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
            let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
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
        Self { width_mm, height_mm, ops: vec![Op::SetOutlineThickness { pt: mm(0.2).into_pt() }] }
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
        self.ops.push(Op::SetOutlineThickness { pt: mm(w_mm).into_pt() });
    }

    fn set_dash_mm(&mut self, dash_mm: f64, gap_mm: f64) {
        let dash_pt = mm(dash_mm).into_pt().0;
        let gap_pt = mm(gap_mm).into_pt().0;
        self.ops.push(Op::SetLineDashPattern { dash: LineDashPattern::new(0.0, &[dash_pt, gap_pt]) });
    }

    fn clear_dash(&mut self) {
        self.ops.push(Op::SetLineDashPattern { dash: LineDashPattern::solid() });
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
            line: Line { points: vec![LinePoint { p: p1, bezier: false }, LinePoint { p: p2, bezier: false }], is_closed: false },
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
            font: PdfFontHandle::Builtin(if bold { BuiltinFont::HelveticaBold } else { BuiltinFont::Helvetica }),
            size: printpdf::Pt(size_pt as f32),
        });
        self.ops.push(Op::SetTextCursor { pos: Point::new(mm(x_mm), mm(self.flip_y(y_mm))) });
        self.ops.push(Op::ShowText { items: vec![TextItem::Text(s.to_string())] });
        self.ops.push(Op::EndTextSection);
    }

    /// Draws `text` (which may contain explicit '\n's, each independently
    /// word-wrapped to `max_w_mm`) starting with its first baseline
    /// `line_h_mm * 0.7` below `y_mm`, one line per `line_h_mm` after that --
    /// mirroring fpdf2's `multi_cell` as used here (explicit newlines for
    /// layout, wrapping only as a width safety net).
    fn multiline(&mut self, x_mm: f64, y_mm: f64, max_w_mm: f64, line_h_mm: f64, text: &str, size_pt: f64, metrics: &Metrics) {
        let mut y = y_mm;
        for raw_line in text.split('\n') {
            for wrapped in metrics.wrap(raw_line, max_w_mm, size_pt) {
                self.text(x_mm, y + line_h_mm * 0.7, &wrapped, false, size_pt);
                y += line_h_mm;
            }
        }
    }
}

fn assign_codes(layout: &Layout) -> (HashMap<String, String>, HashMap<String, (f64, f64)>) {
    let mut codes: HashMap<String, String> = HashMap::new();
    let mut dims: HashMap<String, (f64, f64)> = HashMap::new();
    for sheet in &layout.sheets {
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

/// A titled table that starts a fresh page and paginates itself --
/// reprinting the header row -- if `rows` runs past the bottom margin. Used
/// for both the BOM (small, rarely paginates) and the Parts Index
/// (routinely hundreds of rows). Column widths are computed once, up
/// front, by the caller (never before the page's own orientation/width is
/// known -- see the Parts Index's dynamic widths in `render_index_pages`).
struct TableSpec {
    title: String,
    headers: Vec<String>,
    widths: Vec<f64>,
    rows: Vec<Vec<String>>,
    font_size_pt: f64,
    row_h_mm: f64,
    landscape: bool,
}

fn render_table_pages(spec: &TableSpec) -> Vec<Page> {
    let (page_w, page_h) = if spec.landscape { (LETTER_H_MM, LETTER_W_MM) } else { (LETTER_W_MM, LETTER_H_MM) };

    fn draw_row(page: &mut Page, x0: f64, y_top: f64, widths: &[f64], values: &[String], row_h: f64, font_size_pt: f64, bold: bool) {
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

    draw_row(&mut page, MARGIN_MM, y, &spec.widths, &spec.headers, spec.row_h_mm, spec.font_size_pt, true);
    y += spec.row_h_mm;

    if spec.rows.is_empty() {
        let total_w: f64 = spec.widths.iter().sum();
        draw_row(&mut page, MARGIN_MM, y, &[total_w], &["(none)".to_string()], spec.row_h_mm, spec.font_size_pt, false);
        pages.push(page);
        return pages;
    }

    let bottom = page_h - MARGIN_MM;
    for row in &spec.rows {
        if y + spec.row_h_mm > bottom {
            pages.push(page);
            page = Page::new(page_w, page_h);
            y = MARGIN_MM;
            draw_row(&mut page, MARGIN_MM, y, &spec.widths, &spec.headers, spec.row_h_mm, spec.font_size_pt, true);
            y += spec.row_h_mm;
        }
        draw_row(&mut page, MARGIN_MM, y, &spec.widths, row, spec.row_h_mm, spec.font_size_pt, false);
        y += spec.row_h_mm;
    }
    pages.push(page);
    pages
}

fn render_bom_pages(bom: &[BomLine]) -> Vec<Page> {
    let rows: Vec<Vec<String>> = bom
        .iter()
        .map(|line| {
            vec![
                line.qty.to_string(),
                line.stock.material.name.clone(),
                format_mm_in(line.stock.length_mm),
                format_mm_in(line.stock.width_mm),
                format_mm_in(line.stock.thickness_mm()),
            ]
        })
        .collect();

    render_table_pages(&TableSpec {
        title: "Bill of Materials".to_string(),
        headers: vec!["Qty".to_string(), "Label".to_string(), "Length".to_string(), "Width".to_string(), "Thickness".to_string()],
        widths: vec![20.0, 90.0, 25.0, 25.0, 25.0],
        rows,
        font_size_pt: 11.0,
        row_h_mm: 8.0,
        landscape: false,
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
) -> Vec<Page> {
    fn dim_str(length_mm: f64, width_mm: f64, trim_allowance_mm: f64) -> String {
        let final_s = format!("{} x {}", format_mm_in(length_mm), format_mm_in(width_mm));
        if trim_allowance_mm == 0.0 {
            return final_s;
        }
        let rough = format!("{} x {}", format_mm_in(length_mm + trim_allowance_mm), format_mm_in(width_mm + trim_allowance_mm));
        format!("{rough} -> {final_s}")
    }

    let mut rows: Vec<(String, String, String)> = codes
        .iter()
        .map(|(label, code)| {
            let (length_mm, width_mm) = dims[code];
            (code.clone(), label.clone(), dim_str(length_mm, width_mm, trim_allowance_mm))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let font_size_pt = 9.0;
    let code_w = rows.iter().map(|r| metrics.width_mm(&r.0, font_size_pt, false)).fold(metrics.width_mm("Code", font_size_pt, false), f64::max) + 4.0;
    let dim_w = rows.iter().map(|r| metrics.width_mm(&r.2, font_size_pt, false)).fold(metrics.width_mm("Dimensions", font_size_pt, false), f64::max) + 4.0;
    let path_w = LETTER_H_MM - 2.0 * MARGIN_MM - code_w - dim_w;

    render_table_pages(&TableSpec {
        title: "Parts Index".to_string(),
        headers: vec!["Code".to_string(), "Full Path".to_string(), "Dimensions".to_string()],
        widths: vec![code_w, path_w, dim_w],
        rows: rows.into_iter().map(|(code, label, dims)| vec![code, label, dims]).collect(),
        font_size_pt,
        row_h_mm: 8.0,
        landscape: true,
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

fn render_sheet_page(sheet: &SheetLayout, codes: &HashMap<String, String>, trim_allowance_mm: f64, metrics: &Metrics) -> Page {
    let stock = &sheet.stock;
    let landscape = stock.length_mm >= stock.width_mm;
    let (page_w, page_h) = if landscape { (LETTER_H_MM, LETTER_W_MM) } else { (LETTER_W_MM, LETTER_H_MM) };
    let mut page = Page::new(page_w, page_h);

    let title_h = 12.0;
    let usable_w = page_w - 2.0 * MARGIN_MM;
    let usable_h = page_h - 2.0 * MARGIN_MM - title_h;
    let scale = (usable_w / stock.length_mm).min(usable_h / stock.width_mm);

    let origin_x = MARGIN_MM;
    let origin_y = MARGIN_MM + title_h;

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
    page.rect(origin_x, origin_y, stock.length_mm * scale, stock.width_mm * scale, printpdf::PaintMode::FillStroke);

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
            page.multiline(x + 1.0, y + 1.0, (final_w - 2.0).max(1.0), 3.2, &label, 7.0, metrics);
        } else {
            page.set_fill(255, 255, 255);
            page.set_stroke(0, 0, 0);
            page.rect(x, y, final_w, final_h, printpdf::PaintMode::FillStroke);

            let dims_line = format!("{} x {}", format_mm_in(placement.length_mm), format_mm_in(placement.width_mm));
            let label = on_sheet_label(code, leaf_name, final_h, &dims_line);
            page.multiline(x + 1.0, y + 1.0, (final_w - 2.0).max(1.0), 4.0, &label, 8.0, metrics);
        }
    }

    mark_reference_corner(&mut page, origin_x, origin_y, 5.0);
    page
}

/// One PDF: a bill-of-materials page, one page per sheet with every cut on
/// it drawn to scale and labeled with a short code + its own dimensions,
/// and a closing Parts Index mapping each code back to the full CAD
/// assembly path it was cut from.
///
/// `trim_allowance_mm`, when nonzero, draws a second, dashed rough-cut
/// outline around each placement (final dims + trim_allowance_mm in each
/// direction, extending from the placement's own origin -- see
/// `mark_reference_corner`) alongside the solid final outline, and labels
/// both. Must match whatever trim_allowance_mm was passed to `pack()` for
/// this same Layout -- this only draws the rough outline, it doesn't
/// derive it from anything in `Layout` itself.
pub fn render_pdf(layout: &Layout, bom: &[BomLine], trim_allowance_mm: f64) -> Vec<u8> {
    let metrics = Metrics::new();
    let (codes, dims) = assign_codes(layout);

    let mut pages: Vec<PdfPage> = Vec::new();
    pages.extend(render_bom_pages(bom).into_iter().map(Page::finish));
    for sheet in &layout.sheets {
        pages.push(render_sheet_page(sheet, &codes, trim_allowance_mm, &metrics).finish());
    }
    pages.extend(render_index_pages(&codes, &dims, trim_allowance_mm, &metrics).into_iter().map(Page::finish));

    let mut doc = PdfDocument::new("Story Stick Cutlist");
    doc.with_pages(pages).save(&PdfSaveOptions::default(), &mut Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nesting::{Material, Placement, StockSheet};

    fn sheet_layout(placements: Vec<Placement>) -> SheetLayout {
        SheetLayout {
            stock: StockSheet {
                material: Material { name: "Baltic Birch 3/4".to_string(), thickness_mm: 19.05 },
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
            sheet_index: 0,
            placements,
        }
    }

    fn placement(label: &str, x: f64, y: f64, length: f64, width: f64) -> Placement {
        Placement { part_label: label.to_string(), x_mm: x, y_mm: y, length_mm: length, width_mm: width, rotated: false }
    }

    #[test]
    fn assign_codes_orders_by_first_appearance_across_sheets() {
        let layout = Layout {
            sheets: vec![
                sheet_layout(vec![placement("Bench / Left / Body A", 0.0, 0.0, 100.0, 50.0), placement("Bench / Left / Body B", 100.0, 0.0, 100.0, 50.0)]),
                sheet_layout(vec![placement("Bench / Left / Body A", 0.0, 0.0, 100.0, 50.0)]),
            ],
            unplaced: vec![],
        };
        let (codes, dims) = assign_codes(&layout);
        assert_eq!(codes["Bench / Left / Body A"], "P001");
        assert_eq!(codes["Bench / Left / Body B"], "P002");
        assert_eq!(dims["P001"], (100.0, 50.0));
    }

    #[test]
    fn on_sheet_label_falls_back_to_code_below_min_height() {
        assert_eq!(on_sheet_label("P001", "Body A", 5.0, "10\" x 5\""), "P001");
        assert_eq!(on_sheet_label("P001", "Body A", 20.0, "10\" x 5\""), "P001 Body A\n10\" x 5\"");
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
        assert_eq!(lines, vec!["Supercalifragilisticexpialidocious".to_string()]);
    }

    #[test]
    fn render_pdf_produces_a_nonempty_pdf() {
        let layout = Layout {
            sheets: vec![sheet_layout(vec![placement("Bench / Left / Body A", 0.0, 0.0, 762.0, 438.0)])],
            unplaced: vec![],
        };
        let bom = vec![BomLine {
            stock: StockSheet { material: Material { name: "Baltic Birch 3/4".to_string(), thickness_mm: 19.05 }, length_mm: 2438.4, width_mm: 1219.2 },
            qty: 1,
        }];
        let bytes = render_pdf(&layout, &bom, 0.0);
        assert!(bytes.starts_with(b"%PDF"));
        assert!(bytes.len() > 500);
    }
}
