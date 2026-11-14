//! Identifiers (#19) on a real emulator, Shell root (`adb`): `identity.set`
//! gives an installed app the new Android ID (what the app itself reads,
//! through the probe app from jidr-widgets' `build-hello-apk.sh idprobe`),
//! sets the global value, and both survive a cold boot. `identity.get`
//! reads the IMEI without root.
//!
//! Build the helper for the image's CPU first: `cargo xtask helper` (CI runs
//! `--arch x86_64`). The image comes from `JIDR_TEST_IMAGE` (default: API 31
//! `google_apis` for the host ABI) in `JIDR_TEST_SDK` (default: Android
//! Studio's SDK, opened read-only). The AVD lives in a temp Jidr home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{ConnectInfo, JidrPaths};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

const PROBE: &str = "ly.jidr.idprobe";

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

fn adb(info: &ConnectInfo, args: &[&str]) -> String {
    let out = Command::new(&info.adb_path)
        .arg("-s")
        .arg(&info.adb_serial)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Builds the ID probe app with the SDK's build tools.
fn build_probe(dir: &Path) -> PathBuf {
    let out = dir.join("idprobe.apk");
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../jidr-widgets/tests/fixtures/build-hello-apk.sh");
    let st = Command::new("bash")
        .arg(script)
        .arg(&out)
        .arg("idprobe")
        .status()
        .unwrap();
    assert!(st.success(), "building the probe app failed");
    out
}

/// Starts the probe and returns the Android ID it logs.
async fn probe_reads(info: &ConnectInfo) -> String {
    adb(info, &["shell", "am force-stop ly.jidr.idprobe; logcat -c"]);
    adb(
        info,
        &["shell", &format!("am start -W -n {PROBE}/.MainActivity")],
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let log = adb(info, &["logcat", "-d", "-s", "JidrIdProbe"]);
        if let Some(id) = log
            .lines()
            .filter_map(|l| l.split_once("JidrIdProbe: ").map(|(_, v)| v.trim()))
            .next_back()
        {
            return id.to_string();
        }
        assert!(Instant::now() < deadline, "the probe logged nothing");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn android_id_reaches_apps_and_survives_a_cold_boot() {
    let tmp = tempfile::tempdir().unwrap();
    let apk = build_probe(tmp.path());
    let daemon = Daemon::builder(
        DaemonConfig {
            paths: JidrPaths::under(tmp.path().join("j")),
            studio_sdk: Some(test_sdk()),
            idle_timeout: None,
            sdk_repo: None,
        },
        Arc::new(AospBackend::new()),
    )
    .provider(jidr_root::provider())
    .provider(jidr_widgets::provider())
    .build()
    .unwrap();
    let ctx = CallCtx::new(daemon.state().clone());
    let state = daemon.state().clone();
    kill_orphans("identity-e2e");
    let name = unique_avd("identity-e2e");
    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": name, "image": test_image(), "profile": "small-phone-5", "root_mode": "adb" }),
    )
    .await;
    let _guard = AvdGuard::new(&state.device(&name).await.unwrap().avd);
    call(&daemon, &ctx, "device.start", json!({ "device": name })).await;
    let id_of = || async { state.device(&name).await.unwrap().id };
    let info = state.connect_info(id_of().await).await.unwrap();

    let before = call(&daemon, &ctx, "identity.get", json!({ "device": name })).await;
    println!("before: {before}");
    assert_eq!(
        before["android_id"].as_str().map(str::len),
        Some(16),
        "{before}"
    );

    let out = adb(&info, &["install", "-r", &apk.to_string_lossy()]);
    assert!(out.contains("Success"), "{out}");
    let derived = probe_reads(&info).await;
    println!("the probe's own ID: {derived}");
    assert_ne!(derived, "0123456789abcdef");

    let t = Instant::now();
    let v = call(
        &daemon,
        &ctx,
        "identity.set",
        json!({ "device": name, "android_id": "0123456789ABCDEF" }),
    )
    .await;
    println!("identity.set: {:.1} s: {v}", t.elapsed().as_secs_f64());
    assert_eq!(v["android_id"], "0123456789abcdef", "{v}");
    assert_eq!(
        probe_reads(&info).await,
        "0123456789abcdef",
        "the app's view"
    );

    // Cold boot: both stay (they live in /data).
    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(
        &daemon,
        &ctx,
        "device.start",
        json!({ "device": name, "cold_boot": true }),
    )
    .await;
    let info = state.connect_info(id_of().await).await.unwrap();
    let after = call(&daemon, &ctx, "identity.get", json!({ "device": name })).await;
    assert_eq!(after["android_id"], "0123456789abcdef", "{after}");
    assert_eq!(
        probe_reads(&info).await,
        "0123456789abcdef",
        "after a cold boot"
    );

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}
