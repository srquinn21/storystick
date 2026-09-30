//! `storystick.yaml`: the one file that owns everything specific to a
//! single project -- which STEP export to review, this project's own
//! subset of the global stock catalog (`crate::stock`, by material name
//! only; thickness/sheet sizes always come fresh from there), bracket-tag
//! material rules (`autofill`, authored almost entirely through
//! bulk-edit, see `review`'s module docs), kerf/trim/output settings, and
//! per-part assignment exceptions (`crate::assignments`).
//!
//! Discovered by walking up from the current directory, git-style (see
//! `discover`) -- not passed on the command line. A directory with no
//! `storystick.yaml` anywhere above it triggers `crate::wizard` instead
//! of failing outright.

use crate::assignments::PartOverride;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use storystick_core::nesting::{Material, StockSheet};

pub(crate) const FILENAME: &str = "storystick.yaml";

fn default_kerf_in() -> f64 {
    1.0 / 8.0
}

fn default_out_pdf() -> String {
    "cutlist.pdf".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Settings {
    #[serde(default = "default_kerf_in")]
    pub(crate) kerf_in: f64,
    #[serde(default)]
    pub(crate) trim_allowance_in: f64,
    #[serde(default = "default_out_pdf")]
    pub(crate) out_pdf: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            kerf_in: default_kerf_in(),
            trim_allowance_in: 0.0,
            out_pdf: default_out_pdf(),
        }
    }
}

/// One project's full, persisted state. `step`/`settings.out_pdf` are
/// always relative to the directory this file itself lives in (see
/// `step_path`/`out_pdf_path`), never to the current working directory --
/// `discover` can find this file from a subdirectory, so cwd-relative
/// paths would break the moment you're not sitting right next to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Project {
    pub(crate) step: String,
    /// This project's own subset of the global stock catalog, by
    /// material name -- not a copy of the catalog entries themselves, so
    /// a shop-wide thickness correction or sheet-size change is never
    /// something a project file could drift out of sync on. See
    /// `resolve_materials`.
    #[serde(default)]
    pub(crate) materials: Vec<String>,
    /// Bracket tag -> material name. The tag must appear somewhere in a
    /// part's path for this to apply (see `storystick_core::tags`); the
    /// material name must resolve within `materials` above. Authored
    /// almost entirely via bulk-edit (`review`'s `b` key), not by hand.
    #[serde(default)]
    pub(crate) autofill: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) settings: Settings,
    /// Per-part exceptions -- see `crate::assignments`'s module docs for
    /// why an exception here always outranks an `autofill` rule.
    #[serde(default)]
    pub(crate) assignments: BTreeMap<String, PartOverride>,
}

impl Project {
    /// The STEP export this project reviews, resolved against the
    /// directory `project_file` (this project's own `storystick.yaml`)
    /// lives in.
    pub(crate) fn step_path(&self, project_file: &Path) -> PathBuf {
        project_dir(project_file).join(&self.step)
    }

    pub(crate) fn out_pdf_path(&self, project_file: &Path) -> PathBuf {
        project_dir(project_file).join(&self.settings.out_pdf)
    }

    /// `materials`, resolved to full `Material` values against the global
    /// catalog (`global`, as loaded from `crate::stock`) -- errors by
    /// name if a material this project uses no longer exists there (the
    /// shop catalog changed out from under this project), rather than
    /// silently dropping it.
    pub(crate) fn resolve_materials(
        &self,
        global: &[Material],
    ) -> Result<Vec<Material>, Box<dyn Error>> {
        self.materials
            .iter()
            .map(|name| {
                global
                    .iter()
                    .find(|m| &m.name == name)
                    .cloned()
                    .ok_or_else(|| {
                        format!("storystick.yaml: material {name:?} not found in the stock catalog")
                    })
            })
            .collect::<Result<Vec<Material>, String>>()
            .map_err(Into::into)
    }

    /// Every sheet in `global` whose material is part of this project's
    /// subset -- what `pack()` is actually allowed to nest onto.
    pub(crate) fn resolve_stock(&self, global: &[StockSheet]) -> Vec<StockSheet> {
        global
            .iter()
            .filter(|s| self.materials.iter().any(|m| m == &s.material.name))
            .cloned()
            .collect()
    }

    /// Every `autofill` value and `assignments[].material` must name a
    /// material in `materials` (this project's own subset, already
    /// resolved against the global catalog by `resolve_materials`) --
    /// not just the wider global catalog, since a project deliberately
    /// narrows which materials it uses. Call this right after
    /// `resolve_materials`, before parts are ever loaded.
    ///
    /// Unlike `materials` above and a stock sheet's own `material:`
    /// field, these two maps were never checked against the catalog at
    /// all: a typo'd or stale name in either one used to flow silently
    /// into a part's displayed material (`review::resolve_material`
    /// trusts a name it's handed) instead of failing loudly here, where
    /// it can name the offending file key instead of just leaving a part
    /// mysteriously unresolved.
    pub(crate) fn validate_references(&self, materials: &[Material]) -> Result<(), Box<dyn Error>> {
        let known: Vec<&str> = materials.iter().map(|m| m.name.as_str()).collect();
        for (tag, name) in &self.autofill {
            if !known.contains(&name.as_str()) {
                return Err(unresolved_material_err(
                    &format!("autofill rule {tag:?}"),
                    name,
                    &known,
                ));
            }
        }
        for (key, over) in &self.assignments {
            if let Some(name) = &over.material {
                if !known.contains(&name.as_str()) {
                    return Err(unresolved_material_err(
                        &format!("assignment for {key:?}"),
                        name,
                        &known,
                    ));
                }
            }
        }
        Ok(())
    }
}

fn unresolved_material_err(context: &str, name: &str, known: &[&str]) -> Box<dyn Error> {
    let suggestion = closest_match(name, known)
        .map(|m| format!(" (did you mean {m:?}?)"))
        .unwrap_or_default();
    format!("storystick.yaml: {context} -> material {name:?} not found in this project's materials{suggestion}").into()
}

/// The closest of `candidates` to `target` by edit distance, if any comes
/// within a third of `target`'s own length -- close enough to plausibly
/// be a typo of it, not just some other, unrelated material name. Exists
/// purely to make `validate_references`' error actionable (point at the
/// likely intended name); never used to *resolve* a reference -- an
/// inexact match must never silently stand in for an exact one.
fn closest_match<'a>(target: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let max_distance = (target.chars().count() / 3).max(1);
    candidates
        .iter()
        .map(|c| (*c, levenshtein(target, c)))
        .filter(|(_, d)| *d <= max_distance)
        .min_by_key(|(_, d)| *d)
        .map(|(c, _)| c)
}

/// Classic edit distance, one DP row kept at a time -- no crate dependency
/// worth pulling in for a helper this small, used only for a "did you
/// mean" suggestion.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

fn project_dir(project_file: &Path) -> &Path {
    project_file.parent().unwrap_or_else(|| Path::new("."))
}

/// Pure parsing, no file I/O -- kept separate from `load` so it can be
/// exercised directly with an in-memory string, the same way every other
/// loader in this codebase (`stock::parse`, `stepcrawl::extract_parts`)
/// separates the format from the disk access.
pub(crate) fn parse(text: &str) -> Result<Project, Box<dyn Error>> {
    Ok(yaml_serde::from_str(text)?)
}

pub(crate) fn load(path: &Path) -> Result<Project, Box<dyn Error>> {
    parse(&std::fs::read_to_string(path)?)
}

pub(crate) fn save(project: &Project, path: &Path) -> Result<(), Box<dyn Error>> {
    std::fs::write(path, yaml_serde::to_string(project)?)?;
    Ok(())
}

/// Walks `start`'s ancestors (itself first, then each parent up to the
/// filesystem root), git-style, for the first directory containing
/// `filename` -- pure over an injected `exists` predicate so it's testable
/// without touching a real filesystem; `discover` is the real thing.
pub(crate) fn find_project_file(
    start: &Path,
    filename: &str,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(filename))
        .find(|candidate| exists(candidate))
}

pub(crate) fn discover(start: &Path) -> Option<PathBuf> {
    find_project_file(start, FILENAME, |p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignments::DimensionAssignment;
    use std::collections::HashSet;

    fn material(name: &str, thickness_mm: f64) -> Material {
        Material {
            name: name.to_string(),
            thickness_mm,
        }
    }

    #[test]
    fn parse_reads_a_full_project() {
        let yaml = r#"
step: model.step
materials:
  - "Baltic Birch 3/4"
  - "Sande Ply 3/4"
autofill:
  "[Panel]": "Baltic Birch 3/4"
settings:
  kerf_in: 0.125
  trim_allowance_in: 0.25
  out_pdf: cutlist.pdf
assignments:
  "Bench / Body @ 30.0000x20.0000x0.7500":
    material: Sande Ply 3/4
"#;
        let project = parse(yaml).unwrap();
        assert_eq!(project.step, "model.step");
        assert_eq!(project.materials, vec!["Baltic Birch 3/4", "Sande Ply 3/4"]);
        assert_eq!(project.autofill["[Panel]"], "Baltic Birch 3/4");
        assert_eq!(project.settings.trim_allowance_in, 0.25);
        assert_eq!(
            project.assignments["Bench / Body @ 30.0000x20.0000x0.7500"]
                .material
                .as_deref(),
            Some("Sande Ply 3/4")
        );
    }

    #[test]
    fn parse_defaults_everything_but_step() {
        let project = parse("step: model.step").unwrap();
        assert!(project.materials.is_empty());
        assert!(project.autofill.is_empty());
        assert!(project.assignments.is_empty());
        assert_eq!(project.settings, Settings::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("storystick-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILENAME);

        let mut autofill = BTreeMap::new();
        autofill.insert("[Panel]".to_string(), "Baltic Birch 3/4".to_string());
        let project = Project {
            step: "model.step".to_string(),
            materials: vec!["Baltic Birch 3/4".to_string()],
            autofill,
            settings: Settings::default(),
            assignments: BTreeMap::new(),
        };
        save(&project, &path).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back, project);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn step_path_and_out_pdf_path_resolve_relative_to_the_project_files_own_directory() {
        let project = Project {
            step: "model.step".to_string(),
            materials: vec![],
            autofill: BTreeMap::new(),
            settings: Settings {
                out_pdf: "out/cutlist.pdf".to_string(),
                ..Settings::default()
            },
            assignments: BTreeMap::new(),
        };
        let project_file = Path::new("/projects/bench/storystick.yaml");
        assert_eq!(
            project.step_path(project_file),
            Path::new("/projects/bench/model.step")
        );
        assert_eq!(
            project.out_pdf_path(project_file),
            Path::new("/projects/bench/out/cutlist.pdf")
        );
    }

    #[test]
    fn resolve_materials_looks_up_by_name_against_the_global_catalog() {
        let global = vec![
            material("Baltic Birch 3/4", 19.05),
            material("Baltic Birch 1/4", 6.35),
        ];
        let project = Project {
            step: "model.step".to_string(),
            materials: vec!["Baltic Birch 3/4".to_string()],
            autofill: BTreeMap::new(),
            settings: Settings::default(),
            assignments: BTreeMap::new(),
        };
        let resolved = project.resolve_materials(&global).unwrap();
        assert_eq!(resolved, vec![material("Baltic Birch 3/4", 19.05)]);
    }

    #[test]
    fn resolve_materials_errs_when_a_referenced_name_is_missing_from_the_catalog() {
        let global = vec![material("Baltic Birch 3/4", 19.05)];
        let project = Project {
            step: "model.step".to_string(),
            materials: vec!["Sande Ply 3/4".to_string()],
            autofill: BTreeMap::new(),
            settings: Settings::default(),
            assignments: BTreeMap::new(),
        };
        let err = project.resolve_materials(&global).unwrap_err();
        assert!(err.to_string().contains("Sande Ply 3/4"));
    }

    #[test]
    fn resolve_stock_keeps_only_sheets_of_the_projects_own_materials() {
        let bb34 = material("Baltic Birch 3/4", 19.05);
        let sande34 = material("Sande Ply 3/4", 19.05);
        let global = vec![
            StockSheet {
                material: bb34,
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
            StockSheet {
                material: sande34,
                length_mm: 2438.4,
                width_mm: 1219.2,
            },
        ];
        let project = Project {
            step: "model.step".to_string(),
            materials: vec!["Baltic Birch 3/4".to_string()],
            autofill: BTreeMap::new(),
            settings: Settings::default(),
            assignments: BTreeMap::new(),
        };
        let resolved = project.resolve_stock(&global);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].material.name, "Baltic Birch 3/4");
    }

    #[test]
    fn validate_references_passes_when_autofill_and_assignments_name_real_materials() {
        let materials = vec![
            material("Baltic Birch 3/4", 19.05),
            material("Sande Ply 3/4", 19.05),
        ];
        let mut autofill = BTreeMap::new();
        autofill.insert("[Panel]".to_string(), "Baltic Birch 3/4".to_string());
        let mut assignments = BTreeMap::new();
        assignments.insert(
            "Bench / Body".to_string(),
            PartOverride {
                material: Some("Sande Ply 3/4".to_string()),
                grain_along_length: true,
                dimensions: DimensionAssignment::AS_GUESSED,
            },
        );
        let project = Project {
            step: "model.step".to_string(),
            materials: vec!["Baltic Birch 3/4".to_string(), "Sande Ply 3/4".to_string()],
            autofill,
            settings: Settings::default(),
            assignments,
        };
        assert!(project.validate_references(&materials).is_ok());
    }

    #[test]
    fn validate_references_errs_and_suggests_the_close_name_for_a_typo_d_autofill_rule() {
        let materials = vec![material("Baltic Birch 3/4", 19.05)];
        let mut autofill = BTreeMap::new();
        autofill.insert("[Panel]".to_string(), "Baltic Brich 3/4".to_string());
        let project = Project {
            step: "model.step".to_string(),
            materials: vec!["Baltic Birch 3/4".to_string()],
            autofill,
            settings: Settings::default(),
            assignments: BTreeMap::new(),
        };
        let err = project
            .validate_references(&materials)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("[Panel]"),
            "should name the offending rule: {err}"
        );
        assert!(
            err.contains("Baltic Brich 3/4"),
            "should name the bad value: {err}"
        );
        assert!(
            err.contains("did you mean \"Baltic Birch 3/4\""),
            "should suggest the close match: {err}"
        );
    }

    #[test]
    fn validate_references_errs_on_an_unknown_assignment_material_with_no_suggestion_when_nothing_is_close(
    ) {
        let materials = vec![material("Baltic Birch 3/4", 19.05)];
        let mut assignments = BTreeMap::new();
        assignments.insert(
            "Bench / Body".to_string(),
            PartOverride {
                material: Some("Oak".to_string()),
                grain_along_length: true,
                dimensions: DimensionAssignment::AS_GUESSED,
            },
        );
        let project = Project {
            step: "model.step".to_string(),
            materials: vec!["Baltic Birch 3/4".to_string()],
            autofill: BTreeMap::new(),
            settings: Settings::default(),
            assignments,
        };
        let err = project
            .validate_references(&materials)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Bench / Body"),
            "should name the offending assignment key: {err}"
        );
        assert!(err.contains("Oak"), "should name the bad value: {err}");
        assert!(
            !err.contains("did you mean"),
            "\"Oak\" isn't a plausible typo of \"Baltic Birch 3/4\": {err}"
        );
    }

    #[test]
    fn find_project_file_walks_up_from_a_subdirectory() {
        let existing: HashSet<PathBuf> = [PathBuf::from("/projects/bench/storystick.yaml")]
            .into_iter()
            .collect();
        let start = Path::new("/projects/bench/Carcasses/Left");
        let found = find_project_file(start, "storystick.yaml", |p| existing.contains(p));
        assert_eq!(
            found,
            Some(PathBuf::from("/projects/bench/storystick.yaml"))
        );
    }

    #[test]
    fn find_project_file_prefers_the_nearest_ancestor() {
        let existing: HashSet<PathBuf> = [
            PathBuf::from("/projects/storystick.yaml"),
            PathBuf::from("/projects/bench/storystick.yaml"),
        ]
        .into_iter()
        .collect();
        let start = Path::new("/projects/bench/Carcasses");
        let found = find_project_file(start, "storystick.yaml", |p| existing.contains(p));
        assert_eq!(
            found,
            Some(PathBuf::from("/projects/bench/storystick.yaml"))
        );
    }

    #[test]
    fn find_project_file_returns_none_when_nothing_exists_up_to_the_root() {
        let found = find_project_file(
            Path::new("/projects/bench/Carcasses"),
            "storystick.yaml",
            |_| false,
        );
        assert_eq!(found, None);
    }
}
