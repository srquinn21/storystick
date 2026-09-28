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

/// stock.yaml has two sections: `materials` (a name -> thickness catalog
/// -- see `Material`) and `sheets` (purchasable sizes, each naming which
/// material they're a sheet of). A sheet's thickness always comes from
/// its named material, never repeated per-sheet, so two sheet sizes of
/// the same material can't drift out of sync on thickness.
pub(crate) fn read(path: &Path) -> Result<Vec<StockSheet>, Box<dyn Error>> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "couldn't read stock catalog at {}: {e}\n(create one -- see scripts/stock.example.yaml -- or pass --stock <path>)",
            path.display()
        )
    })?;
    let doc: StockDoc = yaml_serde::from_str(&text)?;

    let materials: HashMap<String, Material> = doc
        .materials
        .into_iter()
        .map(|m| (m.name.clone(), Material { name: m.name, thickness_mm: m.thickness_in * MM_PER_IN }))
        .collect();

    let mut stock = Vec::new();
    for entry in doc.sheets {
        let material = materials
            .get(&entry.material)
            .ok_or_else(|| format!("stock.yaml: sheet references unknown material {:?}", entry.material))?;
        stock.push(StockSheet { material: material.clone(), length_mm: entry.length_in * MM_PER_IN, width_mm: entry.width_in * MM_PER_IN });
    }
    Ok(stock)
}
