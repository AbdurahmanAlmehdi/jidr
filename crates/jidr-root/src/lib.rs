//! CommandProvider for root modes: adb root, Magisk, verify and revert.
//!
//! Commands (plan, "Root engine"):
//! - `root.status`: the stored root state of a device;
//! - `root.verify`: checks the live uid (and Magisk version) against the mode;
//! - `root.set`: switches the mode, live when the device is running;
//! - `root.module.install`: installs a Magisk module zip on a Magisk device.
//!
//! adb-mode devices get `adb root` after every boot through
//! [`CommandProvider::on_booted`]; Magisk devices get a re-patch check.

mod adb_mode;
mod magisk;
mod verify;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use jidr_core::provider::{CallCtx, CommandProvider, CommandSpec, DeviceParams, parse_params};
use jidr_core::{Device, DeviceStatus, JidrError, JidrResult, RootHealth, RootMode};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub use verify::Verification;

/// Shown in `root.status` and `root.set` results so the UI and CLI agree.
pub const PLAY_INTEGRITY_NOTICE: &str = "Play Integrity cannot pass on any emulator; Zygisk/DenyList only hides root from local checks.";

/// The root provider, for `jidr daemon` to register (gap G8).
pub fn provider() -> Arc<dyn CommandProvider> {
    Arc::new(RootProvider)
}

#[derive(Debug, Default)]
pub struct RootProvider;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// Device name or id.
    pub device: String,
    /// `none`, `adb` (images that allow it) or `magisk`.
    pub mode: RootMode,
    /// SHA-256 of a Magisk APK already in the download cache (the CLI imports
    /// it with `--magisk <path>` before calling this); overrides the pinned
    /// release (D30, offline).
    #[serde(default)]
    pub magisk_sha256: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleInstallParams {
    /// Device name or id (must be in `magisk` mode).
    pub device: String,
    /// A local Magisk module zip.
    pub path: PathBuf,
}

#[async_trait]
impl CommandProvider for RootProvider {
    fn name(&self) -> &str {
        "root"
    }

    fn commands(&self) -> Vec<CommandSpec> {
        vec![
            CommandSpec::new::<DeviceParams>("root.status", "Show a device's root mode and health"),
            CommandSpec::new::<DeviceParams>(
                "root.verify",
                "Check a running device's root against its mode",
            )
            .locks_device(),
            CommandSpec::new::<SetParams>(
                "root.set",
                "Set a device's root mode (applied live if it is running; the next start is a cold boot)",
            )
            .locks_device()
            .long_running(),
            CommandSpec::new::<ModuleInstallParams>(
                "root.module.install",
                "Install a Magisk module zip and reboot (Full-root devices only)",
            )
            .locks_device()
            .long_running()
            .destructive(),
        ]
    }

    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
        match method {
            "root.status" => {
                let p: DeviceParams = parse_params(params)?;
                let device = ctx.daemon.device(&p.device).await?;
                Ok(status(&device))
            }
            "root.verify" => {
                let p: DeviceParams = parse_params(params)?;
                let device = ctx.daemon.device(&p.device).await?;
                ensure_running(&device)?;
                to_value(verify::verify(ctx, &device).await?)
            }
            "root.set" => set(ctx, parse_params(params)?).await,
            "root.module.install" => {
                let p: ModuleInstallParams = parse_params(params)?;
                let device = ctx.daemon.device(&p.device).await?;
                ensure_running(&device)?;
                magisk::install_module(ctx, &device, &p.path).await
            }
            other => Err(JidrError::not_found(format!("unknown method {other}"))),
        }
    }

    async fn on_booted(&self, ctx: &CallCtx, device: &Device) -> JidrResult<()> {
        match device.root.mode {
            RootMode::Adb => adb_mode::apply(ctx, device, RootMode::Adb)
                .await
                .map(|_| ()),
            RootMode::Magisk => magisk::on_booted(ctx, device).await,
            RootMode::None => Ok(()),
        }
    }
}

fn to_value<T: serde::Serialize>(v: T) -> JidrResult<Value> {
    serde_json::to_value(v).map_err(|e| JidrError::unexpected(format!("serializing result: {e}")))
}

fn status(device: &Device) -> Value {
    json!({
        "device": device.id,
        "name": device.name,
        "root": device.root,
        "adb_root_allowed": device.image.allows_adb_root(),
        "play_integrity": PLAY_INTEGRITY_NOTICE,
    })
}

fn is_running(device: &Device) -> bool {
    matches!(device.status, DeviceStatus::Running)
}

fn ensure_running(device: &Device) -> JidrResult<()> {
    if is_running(device) {
        Ok(())
    } else {
        Err(JidrError::usage(format!(
            "{} is not running; start it first",
            device.name
        )))
    }
}

async fn set(ctx: &CallCtx, p: SetParams) -> JidrResult<Value> {
    let device = ctx.daemon.device(&p.device).await?;
    if p.mode == RootMode::Adb && !device.image.allows_adb_root() {
        return Err(JidrError::usage(format!(
            "{} images don't allow adb root; use mode none, or magisk for full root",
            device.image.tag
        )));
    }
    if p.mode == RootMode::Magisk {
        return magisk::to_magisk(ctx, &device, p.magisk_sha256.as_deref()).await;
    }

    let changed = device.root.mode != p.mode;
    // Leaving Magisk: clear the patch and cold-boot before applying the target
    // mode, so `none` and `adb` behave exactly as they do today.
    let device = if device.root.mode == RootMode::Magisk {
        magisk::clear_patch(ctx, &device, p.mode).await?
    } else if changed {
        let mode = p.mode;
        ctx.daemon
            .update_device(
                device.id,
                Box::new(move |d: &mut Device| {
                    d.root.mode = mode;
                    d.root.health = RootHealth::Unverified;
                    d.root.verified_at = None;
                    // D29: a root change never resumes a snapshot taken under the old mode.
                    d.launch.force_cold_boot = true;
                }),
            )
            .await?
    } else {
        device
    };

    if !is_running(&device) {
        return Ok(json!({
            "device": device.id,
            "mode": device.root.mode,
            "changed": changed,
            "applied": "next_boot",
            "play_integrity": PLAY_INTEGRITY_NOTICE,
        }));
    }
    ctx.cancel.check()?;
    ctx.report(None, &format!("Applying {} root", mode_label(p.mode)));
    let verification = adb_mode::apply(ctx, &device, p.mode).await?;
    Ok(json!({
        "device": device.id,
        "mode": p.mode,
        "changed": changed,
        "applied": "live",
        "verification": verification,
        "play_integrity": PLAY_INTEGRITY_NOTICE,
    }))
}

fn mode_label(mode: RootMode) -> &'static str {
    match mode {
        RootMode::None => "no",
        RootMode::Adb => "shell (adb)",
        RootMode::Magisk => "full (Magisk)",
    }
}
