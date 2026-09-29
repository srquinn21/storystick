//! Bracket-token material autofill: a part's Shapr3D path already carries
//! tags like `[Panel]`/`[Backer]` (see `storystick_core::tags`), and a
//! stock-catalog material can claim one or more of those tags as a
//! `match:` list (see `crate::stock`). This is the one function that
//! turns "this part's path has tag X" + "this catalog material claims tag
//! X" into a material name -- a starting point only, per docs/poc.md: the
//! caller decides whether to accept it (e.g. only if it's actually
//! thickness-compatible with the part), and the existing flag/correct
//! review workflow still catches anything it missed or got wrong.

use std::collections::HashMap;
use storystick_core::tags::extract_tags;

/// The first tag in `path` (left to right, matching `extract_tags`'
/// order) that `tag_materials` has an opinion on, resolved to that
/// material's name -- or `None` if `path` carries no tag any catalog
/// material claims. `tag_materials` maps one bracket tag (e.g.
/// `"[Panel]"`) to the single material name that claims it; building that
/// map (and rejecting a tag two materials both claim) is `crate::stock`'s
/// job, not this function's -- this only ever does the lookup.
pub(crate) fn guess_material(path: &str, tag_materials: &HashMap<String, String>) -> Option<String> {
    extract_tags(path).iter().find_map(|tag| tag_materials.get(tag).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_materials() -> HashMap<String, String> {
        HashMap::from([
            ("[Panel]".to_string(), "Baltic Birch 3/4 (finished 2 sides)".to_string()),
            ("[Backer]".to_string(), "Sande Ply 3/4 (utility)".to_string()),
        ])
    }

    #[test]
    fn guesses_the_material_a_tag_claims() {
        let path = "Bench / Carcasses / Carcass A / [Panel] Bottom";
        assert_eq!(guess_material(path, &tag_materials()), Some("Baltic Birch 3/4 (finished 2 sides)".to_string()));
    }

    #[test]
    fn no_guess_when_no_tag_in_the_path_is_claimed() {
        let path = "Bench / Carcasses / Carcass A / Bottom";
        assert_eq!(guess_material(path, &tag_materials()), None);
    }

    #[test]
    fn no_guess_for_a_tag_no_material_claims() {
        let path = "Bench / [Trim] Cove Molding";
        assert_eq!(guess_material(path, &tag_materials()), None);
    }

    #[test]
    fn picks_the_first_claimed_tag_when_a_path_carries_more_than_one() {
        // Left to right, same order extract_tags reports them in -- an
        // unusual path (a part nested under a folder whose own name also
        // carries a tag) still resolves deterministically rather than
        // picking whichever tag's material happens to iterate first out
        // of a HashMap.
        let path = "Bench / [Backer] Section / [Panel] Bottom";
        assert_eq!(guess_material(path, &tag_materials()), Some("Sande Ply 3/4 (utility)".to_string()));
    }
}
