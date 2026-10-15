//! Grant the adb shell automatic `su` (D22).
//!
//! On images that allow `adb root` (`google_apis`, `default`, `aosp_atd`) the
//! grant is zero-touch: root adbd, write the shell uid's policy row through
//! `magisk --sqlite`, then drop back to the normal shell. On Play Store images
//! `adb root` is refused, so the guided tap from ADR-0002 is used instead: run
//! `su -c id`, which raises Magisk's normal prompt, and wait for the person to
//! tap Grant.

use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::provider::CallCtx;
use jidr_core::{JidrError, JidrResult};

use crate::verify::parse_uid;

/// Android user id of the adb shell, whose `su` request we pre-approve.
pub const AID_SHELL: u32 = 2000;

/// Magisk's `SuPolicy` values; `Allow` pre-approves without a prompt.
const SU_POLICY_ALLOW: i32 = 2;

/// How long the guided tap waits for the person to tap Grant.
const GUIDED_TIMEOUT: Duration = Duration::from_secs(60);
/// Per-attempt `su -c id` timeout; a shorter window lets the total stay 60 s
/// while re-raising the prompt.
const GUIDED_ATTEMPT: Duration = Duration::from_secs(20);

const POLL: Duration = Duration::from_millis(500);

/// The places Magisk puts its binary, most likely first.
const MAGISK_BINARIES: &[&str] = &[
    "/debug_ramdisk/magisk",
    "/sbin/magisk",
    "/data/adb/magisk/magisk",
];

/// Finds the `magisk` binary on the device.
async fn magisk_binary(adb: &Adb) -> JidrResult<String> {
    for path in MAGISK_BINARIES {
        let (_, out) = adb
            .shell_status(&format!("[ -x {path} ] && echo yes"))
            .await?;
        if out.contains("yes") {
            return Ok((*path).to_string());
        }
    }
    Err(JidrError::root_failed(
        "the magisk binary was not found on the device; Magisk has not initialised",
    ))
}

/// Waits until the shell answers `id` with the uid `mode` promises. adbd
/// restarts for `adb root`/`unroot`, and its transport can stay offline
/// until reconnected (CI: "adbd did not settle to the expected uid").
async fn wait_adbd(adb: &Adb, want_root: bool) -> JidrResult<()> {
    crate::adb_ready::until(
        adb,
        None,
        Duration::from_secs(60),
        "adbd did not settle to the expected uid",
        "id -u",
        |out| (out.trim() == "0") == want_root,
    )
    .await
}

/// Runs `adb root` unless the shell is already root.
pub async fn ensure_adbd_root(adb: &Adb) -> JidrResult<()> {
    match adb.shell_status("id -u").await {
        Ok((_, out)) if out.trim() == "0" => return Ok(()),
        _ => {}
    }
    adb.root().await?;
    wait_adbd(adb, true).await
}

/// Runs `adb unroot` so later checks exercise Magisk's `su`, not adbd's root.
pub async fn drop_adbd_root(adb: &Adb) -> JidrResult<()> {
    adb.unroot().await?;
    wait_adbd(adb, false).await
}

/// Writes and verifies a su policy row for `uid` through `magisk --sqlite`.
async fn set_su_policy(adb: &Adb, bin: &str, uid: u32, policy: i32) -> JidrResult<()> {
    let sql = format!(
        "REPLACE INTO policies (uid, policy, until, logging, notification) VALUES ({uid}, {policy}, 0, 1, 1)"
    );
    adb.shell(&format!("{bin} --sqlite \"{sql}\""))
        .await
        .map_err(|e| JidrError::root_failed(format!("writing the su policy for uid {uid}: {e}")))?;

    let out = adb
        .shell(&format!(
            "{bin} --sqlite \"SELECT policy FROM policies WHERE uid={uid}\""
        ))
        .await
        .map_err(|e| {
            JidrError::root_failed(format!("verifying the su policy for uid {uid}: {e}"))
        })?;
    if !out.contains(&format!("policy={policy}")) {
        return Err(JidrError::root_failed(format!(
            "the su policy for uid {uid} was not stored (device reports {})",
            out.trim()
        )));
    }
    Ok(())
}

/// Zero-touch grant: root adbd, pre-approve the shell, drop back to shell.
pub async fn grant_shell_zero_touch(adb: &Adb) -> JidrResult<()> {
    ensure_adbd_root(adb).await?;
    let bin = magisk_binary(adb).await?;
    set_su_policy(adb, &bin, AID_SHELL, SU_POLICY_ALLOW).await?;
    drop_adbd_root(adb).await?;
    Ok(())
}

/// The guided tap from ADR-0002: run `su -c id` (which raises Magisk's prompt)
/// and wait up to 60 s for the person to tap Grant. Timed out or denied, it
/// fails with `data.state = "awaiting_user_grant"` and the caller offers
/// "Try again" and "Revert".
pub async fn grant_shell_guided(ctx: &CallCtx, adb: &Adb) -> JidrResult<()> {
    ctx.report(
        None,
        "Magisk will ask to grant root to Shell. Tap Grant in the device window.",
    );
    poll_for_grant(&ctx.cancel, adb, GUIDED_TIMEOUT).await
}

/// Polls `su -c id` until it reports uid 0 or `timeout` elapses.
async fn poll_for_grant(
    cancel: &jidr_core::Cancel,
    adb: &Adb,
    timeout: Duration,
) -> JidrResult<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        cancel.check()?;
        let attempt = adb.clone().with_timeout(GUIDED_ATTEMPT);
        match attempt.shell_status("su -c id").await {
            Ok((_, out)) if parse_uid(&out) == Some(0) => return Ok(()),
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(JidrError::root_failed(
                "root grant to Shell was denied or timed out; tap Grant and try again, or revert",
            )
            .with_data(serde_json::json!({ "state": "awaiting_user_grant" })));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Grants the shell su, choosing the zero-touch path when `adb root` works and
/// the guided tap otherwise.
pub async fn grant_shell(ctx: &CallCtx, adb: &Adb, adb_root_allowed: bool) -> JidrResult<()> {
    if adb_root_allowed {
        grant_shell_zero_touch(adb).await
    } else {
        grant_shell_guided(ctx, adb).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_exec(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// A fake adb whose `su -c id` never grants root (the prompt stays pending).
    fn adb_that_never_grants(dir: &Path) -> Adb {
        let adb = dir.join("adb");
        write_exec(
            &adb,
            "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\ncase \"$1\" in\n  shell)\n    case \"$2\" in\n      \"su -c id\") echo denied >&2; exit 1;;\n      \"id -u\") echo 2000;;\n      *) echo ok;;\n    esac;;\n  *) echo ok;;\nesac\n",
        );
        Adb::new(adb)
    }

    #[test]
    fn policy_constants_match_magisk() {
        assert_eq!(SU_POLICY_ALLOW, 2);
        assert_eq!(AID_SHELL, 2000);
    }

    #[tokio::test]
    async fn guided_grant_times_out_as_awaiting_user_grant() {
        let dir = tempfile::tempdir().unwrap();
        let adb = adb_that_never_grants(dir.path());
        let cancel = jidr_core::Cancel::new();
        let err = poll_for_grant(&cancel, &adb, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert_eq!(err.code, jidr_core::ErrorCode::RootFailed);
        assert_eq!(err.data.as_ref().unwrap()["state"], "awaiting_user_grant");
    }

    #[tokio::test]
    async fn guided_grant_succeeds_once_su_answers_root() {
        let dir = tempfile::tempdir().unwrap();
        let adb = dir.path().join("adb");
        write_exec(
            &adb,
            "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\ncase \"$1\" in\n  shell) echo \"uid=0(root) gid=0(root)\";;\n  *) echo ok;;\nesac\n",
        );
        let cancel = jidr_core::Cancel::new();
        poll_for_grant(&cancel, &Adb::new(adb), Duration::from_secs(5))
            .await
            .unwrap();
    }
}
