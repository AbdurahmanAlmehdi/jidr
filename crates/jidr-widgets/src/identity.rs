//! Identifiers widget (#19): the device's Android ID and IMEI.
//!
//! `identity.get` reads both over adb and works without root.
//! `identity.set` changes the Android ID as apps see it: every installed
//! app's SSAID, which is what apps read since Android 8, through the rooted
//! on-device helper (ADR-0003, `jidr_helper::identity`), then the global
//! `android_id` setting. Writing the SSAID restarts the Android framework,
//! so the command waits for it to come back. Both live in `/data`: they
//! survive cold boots and are part of snapshots.
//!
//! The IMEI is read only. The emulated modem's IMEI is a constant, and the
//! guest's SELinux policy leaves no way to put a relay between the radio
//! service and the modem (see docs/root.md, "Identifiers").

use std::time::{Duration, Instant};

use jidr_adb::Adb;
use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{JidrError, JidrResult};
use jidr_helper::host::Helper;
use jidr_helper::identity::check_android_id;
use jidr_helper::rpc::AndroidIdSet;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::device::target;

/// How long the helper may take to stop the framework, rewrite the SSAID
/// table and start it again (`abx2xml` and `cmd package` are slow on a busy
/// host).
const HELPER_TIMEOUT: Duration = Duration::from_secs(180);

/// How long the framework may take to come back after that.
const RESTART_TIMEOUT: Duration = Duration::from_secs(240);

/// How long telephony may take to answer again after the restart.
const TELEPHONY_TIMEOUT: Duration = Duration::from_secs(60);

const POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// Device name or id (Shell root or Full root).
    pub device: String,
    /// New Android ID: 16 hexadecimal digits. Every installed app reads it
    /// (its SSAID), as does `settings get secure android_id`. Restarts the
    /// Android framework.
    pub android_id: String,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<DeviceParams>(
            "identity.get",
            "The device's Android ID and IMEI (no root needed)",
        ),
        CommandSpec::new::<SetParams>(
            "identity.set",
            "Change the device's Android ID as every installed app reads it (needs Shell or Full root; restarts the Android framework; kept across cold boots)",
        )
        .long_running()
        .locks_device(),
    ]
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "identity.get" => {
            let p: DeviceParams = parse_params(params)?;
            get(&target(ctx, &p.device).await?.adb()).await
        }
        "identity.set" => set(ctx, parse_params(params)?).await,
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

/// Checks the new Android ID before anything touches the device.
pub fn check(android_id: &str) -> JidrResult<String> {
    check_android_id(android_id)
        .map_err(|e| JidrError::usage(e.message).with_data(json!({ "field": "android_id" })))
}

/// The IMEI in `service call iphonesubinfo 1 …`'s parcel dump: a status
/// word, a length, then UTF-16 text shown in quotes, e.g.
/// `0x00000000: 00000000 0000000f 00360038 00340037 '........8.6.7.4.'`.
pub fn parse_parcel_string(dump: &str) -> Option<String> {
    let mut text = String::new();
    for line in dump.lines() {
        if let (Some(a), Some(b)) = (line.find('\''), line.rfind('\''))
            && b > a
        {
            text.push_str(&line[a + 1..b]);
        }
    }
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// The IMEI telephony reports (`IPhoneSubInfo.getDeviceId`, transaction 1
/// from API 30 to 36), when telephony runs.
async fn read_imei(adb: &Adb) -> Option<String> {
    adb.shell("service call iphonesubinfo 1 s16 com.android.shell")
        .await
        .ok()
        .and_then(|d| parse_parcel_string(&d))
}

/// Reads the global Android ID and the IMEI (null when telephony has none).
pub async fn get(adb: &Adb) -> JidrResult<Value> {
    let android_id = adb.shell("settings get secure android_id").await?;
    let android_id = android_id.trim();
    let imei = read_imei(adb).await;
    Ok(json!({
        "android_id": (!android_id.is_empty() && android_id != "null").then_some(android_id),
        "imei": imei,
    }))
}

pub async fn set(ctx: &CallCtx, p: SetParams) -> JidrResult<Value> {
    let id = check(&p.android_id)?;
    let t = target(ctx, &p.device).await?;
    let adb = t.adb();
    ctx.report(Some(0.05), "Starting the on-device helper");
    let mut helper = Helper::for_device(ctx, &t.device).await?;
    let before = system_server(&adb).await;
    let had_imei = read_imei(&adb).await.is_some();
    ctx.report(
        Some(0.2),
        "Giving every app the new Android ID and restarting Android",
    );
    helper
        .call_timeout(
            &AndroidIdSet {
                android_id: id.clone(),
            },
            HELPER_TIMEOUT,
        )
        .await?;
    ctx.report(Some(0.4), "Waiting for Android to start again");
    wait_restarted(ctx, &adb, before).await?;
    // The global value, once SettingsProvider runs again (the shell may
    // write it, so this needs no root).
    let (status, out) = adb
        .shell_status(&format!("settings put secure android_id {id}"))
        .await?;
    if status != 0 {
        return Err(JidrError::unexpected(format!(
            "settings put secure android_id: {}",
            out.trim()
        )));
    }
    if had_imei {
        ctx.report(Some(0.9), "Waiting for telephony");
        wait_telephony(ctx, &adb).await;
    }
    get(&adb).await
}

/// system_server's pid, when it runs.
async fn system_server(adb: &Adb) -> Option<String> {
    adb.shell("pidof system_server")
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Waits until a new system_server runs and the boot animation has ended.
async fn wait_restarted(ctx: &CallCtx, adb: &Adb, before: Option<String>) -> JidrResult<()> {
    let start = Instant::now();
    loop {
        ctx.cancel.check()?;
        let now = system_server(adb).await;
        if now.is_some() && now != before {
            let done = adb
                .shell("getprop service.bootanim.exit")
                .await
                .is_ok_and(|s| s.trim() == "1");
            if done {
                return Ok(());
            }
        }
        if start.elapsed() > RESTART_TIMEOUT {
            return Err(JidrError::unexpected(format!(
                "Android didn't finish starting again within {} s",
                RESTART_TIMEOUT.as_secs()
            )));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Waits (best effort) until telephony reports an IMEI again, so the
/// result shows it.
async fn wait_telephony(ctx: &CallCtx, adb: &Adb) {
    let start = Instant::now();
    while start.elapsed() < TELEPHONY_TIMEOUT && ctx.cancel.check().is_ok() {
        if read_imei(adb).await.is_some() {
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_ids_are_checked() {
        assert_eq!(check("0123456789ABCDEF").unwrap(), "0123456789abcdef");
        let e = check("xyz").unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::Usage);
        assert_eq!(e.data.unwrap()["field"], "android_id");
    }

    #[test]
    fn imei_is_read_from_the_parcel() {
        // As API 35 prints it.
        let dump = "Result: Parcel(\t\n\
            0x00000000: 00000000 0000000f 00360038 00340037 '........8.6.7.4.'\n\
            0x00000010: 00300030 00320030 00300032 00370034 '0.0.0.2.2.0.4.7.'\n\
            0x00000020: 00390031 00000039                   '1.9.9...        ')\n";
        assert_eq!(
            parse_parcel_string(dump).as_deref(),
            Some("867400022047199")
        );
        assert_eq!(
            parse_parcel_string("Result: Parcel(00000000 ffffffff   '........')"),
            None
        );
    }
}
