//! STEP -> parts list.
//!
//! Parses a Shapr3D STEP export into a quantity-grouped list of parts,
//! each labeled with its assembly folder path. This module does exactly
//! one job -- it knows nothing about stock sheets, nesting, or rendering.
//!
//! This is a lightweight, dependency-free STEP reader, not a real CAD
//! kernel: it treats the file as a graph of `#id = ENTITY(args...)`
//! records and walks each MANIFOLD_SOLID_BREP down through its topology
//! to collect that body's own vertex points (bounding box = part
//! dimensions), then walks the NEXT_ASSEMBLY_USAGE_OCCURRENCE chain
//! upward to reconstruct the Shapr3D folder tree it lives in. See
//! `geometry` and `assembly` for the details.
//!
//! Length units are assumed to be millimeters, matching a typical Shapr3D
//! STEP export; this reader never checks a file's declared SI_UNIT.

mod assembly;
mod entities;
mod geometry;
mod parts;

pub use parts::{
    off_grid, off_grid_default, relabel_with_known_thickness, with_known_thickness,
    with_known_thickness_default, OffGrid, PartGroup, PartInstance, DEFAULT_GRID_IN,
    DEFAULT_KNOWN_THICKNESS_TOLERANCE_MM, DEFAULT_OFF_GRID_TOLERANCE_IN,
};

use assembly::build_indices;
use entities::{parse_entities, typed, unquote};
use geometry::{bbox, collect_points};
use parts::group_parts;
use std::path::Path;

/// Parse a Shapr3D STEP export into grouped, quantity-counted parts.
pub fn extract_parts(step_path: &Path) -> std::io::Result<Vec<PartGroup>> {
    let entities = parse_entities(step_path)?;
    let (breps_by_container, index) = build_indices(&entities);

    let mut rows: Vec<(String, String, f64, f64, f64, bool)> = Vec::new();
    for brep in &breps_by_container {
        let pd_id = index.container_pd(brep.rep_id);
        let prefix: Vec<String> = match pd_id {
            Some(pd) => index.ancestor_path(pd),
            None => vec![brep.container_name.clone()],
        };
        for &solid_id in &brep.solid_ids {
            let (typ, sargs) = typed(&entities, solid_id);
            if typ.as_deref() != Some("MANIFOLD_SOLID_BREP") {
                continue;
            }
            let body_name = unquote(&sargs[0]);
            let (points, unreliable) = collect_points(&entities, solid_id);
            if points.is_empty() {
                continue;
            }
            let (dx, dy, dz) = bbox(&points);
            let path_str = prefix.join(" / ");
            rows.push((path_str, body_name, dx, dy, dz, unreliable));
        }
    }

    rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    Ok(group_parts(&rows))
}
