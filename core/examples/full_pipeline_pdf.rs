//! Throwaway verification tool: STEP file -> extract_parts -> pack ->
//! render_pdf, writing a real cutlist.pdf for visual comparison against the
//! Python reference on real project data.
use std::path::PathBuf;
use storystick_core::diagrams::render_pdf;
use storystick_core::nesting::{pack, Material, PackablePart, StockSheet};

const MM_PER_IN: f64 = 25.4;

fn round_trip_in(mm: f64) -> f64 {
    let rounded_in = (mm / MM_PER_IN * 10000.0).round() / 10000.0;
    rounded_in * MM_PER_IN
}

fn main() {
    let step_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: full_pipeline_pdf <step-file> <out.pdf>"),
    );
    let out_path = std::env::args()
        .nth(2)
        .expect("usage: full_pipeline_pdf <step-file> <out.pdf>");
    let groups =
        storystick_core::stepcrawl::extract_parts(&step_path).expect("failed to parse STEP file");

    let parts: Vec<PackablePart> = groups
        .iter()
        .map(|g| {
            let mut p = PackablePart::new(
                g.top_folder.clone(),
                round_trip_in(g.length_mm),
                round_trip_in(g.width_mm),
                round_trip_in(g.thickness_mm),
            );
            p.qty = g.qty() as u32;
            p
        })
        .collect();

    let bb34 = Material {
        name: "Baltic Birch 3/4 (finished 2 sides)".to_string(),
        thickness_mm: 0.75 * MM_PER_IN,
    };
    let sande34 = Material {
        name: "Sande Ply 3/4 (utility, unseen parts)".to_string(),
        thickness_mm: 0.75 * MM_PER_IN,
    };
    let bb14 = Material {
        name: "Baltic Birch 1/4".to_string(),
        thickness_mm: 0.25 * MM_PER_IN,
    };
    let stock = vec![
        StockSheet {
            material: bb34,
            length_mm: 96.0 * MM_PER_IN,
            width_mm: 48.0 * MM_PER_IN,
        },
        StockSheet {
            material: sande34,
            length_mm: 96.0 * MM_PER_IN,
            width_mm: 48.0 * MM_PER_IN,
        },
        StockSheet {
            material: bb14,
            length_mm: 96.0 * MM_PER_IN,
            width_mm: 48.0 * MM_PER_IN,
        },
    ];

    let kerf_mm = 0.125 * MM_PER_IN;
    let layout = pack(&parts, &stock, kerf_mm, 0.0);

    let pdf_bytes = render_pdf(&layout, 0.0, |_path: &str| None, "Unsectioned");
    std::fs::write(&out_path, &pdf_bytes).expect("failed to write PDF");
    println!(
        "wrote {} ({} bytes), {} sheets, {} unplaced",
        out_path,
        pdf_bytes.len(),
        layout.sheets.len(),
        layout.unplaced.len()
    );
}
