//! Material and length/width-swap overrides are the state you actually
//! create while reviewing a model -- geometry always comes fresh from the
//! STEP file, never hand-edited. So the only state that needs to survive
//! between runs is a `key -> overrides` map, saved as a sidecar next to
//! the STEP file (`model.step` -> `model.materials.yaml`). The key is
//! each part's `Part::assignment_key` -- its CAD path plus its own
//! dimensions (see that field's doc comment) -- not the bare path or a
//! row position, so a re-export that leaves a part's own path and
//! geometry unchanged keeps its overrides automatically, while a part
//! that's genuinely renamed, restructured, or resized loses them, which
//! is correct: as far as this map is concerned, that's a different part
//! now.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

/// Both fields default away and are omitted individually, so a part
/// you've only assigned a material to (the common case) serializes with
/// just a `material:` line, not an untouched `swapped:` one alongside it
/// -- and a part whose only override is the length/width swap (set
/// before you've picked a material) still gets saved at all, which a
/// `material.is_some()`-only save filter would silently drop.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

pub(crate) fn sidecar_path(step_path: &Path) -> PathBuf {
    step_path.with_extension("materials.yaml")
}

pub(crate) fn load(path: &Path) -> Result<BTreeMap<String, PartOverride>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(yaml_serde::from_str(&text)?)
}

pub(crate) fn save(overrides: &BTreeMap<String, PartOverride>, path: &Path) -> Result<(), Box<dyn Error>> {
    let text = yaml_serde::to_string(overrides)?;
    std::fs::write(path, text)?;
    Ok(())
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
