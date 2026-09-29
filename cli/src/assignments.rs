//! A part's own review-time exception: a material assigned directly to
//! this one part (overriding whatever its project-level bracket-tag rule
//! would otherwise apply -- see `crate::project`'s `autofill` map) and/or
//! its length/width swapped. Geometry always comes fresh from the STEP
//! file, never hand-edited, so this -- keyed by `Part::assignment_key`,
//! a part's CAD path plus its own dimensions (see that field's doc
//! comment) -- is the only per-part state that needs to survive between
//! runs. It's not the *only* source of a part's material though: a part
//! with no exception here still gets a material if its tag has a
//! project-level rule (see `review::load_parts`'s resolution order).

use serde::{Deserialize, Serialize};

/// Both fields default away and are omitted individually, so a part
/// you've only assigned a material to (the common case) serializes with
/// just a `material:` line, not an untouched `swapped:` one alongside it
/// -- and a part whose only override is the length/width swap still gets
/// saved at all, which a `material.is_some()`-only save filter would
/// silently drop.
///
/// `material: None` means "no exception recorded for this part" -- not
/// "explicitly no material." Clearing a part's exception (in the review
/// TUI) removes it from this map entirely rather than recording a null;
/// what a cleared part's material then resolves to (a tag rule, or
/// plain unassigned) is `review::load_parts`'s job, not this type's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PartOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) material: Option<String>,
    /// Grain always runs with a part's length (see `crate::review`'s
    /// module docs); this says whether the part's length/width, as
    /// stepcrawl guessed them (longer of the two in-plane dimensions =
    /// length), have been swapped so the *other* edge runs with the
    /// grain instead.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) swapped: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl PartOverride {
    pub(crate) fn is_empty(&self) -> bool {
        self.material.is_none() && !self.swapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_only_override_omits_the_swapped_line() {
        let over = PartOverride { material: Some("Baltic Birch 3/4".to_string()), swapped: false };
        let yaml = yaml_serde::to_string(&over).unwrap();
        assert_eq!(yaml.trim(), "material: Baltic Birch 3/4");
    }

    #[test]
    fn swap_only_override_omits_the_material_line_and_is_not_empty() {
        let over = PartOverride { material: None, swapped: true };
        let yaml = yaml_serde::to_string(&over).unwrap();
        assert_eq!(yaml.trim(), "swapped: true");
        assert!(!over.is_empty(), "a swap-only override must still be worth saving");
    }

    #[test]
    fn untouched_override_is_empty_and_round_trips() {
        let over = PartOverride::default();
        assert!(over.is_empty());

        let with_both = PartOverride { material: Some("Sande Ply 3/4".to_string()), swapped: true };
        let yaml = yaml_serde::to_string(&with_both).unwrap();
        let back: PartOverride = yaml_serde::from_str(&yaml).unwrap();
        assert_eq!(back.material.as_deref(), Some("Sande Ply 3/4"));
        assert!(back.swapped);
    }
}
