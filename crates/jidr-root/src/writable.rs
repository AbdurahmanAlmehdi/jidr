//! Writable `/system` for shell-root (adb) devices (#25).
//!
//! The emulator's `-writable-system` launch option boots a writable copy of
//! the system image. After `adb root`, `adb remount` mounts `/system`
//! read-write (an overlayfs). The first remount on a device turns verity off
//! and asks for a reboot; after that reboot (and `adb root` again) a second
//! remount succeeds. Changing the setting forces a cold boot (D29):
//! a quick-boot snapshot taken with one system image never resumes on the
//! other.
//!
//! Revert: `adb enable-verity` tears the overlay down while the device still
//! runs, then the device cold-boots without `-writable-system`, so it sees the
//! read-only stock system image again. A device that is reverted while
//! stopped gets the same teardown at its next adb-root boot if an overlay is
//! still mounted.
//!
//! Supported from API 31: on the API 30 image CI runs, the first remount
//! reports success while `/system` stays read-only, and the emulator never
//! comes back from the reboot that would fix it, so older images are refused.
//!
//! Sources: the emulator's public command-line docs (`-writable-system`) and
//! AOSP's public adb / overlayfs docs (`adb remount`, `adb enable-verity`).

use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::provider::CallCtx;
use jidr_core::{Device, DeviceStatus, JidrError, JidrResult, RootMode};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{adb_ready, magisk, verify};

/// The oldest API level with a writable `/system`.
pub(crate) const MIN_API: u32 = 31;
/// How long the guest may take to reboot after verity is turned off.
const REBOOT_TIMEOUT: Duration = Duration::from_secs(240);
/// How long adbd may take to answer as root again.
const ROOT_TIMEOUT: Duration = Duration::from_secs(60);
/// A marker on `/dev` (a tmpfs, so a reboot clears it): tells the old boot
/// from the new one while waiting for `sys.boot_completed`.
const REBOOT_MARKER: &str = "/dev/.jidr-reboot-pending";
/// Prints `writable` when root can create and delete a file under `/system`.
const WRITABLE_PROBE: &str = "touch /system/.jidr-rw && rm /system/.jidr-rw && echo writable";
/// Prints `overlay` when a system partition is mounted through overlayfs (the
/// `adb remount` overlay).
const OVERLAY_PROBE: &str =
    "grep -qE '^overlay /(system|vendor|product|system_ext)? ' /proc/mounts && echo overlay";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// Device name or id (must be in `adb` root mode, on API 31 or later, to enable).
    pub device: String,
    /// `true`: boot with a writable `/system` (`-writable-system` +
    /// `adb remount`). `false`: back to the read-only stock system image.
    pub enabled: bool,
}

/// Why `device` can't get a writable `/system`, if it can't.
pub(crate) fn ineligible(device: &Device) -> Option<JidrError> {
    if !device.image.allows_adb_root() {
        return Some(JidrError::usage(format!(
            "{} images don't allow adb root, so /system can't be made writable; use a google_apis, default or aosp_atd image",
            device.image.tag
        )));
    }
    if device.image.api < MIN_API {
        return Some(JidrError::usage(format!(
            "{} runs API {}; writable /system needs API {MIN_API} or later",
            device.name, device.image.api
        )));
    }
    match device.root.mode {
        RootMode::Adb => None,
        RootMode::None => Some(JidrError::usage(format!(
            "{} has no root; writable /system needs shell (adb) root: run `jidr root set {} adb` first",
            device.name, device.name
        ))),
        RootMode::Magisk => Some(JidrError::usage(format!(
            "{} is Full-root (Magisk); writable /system is for shell (adb) root only: use a Magisk module to change system files",
            device.name
        ))),
    }
}

/// `root.writable_system`: stores the setting and, when the device is
/// running, cold-boots it at once to apply it.
pub(crate) async fn set(ctx: &CallCtx, p: SetParams) -> JidrResult<Value> {
    let device = ctx.daemon.device(&p.device).await?;
    // Turning it off always works, so a device can be reverted whatever its
    // mode; turning it on needs adb root.
    if p.enabled
        && let Some(e) = ineligible(&device)
    {
        return Err(e);
    }
    let enabled = p.enabled;
    if device.root.writable_system == enabled {
        return Ok(json!({
            "device": device.id,
            "writable_system": enabled,
            "changed": false,
            "applied": "unchanged",
        }));
    }
    let running = matches!(device.status, DeviceStatus::Running);
    if !enabled && running && device.root.mode == RootMode::Adb {
        ctx.report(None, "Dropping the /system overlay");
        drop_overlay_live(ctx, &device).await;
    }
    let device = ctx
        .daemon
        .update_device(
            device.id,
            Box::new(move |d: &mut Device| {
                d.root.writable_system = enabled;
                // D29: a snapshot taken on one system image never resumes on the other.
                d.launch.force_cold_boot = true;
            }),
        )
        .await?;
    if !running {
        return Ok(json!({
            "device": device.id,
            "writable_system": enabled,
            "changed": true,
            "applied": "next_boot",
        }));
    }
    ctx.cancel.check()?;
    // The boot hook roots adbd and, when enabled, remounts /system.
    let device = magisk::cold_boot(ctx, &device).await?;
    let adb = verify::device_adb(ctx, &device).await?;
    let system_writable = if enabled {
        // The hook only logs a failure; this makes the command report it.
        ensure_writable(ctx, &adb).await?;
        true
    } else {
        is_writable(&adb).await
    };
    Ok(json!({
        "device": device.id,
        "writable_system": enabled,
        "changed": true,
        "applied": "live",
        "system_writable": system_writable,
    }))
}

/// Boot hook for adb-root devices, after `adb root`: remount `/system`
/// read-write when the setting is on, drop a leftover overlay when it is off.
pub(crate) async fn on_booted(ctx: &CallCtx, device: &Device) -> JidrResult<()> {
    let adb = verify::device_adb(ctx, device).await?;
    if device.root.writable_system {
        return ensure_writable(ctx, &adb).await;
    }
    if has_overlay(&adb).await {
        // Reverted while stopped: the remount overlay outlived the flag.
        ctx.report(None, "Dropping a leftover /system overlay");
        let out = adb.raw(&["enable-verity"]).await?;
        if !out.success() {
            return Err(JidrError::root_failed(format!(
                "{}: adb enable-verity failed: {}",
                device.name,
                text(&out).trim()
            )));
        }
        reboot_and_reroot(ctx, &adb).await?;
    }
    Ok(())
}

/// Best effort, before a running device leaves writable `/system` (or adb
/// root): tear the overlay down while adbd is still root. The cold boot
/// without `-writable-system` follows either way.
pub(crate) async fn drop_overlay_live(ctx: &CallCtx, device: &Device) {
    let Ok(adb) = verify::device_adb(ctx, device).await else {
        return;
    };
    match adb.raw(&["enable-verity"]).await {
        Ok(out) if out.success() => {}
        Ok(out) => tracing::warn!(
            device = %device.name,
            "dropping the /system overlay: adb enable-verity: {}",
            text(&out).trim()
        ),
        Err(e) => tracing::warn!(device = %device.name, "dropping the /system overlay: {e}"),
    }
}

/// What `adb remount` asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Remount {
    Done,
    /// Verity was turned off; the remount needs a reboot first.
    NeedsReboot,
}

/// Reads `adb remount` output: success, a reboot request, or an error.
pub(crate) fn parse_remount(success: bool, output: &str) -> JidrResult<Remount> {
    let lower = output.to_lowercase();
    if lower.contains("remount succeeded") {
        return Ok(Remount::Done);
    }
    if lower.contains("reboot") {
        return Ok(Remount::NeedsReboot);
    }
    if success {
        return Ok(Remount::Done);
    }
    Err(JidrError::root_failed(format!(
        "adb remount failed: {}",
        output.trim()
    )))
}

fn text(out: &jidr_adb::Output) -> String {
    format!("{}{}", out.stdout, out.stderr)
}

/// Runs `adb remount`; returns what it asked for and its output.
async fn remount(adb: &Adb) -> JidrResult<(Remount, String)> {
    let out = adb.raw(&["remount"]).await?;
    let output = text(&out);
    Ok((parse_remount(out.success(), &output)?, output))
}

async fn is_writable(adb: &Adb) -> bool {
    matches!(adb.shell_status(WRITABLE_PROBE).await, Ok((_, out)) if out.trim() == "writable")
}

async fn has_overlay(adb: &Adb) -> bool {
    matches!(adb.shell_status(OVERLAY_PROBE).await, Ok((_, out)) if out.trim() == "overlay")
}

/// Remounts `/system` read-write unless it already is, then checks a file
/// can be written there. The first remount on a device may need a reboot
/// before `/system` becomes writable, whether or not its output says so, so
/// a remount that leaves `/system` read-only gets one reboot and a second
/// remount.
async fn ensure_writable(ctx: &CallCtx, adb: &Adb) -> JidrResult<()> {
    if is_writable(adb).await {
        return Ok(());
    }
    ctx.report(None, "Remounting /system read-write");
    let (first, _) = remount(adb).await?;
    if first == Remount::Done && is_writable(adb).await {
        return Ok(());
    }
    reboot_and_reroot(ctx, adb).await?;
    ctx.report(None, "Remounting /system read-write");
    let (second, output) = remount(adb).await?;
    if second == Remount::NeedsReboot {
        return Err(JidrError::root_failed(format!(
            "adb remount still asks for a reboot after rebooting once: {}",
            output.trim()
        )));
    }
    if is_writable(adb).await {
        Ok(())
    } else {
        Err(JidrError::root_failed(format!(
            "/system is still read-only after rebooting and running adb remount again; is the device booted with -writable-system? adb remount said: {}",
            output.trim()
        )))
    }
}

/// Reboots the guest (the emulator process keeps running), waits for the new
/// boot to complete, then roots adbd again.
async fn reboot_and_reroot(ctx: &CallCtx, adb: &Adb) -> JidrResult<()> {
    ctx.cancel.check()?;
    ctx.report(None, "Rebooting");
    adb.shell_status(&format!("touch {REBOOT_MARKER}")).await?;
    adb.raw(&["reboot"]).await?;
    adb_ready::until(
        adb,
        Some(&ctx.cancel),
        REBOOT_TIMEOUT,
        "after the reboot",
        &format!("[ -e {REBOOT_MARKER} ] && echo old || getprop sys.boot_completed"),
        |out| out.trim() == "1",
    )
    .await?;
    adb_ready::switch_adbd(adb, Some(&ctx.cancel), true, ROOT_TIMEOUT).await?;
    adb_ready::until(
        adb,
        Some(&ctx.cancel),
        ROOT_TIMEOUT,
        "after adb root",
        "id -u",
        |out| out.trim() == "0",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remount_output_is_read() {
        assert_eq!(
            parse_remount(true, "Using overlayfs for /system\nremount succeeded\n").unwrap(),
            Remount::Done
        );
        assert_eq!(
            parse_remount(
                true,
                "Successfully disabled verification\nNow reboot your device for settings to take effect\n"
            )
            .unwrap(),
            Remount::NeedsReboot
        );
        assert_eq!(parse_remount(true, "").unwrap(), Remount::Done);
        let e =
            parse_remount(false, "remount of /system failed: Read-only file system").unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::RootFailed);
        assert!(e.message.contains("Read-only"), "{e}");
    }
}
