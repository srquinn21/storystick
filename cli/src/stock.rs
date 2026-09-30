//! The stock catalog: what sheet goods you can buy. Meant to be stable
//! across projects (a shop's materials don't change per model), so it
//! lives at a fixed config path by default rather than being passed on
//! every invocation -- see `default_path`. Which of these materials a
//! given project actually uses, and which bracket tokens auto-assign to
//! which of them, are project-level concerns (see `crate::project`), not
//! this catalog's -- this module only ever answers "what could I buy."

use crate::MM_PER_IN;
use serde::Deserialize;
use std::error::Error;
use std::path::{Path, PathBuf};
use storystick_core::nesting::{Material, StockSheet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SheetSizeEntry {
    length_in: f64,
    width_in: f64,
}

/// One buyable material: a name, its thickness, and every size it's sold
/// in. Sheet sizes are nested here rather than cross-referenced by name
/// from a separate top-level list, so a sheet can't name the wrong
/// material (or a material that's been renamed or removed since) --
/// there's no name to get wrong, and a material's thickness is typed in
/// exactly one place no matter how many sizes it's sold in.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialEntry {
    name: String,
    thickness_in: f64,
    #[serde(default)]
    sheets: Vec<SheetSizeEntry>,
}

/// `deny_unknown_fields` here (not just diagnostic elsewhere) so a
/// leftover top-level `sheets:` from the pre-nesting format fails loudly
/// at parse time instead of silently being ignored -- which would
/// otherwise leave every material with zero sheets and no stock at all.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StockDoc {
    #[serde(default)]
    materials: Vec<MaterialEntry>,
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

/// stock.yaml is one list, `materials`: each entry names a material, its
/// thickness, and every size it's sold in (`sheets`, nested -- see
/// `MaterialEntry`). There's no cross-list reference to resolve or get
/// wrong.
///
/// Pure parsing/validation, no file I/O -- kept separate from `read` so it
/// can be exercised directly with an in-memory string, the same way every
/// other loader in this codebase (`project::load`, `stepcrawl::extract_parts`)
/// separates the format from the disk access.
pub(crate) fn parse(text: &str) -> Result<Vec<StockSheet>, Box<dyn Error>> {
    let doc: StockDoc = yaml_serde::from_str(text)?;

    let mut stock = Vec::new();
    for entry in doc.materials {
        let material = Material {
            name: entry.name,
            thickness_mm: entry.thickness_in * MM_PER_IN,
        };
        for sheet in entry.sheets {
            stock.push(StockSheet {
                material: material.clone(),
                length_mm: sheet.length_in * MM_PER_IN,
                width_mm: sheet.width_in * MM_PER_IN,
            });
        }
    }
    Ok(stock)
}

pub(crate) fn read(path: &Path) -> Result<Vec<StockSheet>, Box<dyn Error>> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "couldn't read stock catalog at {}: {e}\n(create one -- see scripts/stock.example.yaml -- or pass --stock <path>)",
            path.display()
        )
    })?;
    parse(&text)
}

/// Every distinct material named across `stock`'s sheets, deduped by
/// name, first-appearance order -- the whole shop catalog's materials,
/// with sheet-size variety collapsed away. A project's own material
/// subset (`crate::project::Project::resolve_materials`) filters this
/// down further to just what that project actually uses.
pub(crate) fn distinct_materials(stock: &[StockSheet]) -> Vec<Material> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for sheet in stock {
        if seen.insert(sheet.material.name.clone()) {
            out.push(sheet.material.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_materials_with_their_nested_sheets() {
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4 (finished 2 sides)"
    thickness_in: 0.75
    sheets:
      - length_in: 96
        width_in: 48
  - name: "Baltic Birch 1/4"
    thickness_in: 0.25
    sheets:
      - length_in: 96
        width_in: 48
"#;
        let stock = parse(yaml).unwrap();

        assert_eq!(stock.len(), 2);
        assert_eq!(
            stock[0].material.name,
            "Baltic Birch 3/4 (finished 2 sides)"
        );
        assert!((stock[0].material.thickness_mm - 0.75 * MM_PER_IN).abs() < 1e-9);
        assert!((stock[0].length_mm - 96.0 * MM_PER_IN).abs() < 1e-9);
        assert!((stock[0].width_mm - 48.0 * MM_PER_IN).abs() < 1e-9);
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
      - length_in: 96
        width_in: 48
      - length_in: 60
        width_in: 30
"#;
        let stock = parse(yaml).unwrap();
        assert_eq!(stock.len(), 2);
        assert_eq!(
            stock[0].material.thickness_mm,
            stock[1].material.thickness_mm
        );
    }

    #[test]
    fn parse_a_material_with_no_sheets_yields_no_stock_for_it() {
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4"
    thickness_in: 0.75
"#;
        assert!(parse(yaml).unwrap().is_empty());
    }

    #[test]
    fn parse_errs_on_the_pre_nesting_top_level_sheets_format() {
        // A leftover top-level `sheets:` list from before sheets nested
        // under their material must fail loudly, not silently parse as
        // zero stock for every material (which `#[serde(default)]` alone
        // would do, since the field would just go unrecognized).
        let yaml = r#"
materials:
  - name: "Baltic Birch 3/4"
    thickness_in: 0.75

sheets:
  - material: "Baltic Birch 3/4"
    length_in: 96
    width_in: 48
"#;
        let err = parse(yaml).unwrap_err();
        assert!(
            err.to_string().contains("sheets"),
            "error should name the stray top-level field: {err}"
        );
    }

    #[test]
    fn parse_empty_doc_yields_empty_stock() {
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn distinct_materials_dedupes_by_name_preserving_first_appearance_order() {
        let bb34 = Material {
            name: "Baltic Birch 3/4".to_string(),
            thickness_mm: 19.05,
        };
        let sande34 = Material {
            name: "Sande Ply 3/4".to_string(),
            thickness_mm: 19.05,
        };
        let stock = vec![
            StockSheet {
                material: bb34.clone(),
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
            StockSheet {
                material: sande34,
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
            StockSheet {
                material: bb34,
                length_mm: 1219.2,
                width_mm: 609.6,
            },
        ];
        let distinct = distinct_materials(&stock);
        let names: Vec<&str> = distinct.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["Baltic Birch 3/4", "Sande Ply 3/4"]);
    }
}
