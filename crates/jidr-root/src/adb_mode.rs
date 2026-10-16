//! Shell root through adbd: `adb root` / `adb unroot`, then wait for adbd to
//! come back with the expected uid. Works on `default`, `google_apis` and
//! `aosp_atd` images; Play Store images refuse it ("production builds").

use std::time::Duration;

use jidr_core::provider::CallCtx;
use jidr_core::{Device, JidrError, JidrResult, RootHealth, RootMode};

use crate::verify::{self, Verification};

/// How long adbd may take to restart under its new uid.
const RESTART_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the shell may take to answer before the switch.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(500);

/// Switches a running device's adbd to `mode` and verifies it. On failure the
/// device's root health is set to `failed`.
pub(crate) async fn apply(
    ctx: &CallCtx,
    device: &Device,
    mode: RootMode,
) -> JidrResult<Verification> {
    match switch(ctx, device, mode).await {
        Ok(v) => Ok(v),
        Err(e) => {
            let _ = ctx
                .daemon
                .update_device(device.id, Box::new(|d| d.root.health = RootHealth::Failed))
                .await;
            Err(e)
        }
    }
}

async fn switch(ctx: &CallCtx, device: &Device, mode: RootMode) -> JidrResult<Verification> {
    if mode == RootMode::Magisk {
        return Err(JidrError::usage(
            "Full root (Magisk) isn't applied through adbd",
        ));
    }
    let adb = verify::device_adb(ctx, device).await?;
    crate::adb_ready::switch_adbd(
        &adb,
        Some(&ctx.cancel),
        mode == RootMode::Adb,
        READY_TIMEOUT,
    )
    .await?;
    // adbd restarts: the device drops off and comes back. Poll until the shell
    // answers with the uid the mode promises, or time out.
    let deadline = tokio::time::Instant::now() + RESTART_TIMEOUT;
    let mut last = None;
    loop {
        ctx.cancel.check()?;
        match verify::probe(&adb, mode).await {
            Ok(uid) if verify::uid_matches(mode, uid) => {
                return verify::record(ctx, device, mode, uid, None).await;
            }
            Ok(uid) => last = Some(uid),
            Err(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return match last {
                Some(uid) => verify::record(ctx, device, mode, uid, None).await,
                None => Err(JidrError::root_failed(format!(
                    "{}: adbd didn't come back within {} s",
                    device.name,
                    RESTART_TIMEOUT.as_secs()
                ))),
            };
        }
        tokio::time::sleep(POLL).await;
    }
}
