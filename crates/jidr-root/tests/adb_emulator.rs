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
use std::time::{Duration, Instant};

use jidr_backend::AospBackend;
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

/// Kills every emulator still running `avd`: SIGTERM, then SIGKILL after 5 s.
fn kill_emulators(avd: &str) {
    let pattern = format!("qemu-system.* -avd {avd}( |$)");
    let alive = || {
        std::process::Command::new("pgrep")
            .args(["-f", &pattern])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !alive() {
        return;
    }
    let _ = std::process::Command::new("pkill")
        .args(["-TERM", "-f", &pattern])
        .status();
    for _ in 0..20 {
        if !alive() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = std::process::Command::new("pkill")
        .args(["-KILL", "-f", &pattern])
        .status();
}

/// Shuts the test's emulator down however the test ends (pass, failed
/// assertion or panic), so a red run never leaves an emulator behind.
struct EmulatorGuard(String);

impl Drop for EmulatorGuard {
    fn drop(&mut self) {
        kill_emulators(&self.0);
    }
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

    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": "root-e2e", "image": image, "profile": "small-phone-5", "root_mode": "none" }),
    )
    .await;
    let avd = state.device("root-e2e").await.unwrap().avd;
    // Backstop: a previous run that nextest killed outright never dropped its guard.
    kill_emulators(&avd);
    let _guard = EmulatorGuard(avd);

    // 1. none: the shell is not root.
    let t = Instant::now();
    call(
        &daemon,
        &ctx,
        "device.start",
        json!({ "device": "root-e2e" }),
    )
    .await;
    println!("cold boot: {:.1} s", t.elapsed().as_secs_f64());
    // Checks the guard: a red run must still shut the emulator down.
    if std::env::var_os("JIDR_ROOT_E2E_FAIL_AFTER_BOOT").is_some() {
        panic!("JIDR_ROOT_E2E_FAIL_AFTER_BOOT is set: failing on purpose");
    }
    let v = call(
        &daemon,
        &ctx,
        "root.verify",
        json!({ "device": "root-e2e" }),
    )
    .await;
    assert_eq!(v["mode"], "none");
    assert_ne!(v["uid"], 0, "{v}");

    // 2. none → adb, live.
    let t = Instant::now();
    let r = call(
        &daemon,
        &ctx,
        "root.set",
        json!({ "device": "root-e2e", "mode": "adb" }),
    )
    .await;
    println!("live adb root: {:.1} s", t.elapsed().as_secs_f64());
    assert_eq!(r["applied"], "live");
    assert_eq!(r["verification"]["uid"], 0, "{r}");

    // 3. Stop and start: cold boot (D29), the boot hook roots adbd again.
    call(
        &daemon,
        &ctx,
        "device.stop",
        json!({ "device": "root-e2e" }),
    )
    .await;
    let t = Instant::now();
    call(
        &daemon,
        &ctx,
        "device.start",
        json!({ "device": "root-e2e" }),
    )
    .await;
    println!("cold boot + hook: {:.1} s", t.elapsed().as_secs_f64());
    let d = state.device("root-e2e").await.unwrap();
    assert_eq!(d.root.mode, RootMode::Adb);
    assert_eq!(d.root.health, RootHealth::Ok, "boot hook verified root");
    assert!(!d.launch.force_cold_boot);
    let v = call(
        &daemon,
        &ctx,
        "root.verify",
        json!({ "device": "root-e2e" }),
    )
    .await;
    assert_eq!(v["uid"], 0);

    // 4. adb → none, live.
    let r = call(
        &daemon,
        &ctx,
        "root.set",
        json!({ "device": "root-e2e", "mode": "none" }),
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
            json!({ "device": "root-e2e", "mode": "magisk" }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);

    call(
        &daemon,
        &ctx,
        "device.stop",
        json!({ "device": "root-e2e" }),
    )
    .await;
    call(
        &daemon,
        &ctx,
        "device.delete",
        json!({ "device": "root-e2e" }),
    )
    .await;
}
