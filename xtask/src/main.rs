//! `cargo xtask <task>`: build automation for the Jidr workspace.

mod license_check;
mod release;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  ci              fmt --check, clippy -D warnings, nextest, license-check
  license-check   fail if an Apache-2.0 crate links Slint or the GPL GUI crates (D18)
  e2e-mac         full arm64 end-to-end gate on Apple Silicon (release gate, D28)
  release         build the macOS release artifacts (Stream C, Phase 12a)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("license-check") => license_check::run(),
        Some("e2e-mac") => e2e_mac(),
        Some("release") => release::run(&args[1..]),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            println!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            println!("xtask: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// The cargo binary that invoked us (respects rustup overrides).
pub(crate) fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Runs a command in the workspace root and fails if it exits non-zero.
pub(crate) fn run(program: &str, args: &[&str]) -> Result<()> {
    println!("xtask: $ {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .status()
        .with_context(|| format!("spawning {program}"))?;
    if !status.success() {
        bail!("`{program} {}` failed with {status}", args.join(" "));
    }
    Ok(())
}

fn ci() -> Result<()> {
    let cargo = cargo();
    run(&cargo, &["fmt", "--all", "--check"])?;
    run(
        &cargo,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(
        &cargo,
        &[
            "nextest",
            "run",
            "--workspace",
            "--locked",
            "--no-tests=pass",
        ],
    )?;
    license_check::run()?;
    println!("xtask: ci ok");
    Ok(())
}

/// Release gate on the dev Mac: every emulator test on arm64 images, then the
/// end-to-end scripts. Missing scripts fail the gate.
fn e2e_mac() -> Result<()> {
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        bail!("e2e-mac runs only on Apple Silicon macOS");
    }
    let cargo = cargo();
    run(
        &cargo,
        &[
            "nextest",
            "run",
            "--workspace",
            "--features",
            "emulator",
            "--no-tests=pass",
        ],
    )?;
    for script in [
        "scripts/e2e-cli.sh",
        "scripts/e2e-root.sh",
        "scripts/e2e-studio-untouched.sh",
    ] {
        if !workspace_root().join(script).exists() {
            bail!("{script} is missing");
        }
        run("bash", &[script])?;
    }
    println!("xtask: e2e-mac ok");
    Ok(())
}
