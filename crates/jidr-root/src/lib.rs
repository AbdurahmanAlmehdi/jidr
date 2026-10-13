//! CommandProvider for root modes: adb root, Magisk, verify and revert.
//!
//! Commands (plan, "Root engine"):
//! - `root.status`: the stored root state of a device;
//! - `root.verify`: checks the live uid against the mode and records the result;
//! - `root.set`: switches the mode, live when the device is running.
//!
//! adb-mode devices get `adb root` after every boot through
//! [`CommandProvider::on_booted`]. Magisk mode lands in Phase 9d.

mod adb_mode;
mod verify;

use std::sync::Arc;

use async_trait::async_trait;
use jidr_core::provider::{CallCtx, CommandProvider, CommandSpec, DeviceParams, parse_params};
use jidr_core::{Device, DeviceStatus, JidrError, JidrResult, RootHealth, RootMode};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub use verify::Verification;

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
            other => Err(JidrError::not_found(format!("unknown method {other}"))),
        }
    }

    async fn on_booted(&self, ctx: &CallCtx, device: &Device) -> JidrResult<()> {
        match device.root.mode {
            RootMode::Adb => adb_mode::apply(ctx, device, RootMode::Adb)
                .await
                .map(|_| ()),
            RootMode::None | RootMode::Magisk => Ok(()),
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
    match p.mode {
        RootMode::Magisk => {
            return Err(JidrError::usage(
                "Full root (Magisk) isn't available yet; use mode none or adb",
            ));
        }
        RootMode::Adb if !device.image.allows_adb_root() => {
            return Err(JidrError::usage(format!(
                "{} images don't allow adb root; use mode none, or magisk once it's available",
                device.image.tag
            )));
        }
        RootMode::Adb | RootMode::None => {}
    }
    if device.root.mode == RootMode::Magisk {
        return Err(JidrError::usage(
            "reverting Full root (Magisk) isn't available yet",
        ));
    }

    let changed = device.root.mode != p.mode;
    let device = if changed {
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
    }))
}

fn mode_label(mode: RootMode) -> &'static str {
    match mode {
        RootMode::None => "no",
        RootMode::Adb => "shell (adb)",
        RootMode::Magisk => "full (Magisk)",
    }
}
