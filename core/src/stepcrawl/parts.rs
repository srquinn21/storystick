//! Parts: grouping raw solid-body measurements into quantity-counted
//! PartGroups, and the two dimension-relabeling QC helpers (off_grid,
//! with_known_thickness). Pure data logic -- no STEP-specific parsing
//! here.

use std::collections::HashMap;

pub const MM_PER_IN: f64 = 25.4;
pub const DEFAULT_GRID_IN: f64 = 1.0 / 16.0;
pub const DEFAULT_OFF_GRID_TOLERANCE_IN: f64 = 0.005;
pub const DEFAULT_KNOWN_THICKNESS_TOLERANCE_MM: f64 = 1.0;

/// One physical body contributing to a PartGroup.
#[derive(Debug, Clone, PartialEq)]
pub struct PartInstance {
    pub path: String,
    pub unreliable: bool,
}

/// A set of interchangeable parts: same top-level folder, same L/W/T.
///
/// length_mm/width_mm/thickness_mm are a guess (largest/middle/smallest of
/// the body's three bounding-box dimensions) -- there's no "thickness
/// axis" in a STEP file, this is a woodworking convention layered on
/// after the fact. See `with_known_thickness` for correcting a guess that
/// picked a narrow rip's width as if it were the stock's thickness.
#[derive(Debug, Clone, PartialEq)]
pub struct PartGroup {
    pub top_folder: String,
    pub length_mm: f64,
    pub width_mm: f64,
    pub thickness_mm: f64,
    pub instances: Vec<PartInstance>,
}

impl PartGroup {
    pub fn qty(&self) -> usize {
        self.instances.len()
    }

    pub fn unreliable(&self) -> bool {
        self.instances.iter().any(|i| i.unreliable)
    }
}

/// Signed deviation (inches) from the nearest grid increment, per
/// dimension -- only set for a dimension that exceeds tolerance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OffGrid {
    pub length_in: Option<f64>,
    pub width_in: Option<f64>,
    pub thickness_in: Option<f64>,
}

impl OffGrid {
    pub fn is_flagged(&self) -> bool {
        self.length_in.is_some() || self.width_in.is_some() || self.thickness_in.is_some()
    }
}

/// Round to 3 decimal places of an inch, represented as an integer
/// (thousandths) so it's usable as an exact HashMap key -- f64 isn't
/// Eq/Hash, and this is exactly equivalent to Python's `round(x, 3)` used
/// as a dict-key component, without any floating-point comparison risk.
fn round3_key(value_in: f64) -> i64 {
    (value_in * 1000.0).round() as i64
}

type GroupKey = (String, (i64, i64, i64));
/// One grouped body's own (instance_path, l_mm, w_mm, t_mm, unreliable).
type GroupedRow = (String, f64, f64, f64, bool);

/// rows: (path, body_name, dx_mm, dy_mm, dz_mm, unreliable) -- one row per
/// solid body, `path` being its full assembly-folder prefix (root product
/// name first, as returned by `ancestor_path`).
///
/// Groups bodies into PartGroups by (top-level folder, L/W/T rounded to
/// 0.001") -- two panels count as "the same part, qty 2" if they agree to
/// the nearest thousandth of an inch, regardless of tiny STEP
/// floating-point drift. Grouping is scoped to each body's top-level
/// folder rather than globally, so mirrored parts with identical
/// dimensions (Left Carcass / Right Carcass) stay separate line items
/// instead of merging into one "qty 2" -- they live in different
/// top-level assemblies and get built/installed as distinct pieces.
///
/// Caller is responsible for row order (e.g. sort by (path, name)) if a
/// deterministic PartGroup order matters.
pub fn group_parts(rows: &[(String, String, f64, f64, f64, bool)]) -> Vec<PartGroup> {
    let mut groups: HashMap<GroupKey, Vec<GroupedRow>> = HashMap::new();
    let mut order: Vec<GroupKey> = Vec::new();

    for (path, name, dx, dy, dz, unreliable) in rows {
        let segments: Vec<&str> = path.split(" / ").collect();
        let top_folder = if segments.len() > 1 {
            segments[1]
        } else {
            segments[0]
        }
        .to_string();

        let mut dims = [*dx, *dy, *dz];
        dims.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let (l_mm, w_mm, t_mm) = (dims[0], dims[1], dims[2]);

        let key = (
            round3_key(l_mm / MM_PER_IN),
            round3_key(w_mm / MM_PER_IN),
            round3_key(t_mm / MM_PER_IN),
        );

        let path_segments: Vec<&str> = if segments.len() > 1 {
            segments[1..].to_vec()
        } else {
            segments.clone()
        };
        let mut instance_parts = path_segments;
        instance_parts.push(name.as_str());
        let instance_path = instance_parts.join(" / ");

        let group_key: GroupKey = (top_folder, key);
        if !groups.contains_key(&group_key) {
            groups.insert(group_key.clone(), Vec::new());
            order.push(group_key.clone());
        }
        groups
            .get_mut(&group_key)
            .unwrap()
            .push((instance_path, l_mm, w_mm, t_mm, *unreliable));
    }

    order
        .iter()
        .map(|group_key| {
            let items = &groups[group_key];
            let (top_folder, _) = group_key;
            let (_, l_mm, w_mm, t_mm, _) = &items[0];
            let instances = items
                .iter()
                .map(|(p, _, _, _, u)| PartInstance {
                    path: p.clone(),
                    unreliable: *u,
                })
                .collect();
            PartGroup {
                top_folder: top_folder.clone(),
                length_mm: *l_mm,
                width_mm: *w_mm,
                thickness_mm: *t_mm,
                instances,
            }
        })
        .collect()
}

/// How far off the nearest `grid_in` increment each of part's dimensions
/// falls, in inches -- only reported past `tolerance_in`.
pub fn off_grid(part: &PartGroup, grid_in: f64, tolerance_in: f64) -> OffGrid {
    let delta = |value_mm: f64| -> Option<f64> {
        let value_in = value_mm / MM_PER_IN;
        let nearest = (value_in / grid_in).round() * grid_in;
        let d = value_in - nearest;
        if d.abs() > tolerance_in {
            Some(d)
        } else {
            None
        }
    };
    OffGrid {
        length_in: delta(part.length_mm),
        width_in: delta(part.width_mm),
        thickness_in: delta(part.thickness_mm),
    }
}

pub fn off_grid_default(part: &PartGroup) -> OffGrid {
    off_grid(part, DEFAULT_GRID_IN, DEFAULT_OFF_GRID_TOLERANCE_IN)
}

/// Given three measured dimensions and a known material thickness (e.g.
/// from an explicit material assignment), returns them relabeled as
/// (length_mm, width_mm, thickness_mm): whichever is closest to
/// `thickness_mm` becomes thickness, and the other two are sorted into
/// length/width. Corrects the default largest/middle/smallest guess for a
/// piece ripped narrower than it is thick. Returns Err if no dimension is
/// within `tolerance_mm` -- a real signal, not just a missed correction:
/// it means the assigned material doesn't actually match this part's
/// geometry at all.
pub fn relabel_with_known_thickness(
    dims_mm: (f64, f64, f64),
    thickness_mm: f64,
    tolerance_mm: f64,
) -> Result<(f64, f64, f64), String> {
    let dims = [dims_mm.0, dims_mm.1, dims_mm.2];
    let idx = (0..3usize)
        .min_by(|&a, &b| {
            (dims[a] - thickness_mm)
                .abs()
                .partial_cmp(&(dims[b] - thickness_mm).abs())
                .unwrap()
        })
        .unwrap();
    if (dims[idx] - thickness_mm).abs() > tolerance_mm {
        return Err(format!(
            "no dimension ({:.2} / {:.2} / {:.2} mm) is within {tolerance_mm} mm of known thickness {thickness_mm} mm",
            dims[0], dims[1], dims[2]
        ));
    }
    let thickness = dims[idx];
    let mut remaining: Vec<f64> = (0..3).filter(|&i| i != idx).map(|i| dims[i]).collect();
    remaining.sort_by(|a, b| b.partial_cmp(a).unwrap());
    Ok((remaining[0], remaining[1], thickness))
}

/// Relabel `part`'s dimensions given a known material thickness -- see
/// `relabel_with_known_thickness`.
pub fn with_known_thickness(
    part: &PartGroup,
    thickness_mm: f64,
    tolerance_mm: f64,
) -> Result<PartGroup, String> {
    let (length_mm, width_mm, thickness_mm) = relabel_with_known_thickness(
        (part.length_mm, part.width_mm, part.thickness_mm),
        thickness_mm,
        tolerance_mm,
    )
    .map_err(|e| format!("{} part: {e}", part.top_folder))?;
    Ok(PartGroup {
        top_folder: part.top_folder.clone(),
        length_mm,
        width_mm,
        thickness_mm,
        instances: part.instances.clone(),
    })
}

pub fn with_known_thickness_default(
    part: &PartGroup,
    thickness_mm: f64,
) -> Result<PartGroup, String> {
    with_known_thickness(part, thickness_mm, DEFAULT_KNOWN_THICKNESS_TOLERANCE_MM)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inch(v: f64) -> f64 {
        v * MM_PER_IN
    }

    fn row(
        path: &str,
        name: &str,
        dx: f64,
        dy: f64,
        dz: f64,
        unreliable: bool,
    ) -> (String, String, f64, f64, f64, bool) {
        (path.to_string(), name.to_string(), dx, dy, dz, unreliable)
    }

    #[test]
    fn group_parts_counts_identical_dimensions_as_one_group() {
        let rows = vec![
            row(
                "Root / Bench / Carcasses / Carcass A",
                "[Panel] Bottom",
                inch(30.125),
                inch(16.0),
                inch(0.75),
                false,
            ),
            row(
                "Root / Bench / Carcasses / Carcass B",
                "[Panel] Bottom",
                inch(30.125),
                inch(16.0),
                inch(0.75),
                false,
            ),
        ];

        let groups = group_parts(&rows);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].qty(), 2);
        assert_eq!(groups[0].top_folder, "Bench");
    }

    #[test]
    fn group_parts_keeps_mirrored_parts_in_different_top_folders_separate() {
        let rows = vec![
            row(
                "Root / Console A / Carcasses / Carcass A",
                "[Panel] Left",
                inch(29.75),
                inch(16.0),
                inch(0.75),
                false,
            ),
            row(
                "Root / Console B / Carcasses / Carcass B",
                "[Panel] Right",
                inch(29.75),
                inch(16.0),
                inch(0.75),
                false,
            ),
        ];

        let groups = group_parts(&rows);

        assert_eq!(
            groups.len(),
            2,
            "identical dims in different top-level folders must stay separate line items"
        );
    }

    #[test]
    fn group_parts_tolerates_floating_point_drift() {
        let rows = vec![
            row(
                "Root / Bench / Carcass A",
                "[Backer]",
                inch(31.0) + 1e-6,
                inch(17.0),
                inch(0.25),
                false,
            ),
            row(
                "Root / Bench / Carcass B",
                "[Backer]",
                inch(31.0) - 1e-6,
                inch(17.0),
                inch(0.25),
                false,
            ),
        ];

        let groups = group_parts(&rows);

        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn group_parts_instance_path_drops_root_but_keeps_top_folder() {
        let rows = vec![row(
            "Root Product / Bench / Carcasses / Carcass A",
            "[Backer]",
            inch(31.0),
            inch(17.0),
            inch(0.25),
            false,
        )];

        let groups = group_parts(&rows);

        assert_eq!(
            groups[0].instances[0].path,
            "Bench / Carcasses / Carcass A / [Backer]"
        );
    }

    #[test]
    fn off_grid_flags_dimension_past_tolerance() {
        let part = PartGroup {
            top_folder: "Bench".to_string(),
            length_mm: inch(30.0 + 0.02), // ~0.02" off a 1/16" increment
            width_mm: inch(16.0),
            thickness_mm: inch(0.75),
            instances: vec![PartInstance {
                path: "Bench / X".to_string(),
                unreliable: false,
            }],
        };

        let result = off_grid(&part, DEFAULT_GRID_IN, 0.005);

        assert!(result.length_in.is_some());
        assert!(result.width_in.is_none());
        assert!(result.thickness_in.is_none());
        assert!(result.is_flagged());
    }

    #[test]
    fn off_grid_silent_within_tolerance() {
        let part = PartGroup {
            top_folder: "Bench".to_string(),
            length_mm: inch(30.0),
            width_mm: inch(16.0),
            thickness_mm: inch(0.75),
            instances: vec![PartInstance {
                path: "Bench / X".to_string(),
                unreliable: false,
            }],
        };

        let result = off_grid_default(&part);

        assert!(!result.is_flagged());
    }

    #[test]
    fn with_known_thickness_corrects_a_narrow_rip() {
        // Ripped from 3/4" stock down to a 1/4" wide strip: naive largest/
        // middle/smallest guessing calls 0.25" the thickness.
        let misguessed = PartGroup {
            top_folder: "Bench".to_string(),
            length_mm: inch(24.0),
            width_mm: inch(0.75),
            thickness_mm: inch(0.25),
            instances: vec![PartInstance {
                path: "Bench / X".to_string(),
                unreliable: false,
            }],
        };

        let corrected = with_known_thickness(&misguessed, inch(0.75), 1.0).unwrap();

        assert!((corrected.thickness_mm - inch(0.75)).abs() < 1e-9);
        assert!((corrected.width_mm - inch(0.25)).abs() < 1e-9);
        assert!((corrected.length_mm - inch(24.0)).abs() < 1e-9);
        assert_eq!(corrected.instances, misguessed.instances);
    }

    #[test]
    fn with_known_thickness_errs_when_nothing_matches() {
        let part = PartGroup {
            top_folder: "Bench".to_string(),
            length_mm: inch(24.0),
            width_mm: inch(16.0),
            thickness_mm: inch(0.75),
            instances: vec![PartInstance {
                path: "Bench / X".to_string(),
                unreliable: false,
            }],
        };

        assert!(with_known_thickness(&part, inch(0.25), 1.0).is_err());
    }
}
