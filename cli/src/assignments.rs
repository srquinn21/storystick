//! A part's own review-time exception: a material assigned directly to
//! this one part (overriding whatever its project-level bracket-tag rule
//! would otherwise apply -- see `crate::project`'s `autofill` map),
//! and/or its grain direction or dimension labeling set explicitly.
//! Geometry always comes fresh from the STEP file, never hand-edited, so
//! this -- keyed by `Part::assignment_key`, a part's CAD path plus its
//! own dimensions (see that field's doc comment) -- is the only per-part
//! state that needs to survive between runs. It's not the *only* source
//! of a part's material though: a part with no exception here still gets
//! a material if its tag has a project-level rule (see `review::
//! load_parts`'s resolution order).

use serde::{Deserialize, Serialize};

/// All three fields default away and are omitted individually, so a part
/// you've only assigned a material to (the common case) serializes with
/// just a `material:` line, not untouched `grain_along_length:`/
/// `dimensions:` lines alongside it -- and a part whose only override is
/// a by-design cross-grain call, or a corrected dimension labeling, still
/// gets saved at all, which a `material.is_some()`-only save filter would
/// silently drop.
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
    /// (see `dimensions`/`crate::review`'s module docs): a part can be
    /// labeled "Length: 12, Width: 24" and still have its grain running
    /// along the 24" edge, when that's the deliberate choice.
    #[serde(
        default = "default_grain_along_length",
        skip_serializing_if = "is_true"
    )]
    pub(crate) grain_along_length: bool,
    /// Manual correction of which raw measured value fills which of
    /// length/width/thickness, for when stepcrawl's own largest/middle/
    /// smallest guess is simply wrong for this part -- see
    /// `DimensionAssignment`. Omitted entirely while it's still
    /// `DimensionAssignment::AS_GUESSED`, the overwhelmingly common case.
    #[serde(default, skip_serializing_if = "DimensionAssignment::is_as_guessed")]
    pub(crate) dimensions: DimensionAssignment,
}

impl Default for PartOverride {
    fn default() -> Self {
        Self {
            material: None,
            grain_along_length: true,
            dimensions: DimensionAssignment::AS_GUESSED,
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
        self.material.is_none() && self.grain_along_length && self.dimensions.is_as_guessed()
    }
}

/// One raw measured value from stepcrawl's own largest/middle/smallest
/// guess (`Part::raw_length_in`/`raw_width_in`/`raw_thickness_in`) --
/// never a measurement in its own right, just a name for one of the
/// three numbers already sitting in that guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RawAxis {
    Length,
    Width,
    Thickness,
}

/// Which raw measured value (see `RawAxis`) fills each of a part's three
/// display roles -- length, width, thickness. Exists purely to correct
/// stepcrawl's largest/middle/smallest guess when it's simply wrong for a
/// given part -- e.g. a part measures 0.5 x 0.75 x 24, and the 0.75" edge
/// is actually the thickness even though it isn't the smallest number --
/// the three raw *numbers* are never edited, only which role each one is
/// assigned to.
///
/// `AS_GUESSED` (the default) takes stepcrawl's guess as-is, in which
/// case `review::resolve_dims` is still free to auto-correct it against a
/// known material's thickness (the narrow-rip case). Any other assignment
/// is an explicit human decision instead, applied exactly as given with
/// no further second-guessing -- seeing a wrong three-way guess and
/// deciding to trust a *different* automatic guess in its place would
/// defeat the point of a manual override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct DimensionAssignment {
    pub(crate) length_from: RawAxis,
    pub(crate) width_from: RawAxis,
    pub(crate) thickness_from: RawAxis,
}

impl DimensionAssignment {
    pub(crate) const AS_GUESSED: Self = Self {
        length_from: RawAxis::Length,
        width_from: RawAxis::Width,
        thickness_from: RawAxis::Thickness,
    };

    /// Every way to assign the three raw values to the three roles, in a
    /// fixed cycle starting from `AS_GUESSED` -- what `next` steps
    /// through one keypress at a time, so correcting a mislabeled part is
    /// a matter of stepping past each candidate until the displayed
    /// numbers look right, never editing a number directly.
    const ALL: [DimensionAssignment; 6] = [
        Self::AS_GUESSED,
        Self {
            length_from: RawAxis::Width,
            width_from: RawAxis::Length,
            thickness_from: RawAxis::Thickness,
        },
        Self {
            length_from: RawAxis::Thickness,
            width_from: RawAxis::Width,
            thickness_from: RawAxis::Length,
        },
        Self {
            length_from: RawAxis::Length,
            width_from: RawAxis::Thickness,
            thickness_from: RawAxis::Width,
        },
        Self {
            length_from: RawAxis::Thickness,
            width_from: RawAxis::Length,
            thickness_from: RawAxis::Width,
        },
        Self {
            length_from: RawAxis::Width,
            width_from: RawAxis::Thickness,
            thickness_from: RawAxis::Length,
        },
    ];

    pub(crate) fn is_as_guessed(&self) -> bool {
        *self == Self::AS_GUESSED
    }

    /// The next assignment in `ALL`'s cycle, wrapping back to
    /// `AS_GUESSED` after the last one.
    pub(crate) fn next(&self) -> Self {
        let i = Self::ALL.iter().position(|a| a == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    fn pick(axis: RawAxis, raw: (f64, f64, f64)) -> f64 {
        match axis {
            RawAxis::Length => raw.0,
            RawAxis::Width => raw.1,
            RawAxis::Thickness => raw.2,
        }
    }

    /// Applies this assignment to a part's raw (length, width, thickness)
    /// guess, producing the (length, width, thickness) triple it actually
    /// means.
    pub(crate) fn apply(&self, raw: (f64, f64, f64)) -> (f64, f64, f64) {
        (
            Self::pick(self.length_from, raw),
            Self::pick(self.width_from, raw),
            Self::pick(self.thickness_from, raw),
        )
    }
}

impl Default for DimensionAssignment {
    fn default() -> Self {
        Self::AS_GUESSED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_only_override_omits_the_grain_and_dimensions_lines() {
        let over = PartOverride {
            material: Some("Baltic Birch 3/4".to_string()),
            grain_along_length: true,
            dimensions: DimensionAssignment::AS_GUESSED,
        };
        let yaml = yaml_serde::to_string(&over).unwrap();
        assert_eq!(yaml.trim(), "material: Baltic Birch 3/4");
    }

    #[test]
    fn cross_grain_only_override_omits_the_material_line_and_is_not_empty() {
        let over = PartOverride {
            material: None,
            grain_along_length: false,
            dimensions: DimensionAssignment::AS_GUESSED,
        };
        let yaml = yaml_serde::to_string(&over).unwrap();
        assert_eq!(yaml.trim(), "grain_along_length: false");
        assert!(
            !over.is_empty(),
            "a grain-only override must still be worth saving"
        );
    }

    #[test]
    fn dimensions_only_override_omits_the_other_lines_and_is_not_empty() {
        let over = PartOverride {
            material: None,
            grain_along_length: true,
            dimensions: DimensionAssignment::AS_GUESSED.next(),
        };
        let yaml = yaml_serde::to_string(&over).unwrap();
        assert_eq!(
            yaml.trim(),
            "dimensions:\n  length_from: width\n  width_from: length\n  thickness_from: thickness"
        );
        assert!(
            !over.is_empty(),
            "a dimensions-only override must still be worth saving"
        );
    }

    #[test]
    fn untouched_override_is_empty_and_round_trips() {
        let over = PartOverride::default();
        assert!(over.is_empty());

        let with_all = PartOverride {
            material: Some("Sande Ply 3/4".to_string()),
            grain_along_length: false,
            dimensions: DimensionAssignment::AS_GUESSED.next(),
        };
        let yaml = yaml_serde::to_string(&with_all).unwrap();
        let back: PartOverride = yaml_serde::from_str(&yaml).unwrap();
        assert_eq!(back.material.as_deref(), Some("Sande Ply 3/4"));
        assert!(!back.grain_along_length);
        assert_eq!(back.dimensions, DimensionAssignment::AS_GUESSED.next());
    }

    #[test]
    fn dimension_assignment_cycle_visits_every_permutation_once_and_returns_to_as_guessed() {
        let mut seen = std::collections::HashSet::new();
        let mut current = DimensionAssignment::AS_GUESSED;
        for _ in 0..6 {
            assert!(seen.insert(current), "cycle repeated {current:?} early");
            current = current.next();
        }
        assert_eq!(
            current,
            DimensionAssignment::AS_GUESSED,
            "cycle must return to AS_GUESSED after all 6 permutations"
        );
    }

    #[test]
    fn dimension_assignment_apply_picks_the_assigned_raw_value_for_each_role() {
        let raw = (24.0, 0.75, 18.0);
        let assignment = DimensionAssignment {
            length_from: RawAxis::Thickness,
            width_from: RawAxis::Length,
            thickness_from: RawAxis::Width,
        };
        assert_eq!(assignment.apply(raw), (18.0, 24.0, 0.75));
    }
}
