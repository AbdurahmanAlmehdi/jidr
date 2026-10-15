//! Offline tests of `root.auto_grant` (G15): the D22 "auto-grant all" toggle.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;
use jidr_core::{ErrorCode, RootMode};
use serde_json::json;

async fn to_magisk(env: &Env, name: &str) {
    let entry = env
        .call("cache.import", json!({ "path": env.apk }))
        .await
        .unwrap();
    let sha = entry["sha256"].as_str().unwrap().to_string();
    env.call(
        "root.set",
        json!({ "device": name, "mode": "magisk", "magisk_sha256": sha }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn auto_grant_command_is_registered_locked_not_destructive() {
    let env = Env::start().await;
    let (_, spec) = env.daemon.registry().get("root.auto_grant").unwrap();
    assert!(spec.locks_device && !spec.destructive && !spec.long_running);
    assert_eq!(spec.params["required"], json!(["device", "enabled"]));
    let schema = spec.params.to_string();
    assert!(schema.contains("\"enabled\""), "{schema}");
}

#[tokio::test]
async fn auto_grant_is_refused_on_non_magisk_devices() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    let err = env
        .call(
            "root.auto_grant",
            json!({ "device": "phone", "enabled": true }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("Magisk"), "{err}");
    assert!(!env.device("phone").await.root.auto_grant_all);
}

#[tokio::test]
async fn auto_grant_applies_live_on_a_running_magisk_device() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;

    let r = env
        .call(
            "root.auto_grant",
            json!({ "device": "phone", "enabled": true }),
        )
        .await
        .unwrap();
    assert_eq!(r["applied"], "live");
    assert_eq!(r["auto_grant_all"], true);
    assert_eq!(env.auto_response(), "2", "Magisk auto-grants every app");
    assert!(env.device("phone").await.root.auto_grant_all);

    let r = env
        .call(
            "root.auto_grant",
            json!({ "device": "phone", "enabled": false }),
        )
        .await
        .unwrap();
    assert_eq!(r["applied"], "live");
    assert_eq!(env.auto_response(), "0", "back to the prompt");
    assert!(!env.device("phone").await.root.auto_grant_all);
}

#[tokio::test]
async fn auto_grant_on_a_stopped_device_applies_at_next_boot() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;
    env.call("device.stop", json!({ "device": "phone" }))
        .await
        .unwrap();

    let r = env
        .call(
            "root.auto_grant",
            json!({ "device": "phone", "enabled": true }),
        )
        .await
        .unwrap();
    assert_eq!(r["applied"], "next_boot");
    assert!(env.device("phone").await.root.auto_grant_all);
    assert_eq!(env.auto_response(), "", "not applied while stopped");
}

#[tokio::test]
async fn leaving_magisk_resets_auto_grant() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;
    env.call(
        "root.auto_grant",
        json!({ "device": "phone", "enabled": true }),
    )
    .await
    .unwrap();
    assert!(env.device("phone").await.root.auto_grant_all);

    env.call("root.set", json!({ "device": "phone", "mode": "none" }))
        .await
        .unwrap();
    let d = env.device("phone").await;
    assert_eq!(d.root.mode, RootMode::None);
    assert!(!d.root.auto_grant_all, "D22: reset when leaving Full root");
}

#[tokio::test]
async fn auto_grant_only_writes_when_the_value_changes() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;

    // Off is the default and no key is present → nothing to write.
    env.call(
        "root.auto_grant",
        json!({ "device": "phone", "enabled": false }),
    )
    .await
    .unwrap();
    assert_eq!(env.auto_response(), "");
    assert_eq!(env.force_stop_count(), 0);

    // On → writes the value and restarts the app so it re-reads.
    env.call(
        "root.auto_grant",
        json!({ "device": "phone", "enabled": true }),
    )
    .await
    .unwrap();
    assert_eq!(env.auto_response(), "2");
    assert_eq!(env.force_stop_count(), 1);

    // On again → already "2", no write and no restart.
    env.call(
        "root.auto_grant",
        json!({ "device": "phone", "enabled": true }),
    )
    .await
    .unwrap();
    assert_eq!(env.force_stop_count(), 1);

    // Off → writes "0" and restarts.
    env.call(
        "root.auto_grant",
        json!({ "device": "phone", "enabled": false }),
    )
    .await
    .unwrap();
    assert_eq!(env.auto_response(), "0");
    assert_eq!(env.force_stop_count(), 2);
}
