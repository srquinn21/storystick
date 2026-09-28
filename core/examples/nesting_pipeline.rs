//! Throwaway verification tool: STEP file -> extract_parts -> pack ->
//! bill_of_materials, printed in the same shape as the Python CLI's
//! `storystick cutlist --bom-csv` output, for diffing against the
//! known-good Python reference on real project data.
use std::path::PathBuf;
use storystick_core::nesting::{bill_of_materials, pack, Material, PackablePart, StockSheet};

const MM_PER_IN: f64 = 25.4;

fn main() {
    let step_path = PathBuf::from(std::env::args().nth(1).expect("usage: nesting_pipeline <step-file>"));
    let groups = storystick_core::stepcrawl::extract_parts(&step_path).expect("failed to parse STEP file");

    // Mirror the real pipeline: dimensions round-trip through parts.csv as
    // inches rounded to 4 decimals before being converted back to mm for
    // packing, which is what makes exact-float thickness bucketing line up
    // with stock.yaml's thickness_mm (also derived from a decimal inches
    // value). Feeding raw, full-precision STEP-measured floats straight
    // into pack() would defeat that and was a bug in this verification
    // harness, not in pack() itself.
    fn round_trip_in(mm: f64) -> f64 {
        let rounded_in = (mm / MM_PER_IN * 10000.0).round() / 10000.0;
        rounded_in * MM_PER_IN
    }

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

    // Mirrors scripts/stock.example.yaml.
    let bb34 = Material { name: "Baltic Birch 3/4 (finished 2 sides)".to_string(), thickness_mm: 0.75 * MM_PER_IN };
    let sande34 = Material { name: "Sande Ply 3/4 (utility, unseen parts)".to_string(), thickness_mm: 0.75 * MM_PER_IN };
    let bb14 = Material { name: "Baltic Birch 1/4".to_string(), thickness_mm: 0.25 * MM_PER_IN };
    let stock = vec![
        StockSheet { material: bb34, length_mm: 96.0 * MM_PER_IN, width_mm: 48.0 * MM_PER_IN },
        StockSheet { material: sande34, length_mm: 96.0 * MM_PER_IN, width_mm: 48.0 * MM_PER_IN },
        StockSheet { material: bb14, length_mm: 96.0 * MM_PER_IN, width_mm: 48.0 * MM_PER_IN },
    ];

    let kerf_mm = 0.125 * MM_PER_IN;
    let layout = pack(&parts, &stock, kerf_mm, 0.0);
    let bom = bill_of_materials(&layout);

    println!("material,length_in,width_in,thickness_in,qty");
    for line in &bom {
        println!(
            "{},{},{},{},{}",
            line.stock.material.name,
            line.stock.length_mm / MM_PER_IN,
            line.stock.width_mm / MM_PER_IN,
            line.stock.thickness_mm() / MM_PER_IN,
            line.qty,
        );
    }

    if !layout.unplaced.is_empty() {
        eprintln!("UNPLACED: {} parts", layout.unplaced.len());
        for p in &layout.unplaced {
            eprintln!("  {} {}x{}x{}", p.label, p.length_mm, p.width_mm, p.thickness_mm);
        }
    }
}
