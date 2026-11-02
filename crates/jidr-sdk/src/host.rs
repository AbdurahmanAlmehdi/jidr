//! Facts about the host machine: ABI, hypervisor, free disk space, memory.

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

/// Host RAM in MB (D42 budget).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct HostMemory {
    pub total_mb: u64,
    /// What the OS could hand out now without swapping (free + reclaimable).
    pub available_mb: u64,
}

const MB: u64 = 1024 * 1024;

/// Total and available RAM: `sysctl hw.memsize` + `vm_stat` on macOS,
/// `/proc/meminfo` on Linux, `Win32_OperatingSystem` on Windows (the same
/// numbers as `GlobalMemoryStatusEx`, without `unsafe`).
pub fn memory() -> JidrResult<HostMemory> {
    #[cfg(target_os = "macos")]
    {
        let total = command_out("sysctl", &["-n", "hw.memsize"])?;
        let vm = command_out("vm_stat", &[])?;
        parse_macos(&total, &vm)
    }
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo")
            .ctx(ErrorCode::Unexpected, "reading /proc/meminfo")?;
        parse_meminfo(&text)
    }
    #[cfg(windows)]
    {
        let out = command_out(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "$o = Get-CimInstance Win32_OperatingSystem; \"$($o.TotalVisibleMemorySize) $($o.FreePhysicalMemory)\"",
            ],
        )?;
        parse_windows(&out)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        Err(JidrError::unexpected("host memory is unknown on this OS"))
    }
}

#[cfg(any(target_os = "macos", windows))]
fn command_out(program: &str, args: &[&str]) -> JidrResult<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .ctx(ErrorCode::Unexpected, format!("running {program}"))?;
    if !out.status.success() {
        return Err(JidrError::unexpected(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn bad(what: &str) -> JidrError {
    JidrError::unexpected(format!("unreadable {what}"))
}

/// `sysctl -n hw.memsize` (bytes) and `vm_stat` (pages). Available is free +
/// inactive + speculative + purgeable pages, as Activity Monitor counts it.
pub fn parse_macos(memsize: &str, vm_stat: &str) -> JidrResult<HostMemory> {
    let total: u64 = memsize.trim().parse().map_err(|_| bad("hw.memsize"))?;
    let page: u64 = vm_stat
        .lines()
        .next()
        .and_then(|l| l.split("page size of ").nth(1))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| bad("vm_stat page size"))?;
    let pages = |key: &str| -> u64 {
        vm_stat
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.trim().trim_end_matches('.').parse().ok())
            .unwrap_or(0)
    };
    let free = [
        "Pages free:",
        "Pages inactive:",
        "Pages speculative:",
        "Pages purgeable:",
    ]
    .iter()
    .map(|k| pages(k))
    .sum::<u64>();
    Ok(HostMemory {
        total_mb: total / MB,
        available_mb: (free * page / MB).min(total / MB),
    })
}

/// `/proc/meminfo` (kB).
pub fn parse_meminfo(text: &str) -> JidrResult<HostMemory> {
    let kb = |key: &str| -> Option<u64> {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.trim().trim_end_matches("kB").trim().parse().ok())
    };
    let total = kb("MemTotal:").ok_or_else(|| bad("MemTotal"))?;
    let available = kb("MemAvailable:")
        .or_else(|| Some(kb("MemFree:")? + kb("Cached:").unwrap_or(0)))
        .ok_or_else(|| bad("MemAvailable"))?;
    Ok(HostMemory {
        total_mb: total / 1024,
        available_mb: available / 1024,
    })
}

/// `"<TotalVisibleMemorySize> <FreePhysicalMemory>"` (kB).
pub fn parse_windows(text: &str) -> JidrResult<HostMemory> {
    let mut n = text
        .split_whitespace()
        .map(|v| v.parse::<u64>().map_err(|_| bad("Win32_OperatingSystem")));
    let total = n.next().ok_or_else(|| bad("Win32_OperatingSystem"))??;
    let free = n.next().ok_or_else(|| bad("Win32_OperatingSystem"))??;
    Ok(HostMemory {
        total_mb: total / 1024,
        available_mb: free / 1024,
    })
}

/// Host memory a process and its direct children use now, in MB: the
/// emulator launcher runs QEMU as a child. macOS counts the physical
/// footprint (`top`; RSS misses guest RAM mapped through HVF), Linux `VmRSS`
/// (KVM guest RAM is in QEMU's RSS). `None` when it can't be measured.
pub fn process_footprint_mb(pid: u32) -> Option<u64> {
    let mut pids = vec![pid];
    if let Ok(out) = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    {
        pids.extend(
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .filter_map(|p| p.parse::<u32>().ok()),
        );
    }
    footprint_of(&pids)
}

#[cfg(target_os = "macos")]
fn footprint_of(pids: &[u32]) -> Option<u64> {
    let mut args = vec![
        "-l".to_string(),
        "1".into(),
        "-stats".into(),
        "pid,mem".into(),
    ];
    for p in pids {
        args.extend(["-pid".into(), p.to_string()]);
    }
    let out = Command::new("top").args(&args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let sum: u64 = text
        .lines()
        .filter_map(|l| {
            let mut cols = l.split_whitespace();
            let pid: u32 = cols.next()?.parse().ok()?;
            pids.contains(&pid).then(|| parse_top_mem(cols.next()?))?
        })
        .sum();
    (sum > 0).then_some(sum)
}

#[cfg(target_os = "linux")]
fn footprint_of(pids: &[u32]) -> Option<u64> {
    let sum: u64 = pids
        .iter()
        .filter_map(|p| {
            let status = std::fs::read_to_string(format!("/proc/{p}/status")).ok()?;
            let kb: u64 = status
                .lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))?
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .ok()?;
            Some(kb / 1024)
        })
        .sum();
    (sum > 0).then_some(sum)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn footprint_of(_pids: &[u32]) -> Option<u64> {
    None
}

/// `top`'s MEM column (`4643M`, `812K+`, `2G-`) in MB.
pub fn parse_top_mem(v: &str) -> Option<u64> {
    let v = v.trim_end_matches(['+', '-']);
    let (num, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit() && c != '.')?);
    let n: f64 = num.parse().ok()?;
    let mb = match unit {
        "B" => n / (1024.0 * 1024.0),
        "K" => n / 1024.0,
        "M" => n,
        "G" => n * 1024.0,
        "T" => n * 1024.0 * 1024.0,
        _ => return None,
    };
    Some(mb.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VM_STAT: &str = "\
Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                                     4495.
Pages active:                                 167909.
Pages inactive:                               152642.
Pages speculative:                             13916.
Pages throttled:                                   0.
Pages wired down:                             233989.
Pages purgeable:                                   8.
\"Translation faults\":                     2844410524.
";

    #[test]
    fn host_memory_parsers() {
        let m = parse_macos("17179869184\n", VM_STAT).unwrap();
        assert_eq!(m.total_mb, 16384);
        // (4495 + 152642 + 13916 + 8) pages × 16 KiB.
        assert_eq!(m.available_mb, 2672);
        assert!(parse_macos("x", VM_STAT).is_err());
        assert!(parse_macos("1", "no header").is_err());

        let l = parse_meminfo(
            "MemTotal:       16314020 kB\nMemFree:          812344 kB\nMemAvailable:    9876544 kB\n",
        )
        .unwrap();
        assert_eq!((l.total_mb, l.available_mb), (15931, 9645));
        let old = parse_meminfo("MemTotal: 2048000 kB\nMemFree: 1024000 kB\nCached: 512000 kB\n")
            .unwrap();
        assert_eq!(old.available_mb, 1500, "kernels without MemAvailable");
        assert!(parse_meminfo("").is_err());

        let w = parse_windows("16658228 8123456\r\n").unwrap();
        assert_eq!((w.total_mb, w.available_mb), (16267, 7933));
        assert!(parse_windows("16658228").is_err());
    }

    #[test]
    fn top_mem_units() {
        assert_eq!(parse_top_mem("4643M"), Some(4643));
        assert_eq!(parse_top_mem("812K+"), Some(1));
        assert_eq!(parse_top_mem("2G-"), Some(2048));
        assert_eq!(parse_top_mem("1.5G"), Some(1536));
        assert_eq!(parse_top_mem("-"), None);
        assert_eq!(parse_top_mem("12"), None);
    }

    #[test]
    fn host_memory_probe_works_here() {
        let m = memory().unwrap();
        assert!(m.total_mb >= 1024, "{m:?}");
        assert!(m.available_mb <= m.total_mb, "{m:?}");
        assert!(
            process_footprint_mb(std::process::id()).is_some(),
            "our own footprint"
        );
    }

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
