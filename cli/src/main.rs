//! storystick: review a Shapr3D STEP export interactively -- flag parts
//! needing attention, assign materials, and generate the printable
//! cutlist PDF, all from one command run inside a project directory.
//!
//! There's no CLI argument naming which STEP file or project to use:
//! running `storystick` looks for `storystick.yaml` by walking up from
//! the current directory (see `project::discover`), the same way `git`
//! finds `.git` -- a directory with none anywhere above it runs the
//! creation wizard (`wizard::create`) instead of failing. Geometry always
//! comes fresh from the STEP file (see `stepcrawl`); everything else that
//! persists between runs lives in that one project file (see `project`).

mod assignments;
mod autofill;
mod project;
mod review;
mod sections;
mod stock;
mod wizard;

use clap::Parser;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) const MM_PER_IN: f64 = 25.4;

#[derive(Parser)]
#[command(
    name = "storystick",
    about = "Review this directory's Shapr3D STEP export, assign materials, and generate a printable cutlist PDF."
)]
struct Cli {
    /// stock sheet catalog (default: ~/.config/storystick/stock.yaml)
    #[arg(long, value_name = "PATH")]
    stock: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stock_path = cli.stock.unwrap_or_else(stock::default_path);

    match run(&stock_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(stock_path: &Path) -> Result<(), Box<dyn Error>> {
    let global_stock = stock::read(stock_path)?;
    let cwd = std::env::current_dir()?;

    let (project, project_path) = match project::discover(&cwd) {
        Some(path) => (project::load(&path)?, path),
        None => wizard::create(&cwd, &global_stock)?,
    };

    review::run(project, project_path, global_stock)
}

/// Round to 4 decimal inches -- the resolution parts.csv used to store
/// values at. Keeping it matters beyond display: `pack()` buckets
/// unassigned parts by exact thickness_mm bit-pattern, which only lines
/// up with a stock material's thickness_mm (itself `thickness_in * 25.4`
/// off a clean decimal) if both sides go through the same rounding.
pub(crate) fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}
