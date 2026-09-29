//! First-run project creation: when `crate::project::discover` finds no
//! `storystick.yaml` anywhere above the current directory, this runs
//! once to create one -- plain stdin/stdout prompts, not a ratatui
//! screen, since it happens before the review TUI's terminal takeover
//! and only ever runs the one time per project. Scope is deliberately
//! small: locate the STEP export, and pick which of the global catalog's
//! materials this project uses. Bracket-tag rules (`autofill`) aren't
//! asked about here at all -- bulk-edit (see `review`'s module docs) is
//! where those actually get authored, as you review real parts.

use crate::project::{Project, Settings};
use crate::stock;
use std::collections::BTreeMap;
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use storystick_core::nesting::StockSheet;

fn prompt_line(prompt: &str) -> Result<String, Box<dyn Error>> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

/// Every `*.step`/`*.stp` file directly in `dir` (case-insensitive
/// extension), sorted for a stable prompt order. Pure I/O, no prompting --
/// kept separate so the actual candidate-picking logic (`pick_one`) is
/// testable without a real directory.
fn find_step_candidates(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("step") || ext.eq_ignore_ascii_case("stp")))
        .collect();
    candidates.sort();
    Ok(candidates)
}

/// Parses a 1-based index typed at a prompt against `len` options --
/// pure, so the "did the user type something sane" logic is testable
/// without stdin.
fn parse_index(input: &str, len: usize) -> Result<usize, String> {
    let n: usize = input.trim().parse().map_err(|_| format!("{input:?} isn't a number"))?;
    if n == 0 || n > len {
        return Err(format!("{n} is out of range (1-{len})"));
    }
    Ok(n - 1)
}

/// Parses a comma-separated list of 1-based indices, or the literal
/// `"all"`, against `names` -- pure, same testability rationale as
/// `parse_index`.
fn parse_material_selection(input: &str, names: &[String]) -> Result<Vec<String>, String> {
    if input.trim().eq_ignore_ascii_case("all") {
        return Ok(names.to_vec());
    }
    input
        .split(',')
        .map(|tok| parse_index(tok, names.len()).map(|i| names[i].clone()))
        .collect()
}

fn pick_step_file(dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let candidates = find_step_candidates(dir)?;
    match candidates.len() {
        0 => Err(format!(
            "no .step file found in {} -- export one from Shapr3D first, then run storystick again",
            dir.display()
        )
        .into()),
        1 => Ok(candidates.into_iter().next().unwrap()),
        _ => {
            println!("Multiple STEP files found in {}:", dir.display());
            for (i, path) in candidates.iter().enumerate() {
                println!("  {}) {}", i + 1, path.file_name().unwrap_or_default().to_string_lossy());
            }
            loop {
                let input = prompt_line("Pick one (number): ")?;
                match parse_index(&input, candidates.len()) {
                    Ok(i) => return Ok(candidates[i].clone()),
                    Err(e) => println!("{e}, try again."),
                }
            }
        }
    }
}

fn pick_materials(global_stock: &[StockSheet]) -> Result<Vec<String>, Box<dyn Error>> {
    let names: Vec<String> = stock::distinct_materials(global_stock).into_iter().map(|m| m.name).collect();
    if names.is_empty() {
        return Err("the stock catalog has no materials -- add some to stock.yaml first".into());
    }
    println!("Which materials does this project use?");
    for (i, name) in names.iter().enumerate() {
        println!("  {}) {name}", i + 1);
    }
    loop {
        let input = prompt_line("Numbers separated by commas, or \"all\": ")?;
        match parse_material_selection(&input, &names) {
            Ok(selected) if !selected.is_empty() => return Ok(selected),
            Ok(_) => println!("pick at least one, try again."),
            Err(e) => println!("{e}, try again."),
        }
    }
}

/// Creates a new project in `dir`: finds (or asks about) the STEP export,
/// asks which materials this project uses, saves a fresh
/// `storystick.yaml` there, and returns it loaded -- ready for
/// `review::run` to continue straight into, with no second invocation
/// needed.
pub(crate) fn create(dir: &Path, global_stock: &[StockSheet]) -> Result<(Project, PathBuf), Box<dyn Error>> {
    println!("No {} found above {} -- let's set one up.", crate::project::FILENAME, dir.display());

    let step_file = pick_step_file(dir)?;
    let step = step_file.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let materials = pick_materials(global_stock)?;

    let project = Project { step, materials, autofill: BTreeMap::new(), settings: Settings::default(), assignments: BTreeMap::new() };
    let project_path = dir.join(crate::project::FILENAME);
    crate::project::save(&project, &project_path)?;
    println!("Created {}", project_path.display());

    Ok((project, project_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_index_accepts_a_valid_one_based_choice() {
        assert_eq!(parse_index("2", 3), Ok(1));
    }

    #[test]
    fn parse_index_rejects_zero_and_out_of_range() {
        assert!(parse_index("0", 3).is_err());
        assert!(parse_index("4", 3).is_err());
        assert!(parse_index("not a number", 3).is_err());
    }

    #[test]
    fn parse_material_selection_all_returns_every_name_in_order() {
        let names = vec!["Baltic Birch 3/4".to_string(), "Sande Ply 3/4".to_string()];
        assert_eq!(parse_material_selection("all", &names), Ok(names.clone()));
        assert_eq!(parse_material_selection("ALL", &names), Ok(names));
    }

    #[test]
    fn parse_material_selection_parses_comma_separated_indices() {
        let names = vec!["Baltic Birch 3/4".to_string(), "Sande Ply 3/4".to_string(), "Baltic Birch 1/4".to_string()];
        assert_eq!(parse_material_selection("2, 1", &names), Ok(vec!["Sande Ply 3/4".to_string(), "Baltic Birch 3/4".to_string()]));
    }

    #[test]
    fn parse_material_selection_errs_on_an_out_of_range_index() {
        let names = vec!["Baltic Birch 3/4".to_string()];
        assert!(parse_material_selection("2", &names).is_err());
    }
}
