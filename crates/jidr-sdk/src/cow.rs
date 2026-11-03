//! Copy-on-write directory copies (D42, Phase 13b): a cloned AVD shares
//! every block with its source until either side writes it, so a clone with
//! a ~1 GB snapshot is instant and free.
//!
//! macOS uses APFS `clonefile` and Linux the `FICLONE` reflink (Btrfs, XFS),
//! both through `cp` (`cp -c`, `cp --reflink=always`), so no `unsafe` is
//! needed. Windows ReFS block cloning comes later. Where the filesystem
//! can't clone, a full copy runs after a disk-space check, with a warning.

use std::path::Path;
use std::process::Command;

use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};
use serde::Serialize;

/// How a tree was copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CopyMethod {
    /// Copy-on-write: no extra disk space until written.
    Clone,
    /// Every byte copied.
    Full,
}

/// Copies directory `from` to `to` (which must not exist), cloning when the
/// filesystem allows it.
pub fn cow_copy(from: &Path, to: &Path) -> JidrResult<CopyMethod> {
    if to.exists() {
        return Err(JidrError::usage(format!("{} already exists", to.display())));
    }
    if reflink_tree(from, to) {
        return Ok(CopyMethod::Clone);
    }
    // A failed clone may leave a partial tree behind.
    let _ = std::fs::remove_dir_all(to);
    let needed = crate::avd::disk_usage(from);
    let free = crate::host::free_space(to)?;
    full_copy(from, to, needed, free)?;
    tracing::warn!(
        from = %from.display(),
        bytes = needed,
        "the filesystem can't clone; made a full copy"
    );
    Ok(CopyMethod::Full)
}

/// `cp -c -R` (APFS `clonefile`) or `cp -a --reflink=always` (`FICLONE`).
fn reflink_tree(from: &Path, to: &Path) -> bool {
    let mut cmd = Command::new("cp");
    if cfg!(target_os = "macos") {
        cmd.args(["-c", "-R"]);
    } else if cfg!(target_os = "linux") {
        cmd.args(["-a", "--reflink=always"]);
    } else {
        return false;
    }
    cmd.arg(from)
        .arg(to)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The fallback: refuses with `sdk` (naming both sizes) when `needed` bytes
/// don't fit in `free`, else copies every file.
pub fn full_copy(from: &Path, to: &Path, needed: u64, free: u64) -> JidrResult<()> {
    // Keep 1 GB spare so a copy never fills the disk.
    const SPARE: u64 = 1 << 30;
    if needed.saturating_add(SPARE) > free {
        return Err(JidrError::sdk(format!(
            "this filesystem can't make copy-on-write clones, and a full copy of {} needs {:.1} GB; {:.1} GB is free",
            from.display(),
            needed as f64 / 1e9,
            free as f64 / 1e9
        ))
        .with_data(serde_json::json!({ "needed": needed, "free": free })));
    }
    let result = copy_dir(from, to);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(to);
    }
    result
}

fn copy_dir(from: &Path, to: &Path) -> JidrResult<()> {
    std::fs::create_dir_all(to).ctx(ErrorCode::Sdk, format!("creating {}", to.display()))?;
    for e in std::fs::read_dir(from).ctx(ErrorCode::Sdk, format!("reading {}", from.display()))? {
        let e = e.ctx(ErrorCode::Sdk, "reading directory entry")?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let ty = e.file_type().ctx(ErrorCode::Sdk, "reading file type")?;
        if ty.is_dir() {
            copy_dir(&src, &dst)?;
        } else if ty.is_file() {
            std::fs::copy(&src, &dst).ctx(ErrorCode::Sdk, format!("copying {}", src.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) {
        std::fs::create_dir_all(root.join("snapshots/s1")).unwrap();
        std::fs::write(root.join("config.ini"), "AvdId = a\n").unwrap();
        std::fs::write(root.join("snapshots/s1/ram.bin"), vec![7u8; 4096]).unwrap();
    }

    #[test]
    fn cow_copy_copies_the_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a.avd"), tmp.path().join("b.avd"));
        tree(&a);
        let method = cow_copy(&a, &b).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(method, CopyMethod::Clone, "temp dirs are APFS on macOS");
        }
        assert_eq!(
            std::fs::read(b.join("snapshots/s1/ram.bin")).unwrap(),
            vec![7u8; 4096]
        );
        // The copies are independent.
        std::fs::write(b.join("config.ini"), "AvdId = b\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(a.join("config.ini")).unwrap(),
            "AvdId = a\n"
        );
        assert_eq!(
            cow_copy(&a, &b).unwrap_err().code,
            ErrorCode::Usage,
            "target exists"
        );
    }

    #[test]
    fn full_copy_fallback_checks_disk_space() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        tree(&a);
        let e = full_copy(&a, &b, 5 << 30, 4 << 30).unwrap_err();
        assert_eq!(e.code, ErrorCode::Sdk);
        assert!(
            e.message.contains("5.4 GB") && e.message.contains("4.3 GB"),
            "{e}"
        );
        assert!(!b.exists(), "nothing written when it doesn't fit");
        full_copy(&a, &b, 4096, 8 << 30).unwrap();
        assert_eq!(
            std::fs::read(b.join("snapshots/s1/ram.bin")).unwrap().len(),
            4096
        );
    }
}
