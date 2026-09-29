//! The stock catalog: what sheet goods you can buy. Meant to be stable
//! across projects (a shop's materials don't change per model), so it
//! lives at a fixed config path by default rather than being passed on
//! every invocation -- see `default_path`.

use crate::MM_PER_IN;
use serde::Deserialize;
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use storystick_core::nesting::{Material, StockSheet};

#[derive(Debug, Deserialize)]
struct MaterialEntry {
    name: String,
    thickness_in: f64,
    /// Bracket tokens (e.g. `["[Panel]"]`) this material claims for
    /// autofill (see `crate::autofill`) -- a part whose path carries one
    /// of these gets this material seeded as its initial guess. Optional:
    /// most materials need no autofill entry at all.
    #[serde(default)]
    #[serde(rename = "match")]
    match_tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SheetEntry {
    material: String,
    length_in: f64,
    width_in: f64,
}

#[derive(Debug, Deserialize, Default)]
struct StockDoc {
    #[serde(default)]
    materials: Vec<MaterialEntry>,
    #[serde(default)]
    sheets: Vec<SheetEntry>,
}

/// `$XDG_CONFIG_HOME/storystick/stock.yaml`, falling back to
/// `~/.config/storystick/stock.yaml` -- a fixed, shop-wide location so you
/// don't pass `--stock` on every run. Override with `--stock` for the
/// rare project that needs its own catalog.
pub(crate) fn default_path() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    config_home.join("storystick").join("stock.yaml")
}

/// `stock::read`/`parse`'s result: the purchasable sheets (as before) plus
/// a tag -> material-name map built from every material's `match:` list,
/// for `crate::autofill` to seed an initial material guess from a part's
/// bracket tokens. Kept as one struct returned from one parse rather than
/// two separate lookups, so a caller can never hold a stock list and a
/// tag map that came from two different reads of the file drifting apart.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Catalog {
    pub(crate) stock: Vec<StockSheet>,
    pub(crate) tag_materials: HashMap<String, String>,
}

/// stock.yaml has two sections: `materials` (a name -> thickness catalog
/// -- see `Material`) and `sheets` (purchasable sizes, each naming which
/// material they're a sheet of). A sheet's thickness always comes from
/// its named material, never repeated per-sheet, so two sheet sizes of
/// the same material can't drift out of sync on thickness.
///
/// Pure parsing/validation, no file I/O -- kept separate from `read` so it
/// can be exercised directly with an in-memory string, the same way every
/// other loader in this codebase (`assignments::load`,
/// `stepcrawl::extract_parts`) separates the format from the disk access.
pub(crate) fn parse(text: &str) -> Result<Catalog, Box<dyn Error>> {
    let doc: StockDoc = yaml_serde::from_str(text)?;

    let mut materials: HashMap<String, Material> = HashMap::new();
    let mut tag_materials: HashMap<String, String> = HashMap::new();
    for m in doc.materials {
        for tag in &m.match_tags {
            if let Some(existing) = tag_materials.insert(tag.clone(), m.name.clone()) {
                return Err(format!("stock.yaml: tag {tag:?} is claimed by both {existing:?} and {:?}", m.name).into());
            }
        }
        materials.insert(m.name.clone(), Material { name: m.name, thickness_mm: m.thickness_in * MM_PER_IN });
    }

    let mut stock = Vec::new();
    for entry in doc.sheets {
        let material = materials
            .get(&entry.material)
            .ok_or_else(|| format!("stock.yaml: sheet references unknown material {:?}", entry.material))?;
        stock.push(StockSheet { material: material.clone(), length_mm: entry.length_in * MM_PER_IN, width_mm: entry.width_in * MM_PER_IN });
    }
    Ok(Catalog { stock, tag_materials })
}

pub(crate) fn read(path: &Path) -> Result<Catalog, Box<dyn Error>> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "couldn't read stock catalog at {}: {e}\n(create one -- see scripts/stock.example.yaml -- or pass --stock <path>)",
            path.display()
        )
    })?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_materials_and_sheets() {
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4 (finished 2 sides)"
    thickness_in: 0.75
  - name: "Baltic Birch 1/4"
    thickness_in: 0.25

sheets:
  - material: "Baltic Birch 3/4 (finished 2 sides)"
    length_in: 96
    width_in: 48
  - material: "Baltic Birch 1/4"
    length_in: 96
    width_in: 48
"#;
        let catalog = parse(yaml).unwrap();

        assert_eq!(catalog.stock.len(), 2);
        assert_eq!(catalog.stock[0].material.name, "Baltic Birch 3/4 (finished 2 sides)");
        assert!((catalog.stock[0].material.thickness_mm - 0.75 * MM_PER_IN).abs() < 1e-9);
        assert!((catalog.stock[0].length_mm - 96.0 * MM_PER_IN).abs() < 1e-9);
        assert!((catalog.stock[0].width_mm - 48.0 * MM_PER_IN).abs() < 1e-9);
    }

    #[test]
    fn parse_a_sheet_thickness_always_comes_from_its_material() {
        // Two sheet sizes of the same material must report the exact same
        // thickness -- there's no per-sheet thickness field to drift.
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4"
    thickness_in: 0.75

sheets:
  - material: "Baltic Birch 3/4"
    length_in: 96
    width_in: 48
  - material: "Baltic Birch 3/4"
    length_in: 60
    width_in: 30
"#;
        let catalog = parse(yaml).unwrap();
        assert_eq!(catalog.stock[0].material.thickness_mm, catalog.stock[1].material.thickness_mm);
    }

    #[test]
    fn parse_errs_when_a_sheet_references_an_unknown_material() {
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4"
    thickness_in: 0.75

sheets:
  - material: "Sande Ply 3/4"
    length_in: 96
    width_in: 48
"#;
        let err = parse(yaml).unwrap_err();
        assert!(err.to_string().contains("Sande Ply 3/4"), "error should name the unresolved material: {err}");
    }

    #[test]
    fn parse_empty_doc_yields_empty_stock() {
        let catalog = parse("").unwrap();
        assert!(catalog.stock.is_empty());
        assert!(catalog.tag_materials.is_empty());
    }

    #[test]
    fn parse_builds_a_tag_to_material_map_from_match_lists() {
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4 (finished 2 sides)"
    thickness_in: 0.75
    match: ["[Panel]", "[Door]"]
  - name: "Sande Ply 3/4 (utility)"
    thickness_in: 0.75
    match: ["[Backer]"]
  - name: "Baltic Birch 1/4"
    thickness_in: 0.25

sheets:
  - material: "Baltic Birch 3/4 (finished 2 sides)"
    length_in: 96
    width_in: 48
"#;
        let catalog = parse(yaml).unwrap();
        assert_eq!(catalog.tag_materials["[Panel]"], "Baltic Birch 3/4 (finished 2 sides)");
        assert_eq!(catalog.tag_materials["[Door]"], "Baltic Birch 3/4 (finished 2 sides)");
        assert_eq!(catalog.tag_materials["[Backer]"], "Sande Ply 3/4 (utility)");
        assert_eq!(catalog.tag_materials.len(), 3, "a material with no match: list contributes no tags");
    }

    #[test]
    fn parse_errs_when_two_materials_claim_the_same_tag() {
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4"
    thickness_in: 0.75
    match: ["[Panel]"]
  - name: "Sande Ply 3/4"
    thickness_in: 0.75
    match: ["[Panel]"]
"#;
        let err = parse(yaml).unwrap_err();
        assert!(err.to_string().contains("[Panel]"), "error should name the contested tag: {err}");
    }
}
