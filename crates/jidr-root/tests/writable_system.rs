//! Offline tests of `root.writable_system` (#25): fake backend, fake adb.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;
use jidr_core::{Device, ErrorCode, RootHealth, RootMode};
use serde_json::json;

#[tokio::test]
async fn command_is_registered_with_the_right_flags() {
    let env = Env::start().await;
    let spec = env
        .daemon
        .registry()
        .get("root.writable_system")
        .unwrap()
        .1
        .clone();
    assert!(spec.locks_device && spec.long_running && !spec.destructive);
    assert_eq!(spec.params["required"], json!(["device", "enabled"]));
}

#[tokio::test]
async fn enabling_a_stopped_device_applies_at_the_next_cold_boot() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    let r = env
        .call(
            "root.writable_system",
            json!({ "device": "phone", "enabled": true }),
        )
        .await
        .unwrap();
    assert_eq!(r["applied"], "next_boot");
    assert_eq!(r["changed"], true);
    let d = env.device("phone").await;
    assert!(d.root.writable_system);
    assert!(
        d.launch.force_cold_boot,
        "D29: the change forces a cold boot"
    );

    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    let (_, opts) = env.backend.launches().pop().unwrap();
    assert!(opts.writable_system, "launched with -writable-system");
    assert!(opts.cold_boot);
    // The boot hook: adb root, remount (asks for a reboot), reboot, adb root,
    // remount again.
    assert_eq!(env.adb_count("reboots"), 1);
    assert!(env.adb_file("system-rw").exists(), "/system is writable");
    assert_eq!(env.adbd_uid(), "0", "adbd is root again after the reboot");
    let d = env.device("phone").await;
    assert!(!d.launch.force_cold_boot);
    assert_eq!(d.root.health, RootHealth::Ok);

    // A later boot only remounts: verity is already off, no second reboot.
    env.call("device.stop", json!({ "device": "phone" }))
        .await
        .unwrap();
    env.reset_adbd("2000");
    std::fs::remove_file(env.adb_file("system-rw")).unwrap();
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert!(env.backend.launches().pop().unwrap().1.writable_system);
    assert_eq!(env.adb_count("reboots"), 1);
    assert!(env.adb_file("system-rw").exists());
}

/// Regression (#85 CI, API 30): the first `adb remount` reported success but
/// left `/system` read-only until a reboot, so `root.writable_system` failed
/// with `root_failed` instead of rebooting once and remounting.
#[tokio::test]
async fn a_remount_that_reports_success_but_stays_read_only_gets_a_reboot() {
    let env = Env::start().await;
    std::fs::write(env.adb_file("remount-says-done"), b"1").unwrap();
    env.create("phone", "35:google_apis", Some("adb")).await;
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();

    let r = env
        .call(
            "root.writable_system",
            json!({ "device": "phone", "enabled": true }),
        )
        .await
        .unwrap();
    assert_eq!(r["system_writable"], true);
    assert_eq!(env.adb_count("reboots"), 1, "rebooted once, then remounted");
    assert!(env.adb_file("system-rw").exists(), "/system is writable");
    assert_eq!(env.adbd_uid(), "0", "adbd is root again after the reboot");
}

#[tokio::test]
async fn a_running_device_is_cold_booted_on_and_back_off() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert!(!env.backend.launches().pop().unwrap().1.writable_system);
    let launches = env.backend.launches().len();

    let r = env
        .call(
            "root.writable_system",
            json!({ "device": "phone", "enabled": true }),
        )
        .await
        .unwrap();
    assert_eq!(r["applied"], "live");
    assert_eq!(r["system_writable"], true);
    assert_eq!(env.backend.launches().len(), launches + 1, "restarted once");
    let (_, opts) = env.backend.launches().pop().unwrap();
    assert!(opts.writable_system && opts.cold_boot, "{opts:?}");
    assert!(env.adb_file("system-rw").exists());

    // Revert: the overlay is dropped while adbd is root, then a cold boot on
    // the stock read-only system image.
    let r = env
        .call(
            "root.writable_system",
            json!({ "device": "phone", "enabled": false }),
        )
        .await
        .unwrap();
    assert_eq!(r["applied"], "live");
    assert_eq!(r["changed"], true);
    assert_eq!(r["system_writable"], false);
    assert_eq!(env.adb_count("verity-enabled"), 1, "adb enable-verity ran");
    assert!(!env.adb_file("overlay").exists());
    let (_, opts) = env.backend.launches().pop().unwrap();
    assert!(!opts.writable_system && opts.cold_boot, "{opts:?}");
    let d = env.device("phone").await;
    assert!(!d.root.writable_system);
    assert_eq!(d.root.mode, RootMode::Adb, "still shell root");
    assert_eq!(env.adbd_uid(), "0");
}

#[tokio::test]
async fn the_same_value_is_a_no_op_without_a_cold_boot() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    let r = env
        .call(
            "root.writable_system",
            json!({ "device": "phone", "enabled": false }),
        )
        .await
        .unwrap();
    assert_eq!(r["changed"], false);
    assert_eq!(r["applied"], "unchanged");
    assert!(!env.device("phone").await.launch.force_cold_boot);
}

#[tokio::test]
async fn only_adb_root_devices_qualify() {
    let env = Env::start().await;

    // No root.
    env.create("plain", "35:google_apis", Some("none")).await;
    let err = env
        .call(
            "root.writable_system",
            json!({ "device": "plain", "enabled": true }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("root set plain adb"), "{err}");
    assert!(!env.device("plain").await.root.writable_system);

    // Play Store images refuse adb root.
    env.create("ps", "35:google_apis_playstore", None).await;
    let err = env
        .call(
            "root.writable_system",
            json!({ "device": "ps", "enabled": true }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("google_apis_playstore"), "{err}");

    // API 30 and older: the remount leaves /system read-only and the guest
    // doesn't come back from the reboot that would fix it (CI, #85).
    let d = env.create("old", "35:google_apis", Some("adb")).await;
    env.ctx
        .daemon
        .update_device(d.id, Box::new(|d: &mut Device| d.image.api = 30))
        .await
        .unwrap();
    let err = env
        .call(
            "root.writable_system",
            json!({ "device": "old", "enabled": true }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("API 31 or later"), "{err}");
    assert!(!env.device("old").await.root.writable_system);

    // Full root (Magisk).
    let d = env.create("full", "35:google_apis", Some("adb")).await;
    env.ctx
        .daemon
        .update_device(
            d.id,
            Box::new(|d: &mut Device| d.root.mode = RootMode::Magisk),
        )
        .await
        .unwrap();
    let err = env
        .call(
            "root.writable_system",
            json!({ "device": "full", "enabled": true }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("Magisk"), "{err}");

    // Turning it off is always allowed.
    let r = env
        .call(
            "root.writable_system",
            json!({ "device": "plain", "enabled": false }),
        )
        .await
        .unwrap();
    assert_eq!(r["changed"], false);
}

#[tokio::test]
async fn the_flag_only_reaches_the_launch_in_adb_mode() {
    let env = Env::start().await;
    // A stale flag on a device that is no longer adb root (e.g. edited by
    // hand): it must not boot a writable system image.
    let d = env.create("plain", "35:google_apis", Some("none")).await;
    env.ctx
        .daemon
        .update_device(
            d.id,
            Box::new(|d: &mut Device| d.root.writable_system = true),
        )
        .await
        .unwrap();
    env.call("device.start", json!({ "device": "plain" }))
        .await
        .unwrap();
    assert!(!env.backend.launches().pop().unwrap().1.writable_system);
    assert_eq!(env.adb_count("reboots"), 0);
}

#[tokio::test]
async fn leaving_adb_root_drops_writable_system() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    env.call(
        "root.writable_system",
        json!({ "device": "phone", "enabled": true }),
    )
    .await
    .unwrap();

    env.call("root.set", json!({ "device": "phone", "mode": "none" }))
        .await
        .unwrap();
    assert_eq!(env.adb_count("verity-enabled"), 1, "overlay torn down");
    let d = env.device("phone").await;
    assert_eq!(d.root.mode, RootMode::None);
    assert!(!d.root.writable_system);
    assert!(d.launch.force_cold_boot);

    env.call("device.stop", json!({ "device": "phone" }))
        .await
        .unwrap();
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    let (_, opts) = env.backend.launches().pop().unwrap();
    assert!(!opts.writable_system && opts.cold_boot, "{opts:?}");
}

#[tokio::test]
async fn a_leftover_overlay_is_dropped_at_the_next_adb_boot() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    // Reverted while stopped, but the remount overlay is still mounted.
    std::fs::write(env.adb_file("overlay"), b"1").unwrap();
    std::fs::write(env.adb_file("verity-off"), b"1").unwrap();
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(env.adb_count("verity-enabled"), 1);
    assert_eq!(env.adb_count("reboots"), 1);
    assert!(!env.adb_file("overlay").exists());
    assert_eq!(env.adbd_uid(), "0", "rooted again after the reboot");

    // Without an overlay an adb boot doesn't reboot.
    env.call("device.stop", json!({ "device": "phone" }))
        .await
        .unwrap();
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(env.adb_count("reboots"), 1);
}
