//! storystick: review a Shapr3D STEP export interactively -- flag parts
//! needing attention, assign materials from your stock catalog, and
//! generate the printable cutlist PDF, all from one command.
//!
//! There's no intermediate parts.csv: geometry always comes fresh from
//! the STEP file (see `stepcrawl`); the only state that persists between
//! runs is the path -> material assignment sidecar (see `assignments`).

mod assignments;
mod autofill;
mod review;
mod sections;
mod stock;

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

pub(crate) const MM_PER_IN: f64 = 25.4;

#[derive(Parser)]
#[command(
    name = "storystick",
    about = "Review a Shapr3D STEP export, assign materials, and generate a printable cutlist PDF."
)]
struct Cli {
    /// path to a Shapr3D STEP export
    step_path: PathBuf,
    /// stock sheet catalog (default: ~/.config/storystick/stock.yaml)
    #[arg(long, value_name = "PATH")]
    stock: Option<PathBuf>,
    /// cutlist PDF output path, used when generating from within review
    /// (default: <model>.cutlist.pdf next to the STEP file)
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,
    /// saw kerf, inches, used when generating the cutlist (default: 1/8")
    #[arg(long, default_value_t = 1.0 / 8.0)]
    kerf_in: f64,
    /// extra rough-cut margin per part, inches, used when generating the
    /// cutlist (default: 0, i.e. off)
    #[arg(long, default_value_t = 0.0)]
    trim_allowance_in: f64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stock_path = cli.stock.unwrap_or_else(stock::default_path);
    let out_path = cli.out.unwrap_or_else(|| cli.step_path.with_extension("cutlist.pdf"));

    match review::run(&cli.step_path, &stock_path, &out_path, cli.kerf_in, cli.trim_allowance_in) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Round to 4 decimal inches -- the resolution parts.csv used to store
/// values at. Keeping it matters beyond display: `pack()` buckets
/// unassigned parts by exact thickness_mm bit-pattern, which only lines
/// up with a stock material's thickness_mm (itself `thickness_in * 25.4`
/// off a clean decimal) if both sides go through the same rounding.
pub(crate) fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}
