//! Phase 9d acceptance on a real emulator: none → magisk (full setup) → none,
//! with `su -c id` = uid 0 in magisk mode and the shared SDK ramdisk hash
//! unchanged at the end.
//!
//! The image comes from `JIDR_TEST_IMAGE` (default: API 31 `google_apis` for
//! the host ABI) in `JIDR_TEST_SDK` (default: Android Studio's SDK, opened
//! read-only). Magisk is the pinned release, downloaded at runtime (or a local
//! APK via `JIDR_TEST_MAGISK`). The AVD lives in a temp Jidr home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use jidr_adb::Adb;
use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{JidrPaths, RootMode};
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
async fn magisk_root_cycle_none_magisk_none() {
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
    kill_orphans("root-magisk");
    let name = unique_avd("root-magisk");

    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": name, "image": image, "profile": "small-phone-5", "root_mode": "none" }),
    )
    .await;
    let device = state.device(&name).await.unwrap();
    let _guard = AvdGuard::new(&device.avd);
    // The pristine ramdisk Jidr must never touch (D3).
    let pristine = device
        .sdk_root
        .join(device.image.sysdir())
        .join("ramdisk.img");
    let before = jidr_sdk::cache::sha256_file(&pristine).unwrap();

    // An optional local APK avoids the runtime download (offline tests).
    let mut params = json!({ "device": name, "mode": "magisk" });
    if let Some(apk) = std::env::var_os("JIDR_TEST_MAGISK") {
        let entry = call(
            &daemon,
            &ctx,
            "cache.import",
            json!({ "path": apk.to_string_lossy() }),
        )
        .await;
        params["magisk_sha256"] = entry["sha256"].clone();
    }

    // 1. none → magisk: the full setup (patch, cold boot, app, env, grant).
    let r = call(&daemon, &ctx, "root.set", params).await;
    assert_eq!(r["mode"], "magisk", "{r}");
    assert_eq!(r["verification"]["uid"], 0, "{r}");
    assert_eq!(r["verification"]["command"], "su -c id");

    let d = state.device(&name).await.unwrap();
    assert_eq!(d.root.mode, RootMode::Magisk);
    assert!(d.root.magisk_version.is_some());

    // 2. The shared SDK ramdisk is byte-identical.
    assert_eq!(jidr_sdk::cache::sha256_file(&pristine).unwrap(), before);

    // 2b. auto-grant all (D22/G15): on → Magisk's setting reads "grant", off → "prompt".
    let info = state.connect_info(d.id).await.unwrap();
    let adb = Adb::new(info.adb_path).device(info.adb_serial);
    let prefs =
        "/data/user_de/0/com.topjohnwu.magisk/shared_prefs/com.topjohnwu.magisk_preferences.xml";

    let r = call(
        &daemon,
        &ctx,
        "root.auto_grant",
        json!({ "device": name, "enabled": true }),
    )
    .await;
    assert_eq!(r["applied"], "live", "{r}");
    let out = adb.shell(&format!("su -c 'cat {prefs}'")).await.unwrap();
    assert!(
        out.contains("su_auto_response\">2</string>"),
        "Magisk auto-grants every app: {out}"
    );

    let r = call(
        &daemon,
        &ctx,
        "root.auto_grant",
        json!({ "device": name, "enabled": false }),
    )
    .await;
    assert_eq!(r["applied"], "live", "{r}");
    let out = adb.shell(&format!("su -c 'cat {prefs}'")).await.unwrap();
    assert!(
        out.contains("su_auto_response\">0</string>"),
        "Magisk is back to the prompt: {out}"
    );

    // 3. magisk → none: revert deletes the patched copy and cold-boots.
    let r = call(
        &daemon,
        &ctx,
        "root.set",
        json!({ "device": name, "mode": "none" }),
    )
    .await;
    assert_eq!(r["applied"], "live", "{r}");
    assert_ne!(r["verification"]["uid"], 0, "{r}");
    let d = state.device(&name).await.unwrap();
    assert_eq!(d.root.mode, RootMode::None);
    assert!(d.root.patched_ramdisk_path.is_none());

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}
