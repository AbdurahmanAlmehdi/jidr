//! Phase 9c acceptance on a real emulator: none → adb (live) → cold boot
//! (the boot hook re-applies root) → none, verified at every step.
//!
//! The image comes from `JIDR_TEST_IMAGE` (default: API 31 `google_apis` for
//! the host ABI) in `JIDR_TEST_SDK` (default: Android Studio's SDK, opened
//! read-only). The AVD lives in a temp Jidr home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{ErrorCode, JidrPaths, RootHealth, RootMode};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

fn test_sdk() -> PathBuf {
    std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Android/sdk")))
        .unwrap()
}

fn test_image() -> String {
    std::env::var("JIDR_TEST_IMAGE").unwrap_or_else(|_| {
        let abi = if cfg!(target_arch = "aarch64") {
            "arm64-v8a"
        } else {
            "x86_64"
        };
        format!("system-images;android-31;google_apis;{abi}")
    })
}

async fn call(daemon: &Arc<Daemon>, ctx: &CallCtx, method: &str, params: Value) -> Value {
    let (p, _) = daemon.registry().get(method).unwrap();
    p.call(ctx, method, params)
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn adb_root_cycle_none_adb_none() {
    let tmp = tempfile::tempdir().unwrap();
    let daemon = Daemon::builder(
        DaemonConfig {
            paths: JidrPaths::under(tmp.path()),
            studio_sdk: Some(test_sdk()),
            idle_timeout: None,
            sdk_repo: None,
        },
        Arc::new(AospBackend::new()),
    )
    .provider(jidr_root::provider())
    .build()
    .unwrap();
    let ctx = CallCtx::new(daemon.state().clone());
    let state = daemon.state().clone();
    let image = test_image();
    println!("image: {image}");
    // Leftovers of earlier killed runs, then a name unique to this process.
    kill_orphans("root-e2e");
    let name = unique_avd("root-e2e");

    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": name, "image": image, "profile": "small-phone-5", "root_mode": "none" }),
    )
    .await;
    // Kills the emulator on drop, and via a reaper process if this test is killed.
    let _guard = AvdGuard::new(&state.device(&name).await.unwrap().avd);

    // 1. none: the shell is not root.
    let t = Instant::now();
    call(&daemon, &ctx, "device.start", json!({ "device": name })).await;
    println!("cold boot: {:.1} s", t.elapsed().as_secs_f64());
    // Checks the guard: a red run must still shut the emulator down.
    if std::env::var_os("JIDR_ROOT_E2E_FAIL_AFTER_BOOT").is_some() {
        panic!("JIDR_ROOT_E2E_FAIL_AFTER_BOOT is set: failing on purpose");
    }
    let v = call(&daemon, &ctx, "root.verify", json!({ "device": name })).await;
    assert_eq!(v["mode"], "none");
    assert_ne!(v["uid"], 0, "{v}");

    // 2. none → adb, live.
    let t = Instant::now();
    let r = call(
        &daemon,
        &ctx,
        "root.set",
        json!({ "device": name, "mode": "adb" }),
    )
    .await;
    println!("live adb root: {:.1} s", t.elapsed().as_secs_f64());
    assert_eq!(r["applied"], "live");
    assert_eq!(r["verification"]["uid"], 0, "{r}");

    // 3. Stop and start: cold boot (D29), the boot hook roots adbd again.
    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    let t = Instant::now();
    call(&daemon, &ctx, "device.start", json!({ "device": name })).await;
    println!("cold boot + hook: {:.1} s", t.elapsed().as_secs_f64());
    let d = state.device(&name).await.unwrap();
    assert_eq!(d.root.mode, RootMode::Adb);
    assert_eq!(d.root.health, RootHealth::Ok, "boot hook verified root");
    assert!(!d.launch.force_cold_boot);
    let v = call(&daemon, &ctx, "root.verify", json!({ "device": name })).await;
    assert_eq!(v["uid"], 0);

    // 4. adb → none, live.
    let r = call(
        &daemon,
        &ctx,
        "root.set",
        json!({ "device": name, "mode": "none" }),
    )
    .await;
    assert_eq!(r["applied"], "live");
    assert_ne!(r["verification"]["uid"], 0, "{r}");

    // 5. Magisk isn't available yet (9d).
    let (p, _) = daemon.registry().get("root.set").unwrap();
    let err = p
        .call(
            &ctx,
            "root.set",
            json!({ "device": name, "mode": "magisk" }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}
