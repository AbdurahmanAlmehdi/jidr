//! `cargo xtask helper`: builds the rooted on-device helper (ADR-0003) for
//! the Android guest, as a static musl executable linked with the
//! toolchain's own `rust-lld`, so no Android NDK or C toolchain is needed.
//!
//! Output: `<target>/jidr-helper/<arch>/jidr-helper`, where the daemon looks
//! for it in development builds (`jidr_helper::host::search_dirs`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{cargo, workspace_root};

const USAGE: &str = "\
usage: cargo xtask helper [--arch aarch64|x86_64]...

Builds jidr-helper for each guest CPU (default: both) into
<target>/jidr-helper/<arch>/jidr-helper.";

/// Guest CPUs: (the helper's arch directory, the Rust target).
pub const TARGETS: [(&str, &str); 2] = [
    ("aarch64", "aarch64-unknown-linux-musl"),
    ("x86_64", "x86_64-unknown-linux-musl"),
];

fn parse_args(args: &[String]) -> Result<Option<Vec<&'static str>>> {
    let mut arches = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--arch" => {
                let a = it.next().context("--arch needs aarch64 or x86_64")?;
                let (arch, _) = TARGETS
                    .iter()
                    .find(|(name, _)| name == a)
                    .with_context(|| format!("unknown arch `{a}`\n{USAGE}"))?;
                arches.push(*arch);
            }
            "-h" | "--help" => return Ok(None),
            other => bail!("unknown argument `{other}`\n{USAGE}"),
        }
    }
    if arches.is_empty() {
        arches = TARGETS.iter().map(|(a, _)| *a).collect();
    }
    Ok(Some(arches))
}

pub fn run(args: &[String]) -> Result<()> {
    let Some(arches) = parse_args(args)? else {
        println!("{USAGE}");
        return Ok(());
    };
    for (arch, path) in build(&arches)? {
        println!("xtask: helper {arch}: {}", path.display());
    }
    Ok(())
}

/// The toolchain's sysroot; `rust-lld` loads `libLLVM` from `<sysroot>/lib`.
fn sysroot() -> Result<PathBuf> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = Command::new(rustc)
        .args(["--print", "sysroot"])
        .current_dir(workspace_root())
        .output()
        .context("running rustc --print sysroot")?;
    if !out.status.success() {
        bail!("rustc --print sysroot failed");
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Adds the Rust target with rustup when it isn't installed yet.
fn ensure_target(target: &str) -> Result<()> {
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .current_dir(workspace_root())
        .output()
        .context("running rustup (install the target by hand if you don't use rustup)")?;
    if String::from_utf8_lossy(&installed.stdout)
        .lines()
        .any(|l| l.trim() == target)
    {
        return Ok(());
    }
    crate::run("rustup", &["target", "add", target])
}

/// Builds the helper for `arches`; returns where each one is.
pub fn build(arches: &[&str]) -> Result<Vec<(String, PathBuf)>> {
    let root = workspace_root();
    let target_dir = crate::release::target_dir(&root);
    let lib = sysroot()?.join("lib");
    let mut built = Vec::new();
    for arch in arches {
        let (_, target) = TARGETS
            .iter()
            .find(|(a, _)| a == arch)
            .with_context(|| format!("unknown arch `{arch}`"))?;
        ensure_target(target)?;
        let linker_var = format!(
            "CARGO_TARGET_{}_LINKER",
            target.to_uppercase().replace('-', "_")
        );
        println!("xtask: $ cargo build -p jidr-helper --profile helper --target {target}");
        let status = Command::new(cargo())
            .args([
                "build",
                "--locked",
                "-p",
                "jidr-helper",
                "--bin",
                "jidr-helper",
                "--profile",
                "helper",
                "--target",
                target,
            ])
            .env(linker_var, "rust-lld")
            .env(library_path_var(), &lib)
            .current_dir(&root)
            .status()
            .context("spawning cargo")?;
        if !status.success() {
            bail!("building jidr-helper for {target} failed with {status}");
        }
        let from = target_dir.join(target).join("helper/jidr-helper");
        let to = output(&target_dir, arch);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(&from, &to)
            .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        built.push((arch.to_string(), to));
    }
    Ok(built)
}

/// Where `cargo xtask helper` puts the build for `arch`.
pub fn output(target_dir: &Path, arch: &str) -> PathBuf {
    target_dir
        .join("jidr-helper")
        .join(arch)
        .join("jidr-helper")
}

fn library_path_var() -> &'static str {
    if cfg!(target_os = "macos") {
        "DYLD_FALLBACK_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arches_default_to_both_and_are_checked() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_args(&[]).unwrap(), Some(vec!["aarch64", "x86_64"]));
        assert_eq!(
            parse_args(&args(&["--arch", "x86_64"])).unwrap(),
            Some(vec!["x86_64"])
        );
        assert!(parse_args(&args(&["--arch", "armv7"])).is_err());
        assert!(parse_args(&args(&["--arch"])).is_err());
        assert_eq!(parse_args(&args(&["-h"])).unwrap(), None);
        assert_eq!(
            output(Path::new("/t"), "aarch64"),
            Path::new("/t/jidr-helper/aarch64/jidr-helper")
        );
    }
}
