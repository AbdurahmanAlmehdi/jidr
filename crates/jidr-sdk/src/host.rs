//! Facts about the host machine: ABI, hypervisor, free disk space.

use std::path::Path;
use std::process::Command;

use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};

/// The system-image ABI that runs natively on this host.
pub fn host_abi() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else {
        "x86_64"
    }
}

/// `host-os` as used in the SDK repository XML.
pub fn host_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macosx"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// `host-arch` as used in the SDK repository XML.
pub fn host_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x64"
    }
}

/// True if hardware virtualization is usable (HVF on macOS, KVM on Linux).
pub fn hypervisor_available() -> bool {
    if cfg!(target_os = "macos") {
        Command::new("sysctl")
            .args(["-n", "kern.hv_support"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
            .unwrap_or(false)
    } else if cfg!(target_os = "linux") {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok()
    } else {
        false
    }
}

/// Bytes available to this user on the volume holding `path` (or its nearest
/// existing ancestor).
pub fn free_space(path: &Path) -> JidrResult<u64> {
    let existing = path.ancestors().find(|p| p.exists()).ok_or_else(|| {
        JidrError::unexpected(format!("no existing ancestor of {}", path.display()))
    })?;
    fs4::available_space(existing).ctx(
        ErrorCode::Sdk,
        format!("free space of {}", existing.display()),
    )
}

/// Fails with `sdk` if fewer than `needed` bytes are free at `path`.
pub fn ensure_free_space(path: &Path, needed: u64) -> JidrResult<()> {
    let free = free_space(path)?;
    if free < needed {
        return Err(JidrError::sdk(format!(
            "not enough disk space at {}: need {:.1} GB, {:.1} GB free",
            path.display(),
            needed as f64 / 1e9,
            free as f64 / 1e9
        ))
        .with_data(serde_json::json!({"needed": needed, "free": free})));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_matches_arch() {
        #[cfg(target_arch = "aarch64")]
        assert_eq!((host_abi(), host_arch()), ("arm64-v8a", "aarch64"));
        #[cfg(target_arch = "x86_64")]
        assert_eq!((host_abi(), host_arch()), ("x86_64", "x64"));
    }

    #[test]
    fn free_space_uses_existing_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("a/b/c");
        assert!(free_space(&missing).unwrap() > 0);
        let err = ensure_free_space(&missing, u64::MAX).unwrap_err();
        assert_eq!(err.code, ErrorCode::Sdk);
    }
}
