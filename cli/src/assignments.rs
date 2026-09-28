//! Material assignments are the one thing you actually create while
//! reviewing a model -- geometry always comes fresh from the STEP file,
//! never hand-edited. So the only state that needs to survive between
//! runs is a `key -> material name` map, saved as a sidecar next to the
//! STEP file (`model.step` -> `model.materials.yaml`). The key is each
//! part's `Part::assignment_key` -- its CAD path plus its own dimensions
//! (see that field's doc comment) -- not the bare path or a row position,
//! so a re-export that leaves a part's own path and geometry unchanged
//! keeps its assignment automatically, while a part that's genuinely
//! renamed, restructured, or resized loses it, which is correct: as far
//! as this map is concerned, that's a different part now.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

pub(crate) fn sidecar_path(step_path: &Path) -> PathBuf {
    step_path.with_extension("materials.yaml")
}

pub(crate) fn load(path: &Path) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(path)?;
    Ok(yaml_serde::from_str(&text)?)
}

pub(crate) fn save(assignments: &BTreeMap<String, String>, path: &Path) -> Result<(), Box<dyn Error>> {
    let text = yaml_serde::to_string(assignments)?;
    std::fs::write(path, text)?;
    Ok(())
}
