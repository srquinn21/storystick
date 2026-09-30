//! A part's own review-time exception: a material assigned directly to
//! this one part (overriding whatever its project-level bracket-tag rule
//! would otherwise apply -- see `crate::project`'s `autofill` map) and/or
//! its grain direction set explicitly. Geometry always comes fresh from
//! the STEP file, never hand-edited, so this -- keyed by `Part::
//! assignment_key`, a part's CAD path plus its own dimensions (see that
//! field's doc comment) -- is the only per-part state that needs to
//! survive between runs. It's not the *only* source of a part's material
//! though: a part with no exception here still gets a material if its
//! tag has a project-level rule (see `review::load_parts`'s resolution
//! order).

use serde::{Deserialize, Serialize};

/// Both fields default away and are omitted individually, so a part
/// you've only assigned a material to (the common case) serializes with
/// just a `material:` line, not an untouched `grain_along_length:` one
/// alongside it -- and a part whose only override is a by-design cross-
/// grain call still gets saved at all, which a `material.is_some()`-only
/// save filter would silently drop.
///
/// `material: None` means "no exception recorded for this part" -- not
/// "explicitly no material." Clearing a part's exception (in the review
/// TUI) removes it from this map entirely rather than recording a null;
/// what a cleared part's material then resolves to (a tag rule, or
/// plain unassigned) is `review::load_parts`'s job, not this type's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PartOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) material: Option<String>,
    /// Which of this part's two in-plane dimensions the grain runs
    /// along -- `true` for length (the common case, and the default), `false`
    /// for width, when that's how this piece was actually designed. This
    /// is independent of which dimension is *labeled* length vs width
    /// (always the longer one, never user-editable -- see `crate::review`'s
    /// module docs): a part can be labeled "Length: 12, Width: 24" and
    /// still have its grain running along the 24" edge, when that's the
    /// deliberate choice.
    #[serde(
        default = "default_grain_along_length",
        skip_serializing_if = "is_true"
    )]
    pub(crate) grain_along_length: bool,
}

impl Default for PartOverride {
    fn default() -> Self {
        Self {
            material: None,
            grain_along_length: true,
        }
    }
}

fn default_grain_along_length() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

impl PartOverride {
    pub(crate) fn is_empty(&self) -> bool {
        self.material.is_none() && self.grain_along_length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_only_override_omits_the_grain_line() {
        let over = PartOverride {
            material: Some("Baltic Birch 3/4".to_string()),
            grain_along_length: true,
        };
        let yaml = yaml_serde::to_string(&over).unwrap();
        assert_eq!(yaml.trim(), "material: Baltic Birch 3/4");
    }

    #[test]
    fn cross_grain_only_override_omits_the_material_line_and_is_not_empty() {
        let over = PartOverride {
            material: None,
            grain_along_length: false,
        };
        let yaml = yaml_serde::to_string(&over).unwrap();
        assert_eq!(yaml.trim(), "grain_along_length: false");
        assert!(
            !over.is_empty(),
            "a grain-only override must still be worth saving"
        );
    }

    #[test]
    fn untouched_override_is_empty_and_round_trips() {
        let over = PartOverride::default();
        assert!(over.is_empty());

        let with_both = PartOverride {
            material: Some("Sande Ply 3/4".to_string()),
            grain_along_length: false,
        };
        let yaml = yaml_serde::to_string(&with_both).unwrap();
        let back: PartOverride = yaml_serde::from_str(&yaml).unwrap();
        assert_eq!(back.material.as_deref(), Some("Sande Ply 3/4"));
        assert!(!back.grain_along_length);
    }
}
