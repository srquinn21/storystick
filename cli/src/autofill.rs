//! Bracket-token material autofill: a part's Shapr3D path already carries
//! tags like `[Panel]`/`[Backer]` (see `storystick_core::tags`), and a
//! project's `storystick.yaml` can map one or more of those tags to a
//! material name (`crate::project::Project::autofill`, authored almost
//! entirely through bulk-edit -- see `review`'s module docs). This is the
//! one function that turns "this part's path has tag X" + "this
//! project's rules say X means material Y" into a material name --
//! `review::resolve_material` decides how this interacts with a part's
//! own exception, not this function.

use std::collections::BTreeMap;
use storystick_core::tags::extract_tags;

/// The first tag in `path` (left to right, matching `extract_tags`'
/// order) that `autofill_map` has a rule for, resolved to that rule's
/// material name -- or `None` if `path` carries no tag with a configured
/// rule. Building/editing `autofill_map` (and what a tag with no rule at
/// all means) is `crate::project`/`review`'s job, not this function's --
/// this only ever does the lookup.
pub(crate) fn guess_material(
    path: &str,
    autofill_map: &BTreeMap<String, String>,
) -> Option<String> {
    extract_tags(path)
        .iter()
        .find_map(|tag| autofill_map.get(tag).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn autofill_map() -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "[Panel]".to_string(),
                "Baltic Birch 3/4 (finished 2 sides)".to_string(),
            ),
            (
                "[Backer]".to_string(),
                "Sande Ply 3/4 (utility)".to_string(),
            ),
        ])
    }

    #[test]
    fn guesses_the_material_a_tag_claims() {
        let path = "Bench / Carcasses / Carcass A / [Panel] Bottom";
        assert_eq!(
            guess_material(path, &autofill_map()),
            Some("Baltic Birch 3/4 (finished 2 sides)".to_string())
        );
    }

    #[test]
    fn no_guess_when_no_tag_in_the_path_is_claimed() {
        let path = "Bench / Carcasses / Carcass A / Bottom";
        assert_eq!(guess_material(path, &autofill_map()), None);
    }

    #[test]
    fn no_guess_for_a_tag_no_rule_claims() {
        let path = "Bench / [Trim] Cove Molding";
        assert_eq!(guess_material(path, &autofill_map()), None);
    }

    #[test]
    fn picks_the_first_ruled_tag_when_a_path_carries_more_than_one() {
        // Left to right, same order extract_tags reports them in -- an
        // unusual path (a part nested under a folder whose own name also
        // carries a tag) still resolves deterministically.
        let path = "Bench / [Backer] Section / [Panel] Bottom";
        assert_eq!(
            guess_material(path, &autofill_map()),
            Some("Sande Ply 3/4 (utility)".to_string())
        );
    }
}
