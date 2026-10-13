//! Root verification: compare the live uid of the adb shell (or of `su`)
//! with what the device's root mode promises, and record the result.

use std::time::Duration;

use chrono::{DateTime, Utc};
use jidr_adb::Adb;
use jidr_core::provider::CallCtx;
use jidr_core::{Device, JidrError, JidrResult, RootHealth, RootMode};
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

/// The result of one verification.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Verification {
    pub device: Uuid,
    pub mode: RootMode,
    /// The command that was run in `adb shell`.
    pub command: String,
    /// The uid it reported.
    pub uid: u32,
    pub ok: bool,
    pub health: RootHealth,
    pub verified_at: DateTime<Utc>,
}

/// What `adb shell` runs to check `mode`.
pub(crate) fn probe_command(mode: RootMode) -> &'static str {
    match mode {
        RootMode::Magisk => "su -c id",
        RootMode::None | RootMode::Adb => "id",
    }
}

/// `none` must not be root; `adb` and `magisk` must be.
pub(crate) fn uid_matches(mode: RootMode, uid: u32) -> bool {
    match mode {
        RootMode::None => uid != 0,
        RootMode::Adb | RootMode::Magisk => uid == 0,
    }
}

/// The uid in `id` output: `uid=0(root) gid=0(root) …`.
pub(crate) fn parse_uid(output: &str) -> Option<u32> {
    let rest = &output[output.find("uid=")? + 4..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// An `adb` aimed at a running device.
pub(crate) async fn device_adb(ctx: &CallCtx, device: &Device) -> JidrResult<Adb> {
    let info = ctx.daemon.connect_info(device.id).await?;
    Ok(Adb::new(info.adb_path)
        .device(info.adb_serial)
        .with_timeout(Duration::from_secs(20)))
}

/// Runs the probe once and returns the uid.
pub(crate) async fn probe(adb: &Adb, mode: RootMode) -> JidrResult<u32> {
    let cmd = probe_command(mode);
    let (_, out) = adb.shell_status(cmd).await?;
    parse_uid(&out)
        .ok_or_else(|| JidrError::root_failed(format!("`{cmd}` printed no uid: {}", out.trim())))
}

/// Probes, stores `health` and `verified_at`, and fails with `root_failed`
/// when the uid doesn't match the mode.
pub(crate) async fn verify(ctx: &CallCtx, device: &Device) -> JidrResult<Verification> {
    let adb = device_adb(ctx, device).await?;
    let mode = device.root.mode;
    let uid = probe(&adb, mode).await?;
    record(ctx, device, mode, uid).await
}

pub(crate) async fn record(
    ctx: &CallCtx,
    device: &Device,
    mode: RootMode,
    uid: u32,
) -> JidrResult<Verification> {
    let ok = uid_matches(mode, uid);
    let health = if ok {
        RootHealth::Ok
    } else {
        RootHealth::Failed
    };
    let now = Utc::now();
    ctx.daemon
        .update_device(
            device.id,
            Box::new(move |d| {
                d.root.health = health;
                d.root.verified_at = Some(now);
            }),
        )
        .await?;
    let v = Verification {
        device: device.id,
        mode,
        command: probe_command(mode).to_string(),
        uid,
        ok,
        health,
        verified_at: now,
    };
    if ok {
        Ok(v)
    } else {
        Err(JidrError::root_failed(format!(
            "{}: `{}` reported uid {uid}, expected {}",
            device.name,
            v.command,
            if mode == RootMode::None {
                "non-root"
            } else {
                "0 (root)"
            }
        ))
        .with_data(serde_json::to_value(&v).unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uids_from_id_output() {
        assert_eq!(parse_uid("uid=0(root) gid=0(root) groups=0(root)"), Some(0));
        assert_eq!(
            parse_uid("uid=2000(shell) gid=2000(shell) groups=2000(shell),1004(input)\n"),
            Some(2000)
        );
        assert_eq!(
            parse_uid("/system/bin/sh: su: inaccessible or not found"),
            None
        );
        assert_eq!(parse_uid("uid=(x)"), None);
        assert_eq!(parse_uid(""), None);
    }

    #[test]
    fn modes_expect_the_right_uid() {
        assert!(uid_matches(RootMode::None, 2000));
        assert!(!uid_matches(RootMode::None, 0));
        assert!(uid_matches(RootMode::Adb, 0));
        assert!(!uid_matches(RootMode::Adb, 2000));
        assert!(uid_matches(RootMode::Magisk, 0));
        assert_eq!(probe_command(RootMode::Magisk), "su -c id");
        assert_eq!(probe_command(RootMode::Adb), "id");
    }
}
