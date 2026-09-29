//! Construction-stage classification for the cutlist PDF's per-section
//! grouping (see docs/poc.md's "folder-name-based PDF section grouping").
//! A part's section is read off its own CAD folder naming -- "Left
//! Carcass" -> Carcasses, "Left Door" -> Doors, and so on -- the same
//! "keyword found somewhere in a path" mechanism bracket-token material
//! autofill uses, just matching plain words instead of `[Bracket]`
//! tokens (see `storystick_core::tags::classify_by_keyword`).
//!
//! This rule list is this user's own Shapr3D naming discipline, not a
//! generic catalog -- so unlike the stock catalog, it's a fixed list
//! here rather than something loaded from a config file. Order matters:
//! the first rule whose keyword appears anywhere in a path wins (e.g. a
//! "Face Frame" folder must be listed ahead of a bare "Frame" rule, if
//! one ever existed, or it'd never be reached).

use storystick_core::tags::classify_by_keyword;

const RULES: &[(&str, &str)] = &[
    ("Carcass", "Carcasses"),
    ("Door", "Doors"),
    ("Face Frame", "Face Frames"),
    ("Drawer", "Drawers"),
];

/// The section a sheet with no placements, or whose first placement's
/// path matches no rule, is filed under -- see
/// `storystick_core::diagrams::group_sheets_by_section`.
pub(crate) const UNSECTIONED: &str = "Unsectioned";

pub(crate) fn classify(path: &str) -> Option<String> {
    classify_by_keyword(path, RULES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_each_known_construction_stage() {
        assert_eq!(classify("Bench / Left Carcass / Bottom"), Some("Carcasses".to_string()));
        assert_eq!(classify("Bench / Left Door / Panel"), Some("Doors".to_string()));
        assert_eq!(classify("Bench / Face Frame / Rail"), Some("Face Frames".to_string()));
        assert_eq!(classify("Bench / Top Drawer / Front"), Some("Drawers".to_string()));
    }

    #[test]
    fn returns_none_for_a_path_matching_no_rule() {
        assert_eq!(classify("Bench / Hardware / Hinge Block"), None);
    }
}
