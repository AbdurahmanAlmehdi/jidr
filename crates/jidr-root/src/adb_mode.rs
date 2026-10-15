//! Shell root through adbd: `adb root` / `adb unroot`, then wait for adbd to
//! come back with the expected uid. Works on `default`, `google_apis` and
//! `aosp_atd` images; Play Store images refuse it ("production builds").

use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::provider::CallCtx;
use jidr_core::{Cancel, Device, JidrError, JidrResult, RootHealth, RootMode};

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
    toggle(&adb, Some(&ctx.cancel), mode, READY_TIMEOUT).await?;
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

/// `adb root` (`mode` adb) or `adb unroot` (none), once the shell answers.
///
/// Regression (CI, API 35): right after the boot the transport was stale and
/// `adb root` failed with "unable to connect for root: closed". So wait for
/// the shell first, and when the transport still drops during the call, wait
/// again and retry once.
async fn toggle(
    adb: &Adb,
    cancel: Option<&Cancel>,
    mode: RootMode,
    ready: Duration,
) -> JidrResult<()> {
    let wait = || {
        crate::adb_ready::until(
            adb,
            cancel,
            ready,
            "before switching adbd",
            "echo ok",
            |o| o.trim() == "ok",
        )
    };
    let run = || async {
        if mode == RootMode::Adb {
            adb.root().await
        } else {
            adb.unroot().await
        }
    };
    wait().await?;
    match run().await {
        Err(e) if transport_dropped(&e) => {
            wait().await?;
            run().await
        }
        r => r,
    }
}

/// adb lost the device mid-call (as opposed to refusing the switch).
fn transport_dropped(e: &JidrError) -> bool {
    let m = e.message.to_lowercase();
    ["unable to connect", "closed", "offline", "no devices"]
        .iter()
        .any(|s| m.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake adb whose shell answers, and whose first `adb root` fails the
    /// way the API 35 CI run did.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_closed_transport_during_adb_root_is_retried() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tried = dir.path().join("tried");
        let bin = dir.path().join("adb");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\n\
                 case \"$1\" in\n\
                 shell) echo ok;;\n\
                 root) if [ -f {f} ]; then echo 'restarting adbd as root'; \
                 else touch {f}; echo 'adb: unable to connect for root: closed' >&2; exit 1; fi;;\n\
                 esac\n",
                f = tried.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = Adb::new(&bin).device("emulator-5554");
        let r = toggle(&adb, None, RootMode::Adb, Duration::from_secs(5)).await;
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn a_refused_switch_is_not_retried() {
        assert!(transport_dropped(&JidrError::sdk(
            "adb root failed: adb: unable to connect for root: closed"
        )));
        assert!(!transport_dropped(&JidrError::root_failed(
            "adb root: adbd cannot run as root in production builds"
        )));
    }
}
