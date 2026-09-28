//! Throwaway verification tool: prints extract_parts() output in the same
//! shape as the Python CLI's `storystick parts` CSV, for diffing against
//! the known-good Python reference on real project data.
use std::path::PathBuf;

fn leaf(path: &str) -> &str {
    path.rsplit(" / ").next().unwrap_or(path)
}

fn main() {
    let step_path = PathBuf::from(std::env::args().nth(1).expect("usage: extract_parts_csv <step-file>"));
    let groups = storystick_core::stepcrawl::extract_parts(&step_path).expect("failed to parse STEP file");

    println!("top_folder,label,path,length_in,width_in,thickness_in,unreliable");
    for group in &groups {
        for instance in &group.instances {
            println!(
                "{},{},{},{:.4},{:.4},{:.4},{}",
                group.top_folder,
                leaf(&instance.path),
                instance.path,
                group.length_mm / 25.4,
                group.width_mm / 25.4,
                group.thickness_mm / 25.4,
                instance.unreliable,
            );
        }
    }
}
