//! Offline tests of the root provider (fake backend, fake adb).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;
use jidr_core::{ErrorCode, RootHealth, RootMode};
use serde_json::json;

#[tokio::test]
async fn adb_commands_are_registered_with_the_right_flags() {
    let env = Env::start().await;
    let set = env.daemon.registry().get("root.set").unwrap().1.clone();
    assert!(set.locks_device && set.long_running && !set.destructive);
    assert_eq!(set.params["required"], json!(["device", "mode"]));
    let schema = set.params.to_string();
    for mode in ["\"none\"", "\"adb\"", "\"magisk\""] {
        assert!(schema.contains(mode), "{schema}");
    }
    let verify = env.daemon.registry().get("root.verify").unwrap().1.clone();
    assert!(verify.locks_device && !verify.long_running);
    let status = env.daemon.registry().get("root.status").unwrap().1.clone();
    assert!(!status.locks_device);
}

#[tokio::test]
async fn adb_device_is_rooted_and_verified_after_every_boot() {
    let env = Env::start().await;
    // D7: google_apis defaults to adb mode.
    let d = env.create("phone", "35:google_apis", None).await;
    assert_eq!(d.root.mode, RootMode::Adb);
    assert_eq!(d.root.health, RootHealth::Unverified);

    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(env.adbd_uid(), "0");
    let d = env.device("phone").await;
    assert_eq!(d.root.health, RootHealth::Ok);
    assert!(d.root.verified_at.is_some());

    // A real reboot restarts adbd as shell; the hook roots it again.
    env.call("device.stop", json!({ "device": "phone" }))
        .await
        .unwrap();
    env.reset_adbd("2000");
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(env.adbd_uid(), "0");
}

#[tokio::test]
async fn none_and_adb_switch_live_on_a_running_device() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();

    let r = env
        .call("root.set", json!({ "device": "phone", "mode": "none" }))
        .await
        .unwrap();
    assert_eq!(r["applied"], "live");
    assert_eq!(r["changed"], true);
    assert_eq!(r["verification"]["uid"], 2000);
    assert_eq!(env.adbd_uid(), "2000");
    let d = env.device("phone").await;
    assert_eq!(d.root.mode, RootMode::None);
    assert_eq!(d.root.health, RootHealth::Ok);
    assert!(
        d.launch.force_cold_boot,
        "D29: root changes force a cold boot"
    );

    let r = env
        .call("root.set", json!({ "device": "phone", "mode": "adb" }))
        .await
        .unwrap();
    assert_eq!(r["verification"]["uid"], 0);
    assert_eq!(r["verification"]["command"], "id");
    let status = env
        .call("root.status", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(status["root"]["mode"], "adb");
    assert_eq!(status["root"]["health"], "ok");
    assert_eq!(status["adb_root_allowed"], true);
}

#[tokio::test]
async fn setting_a_stopped_device_applies_at_the_next_cold_boot() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("none")).await;
    let r = env
        .call("root.set", json!({ "device": "phone", "mode": "adb" }))
        .await
        .unwrap();
    assert_eq!(r["applied"], "next_boot");
    assert!(env.device("phone").await.launch.force_cold_boot);

    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    let (_, opts) = env.backend.launches().pop().unwrap();
    assert!(opts.cold_boot, "the first boot after a root change is cold");
    let d = env.device("phone").await;
    assert!(!d.launch.force_cold_boot, "cleared after the boot");
    assert_eq!(d.root.health, RootHealth::Ok);
    assert_eq!(env.adbd_uid(), "0");
}

#[tokio::test]
async fn same_mode_is_a_no_op_without_a_cold_boot() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("none")).await;
    let r = env
        .call("root.set", json!({ "device": "phone", "mode": "none" }))
        .await
        .unwrap();
    assert_eq!(r["changed"], false);
    assert!(!env.device("phone").await.launch.force_cold_boot);
}

#[tokio::test]
async fn play_store_images_refuse_adb_root() {
    let env = Env::start().await;
    let d = env.create("ps", "35:google_apis_playstore", None).await;
    assert_eq!(
        d.root.mode,
        RootMode::None,
        "D7: no adb default on Play Store"
    );
    let err = env
        .call("root.set", json!({ "device": "ps", "mode": "adb" }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("google_apis_playstore"), "{err}");
    let status = env
        .call("root.status", json!({ "device": "ps" }))
        .await
        .unwrap();
    assert_eq!(status["adb_root_allowed"], false);
}

#[tokio::test]
async fn verify_records_a_lost_root_as_failed() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    let ok = env
        .call("root.verify", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(ok["ok"], true);

    // adbd lost root behind our back.
    env.reset_adbd("2000");
    let err = env
        .call("root.verify", json!({ "device": "phone" }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::RootFailed);
    assert_eq!(err.data.as_ref().unwrap()["uid"], 2000);
    assert_eq!(env.device("phone").await.root.health, RootHealth::Failed);
}

#[tokio::test]
async fn verify_needs_a_running_device_and_known_params() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    let err = env
        .call("root.verify", json!({ "device": "phone" }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("not running"), "{err}");

    let err = env
        .call("root.set", json!({ "device": "phone", "mood": "adb" }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("mood"), "{err}");

    let err = env
        .call("root.status", json!({ "device": "nope" }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}
