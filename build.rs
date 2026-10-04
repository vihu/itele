//! Compiles the Slint UI in `ui/`, and names the commit itele is built from
//! for Settings' About section.

use std::path::Path;
use std::process::Command;

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the Slint UI compiles");

    // The short hash and date of the last commit, for example
    // `7542fa1 2026-10-04`; empty outside a Git checkout or before the
    // first commit.
    let commit = Command::new("git")
        .args(["log", "-1", "--format=%h %cs"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    println!("cargo:rustc-env=ITELE_COMMIT={commit}");
    if Path::new(".git/logs/HEAD").exists() {
        println!("cargo:rerun-if-changed=.git/logs/HEAD");
    }
}
