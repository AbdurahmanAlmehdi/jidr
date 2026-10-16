//! Phase 9f acceptance on a real emulator: a Full-root device reports the
//! build props it was given after a cold boot, still has root, and reports
//! its stock values again once root is reverted.
//!
//! Same setup as `magisk_emulator` (`JIDR_TEST_IMAGE`, `JIDR_TEST_SDK`,
//! optional `JIDR_TEST_MAGISK`).
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use jidr_adb::Adb;
use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::JidrPaths;
use jidr_core::provider::{CallCtx, DaemonContext};
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

/// `getprop name`, polled: right after a boot the shell may not answer yet.
async fn getprop(adb: &Adb, name: &str) -> String {
    let mut last = String::new();
    for _ in 0..60 {
        match adb.shell(&format!("getprop {name}")).await {
            Ok(v) if !v.trim().is_empty() => return v.trim().to_string(),
            Ok(v) => last = v,
            Err(e) => last = e.message,
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("getprop {name} never answered: {last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn props_spoof_after_cold_boot_and_stock_after_revert() {
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
    kill_orphans("root-props");
    let name = unique_avd("root-props");

    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": name, "image": image, "profile": "small-phone-5", "root_mode": "none" }),
    )
    .await;
    let device = state.device(&name).await.unwrap();
    let _guard = AvdGuard::new(&device.avd);

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
    let r = call(&daemon, &ctx, "root.set", params).await;
    assert_eq!(r["verification"]["uid"], 0, "{r}");

    let adb = || async {
        let d = state.device(&name).await.unwrap();
        let info = state.connect_info(d.id).await.unwrap();
        Adb::new(info.adb_path).device(info.adb_serial)
    };
    let stock_model = getprop(&adb().await, "ro.product.model").await;
    let stock_patch = getprop(&adb().await, "ro.build.version.security_patch").await;
    println!("stock: {stock_model} / {stock_patch}");
    assert_ne!(stock_model, "Jidr One");

    // 1. Set the props: stored, re-patched, applied on the next start.
    let r = call(
        &daemon,
        &ctx,
        "device.props.set",
        json!({ "device": name, "props": {
            "model": "Jidr One", "brand": "jidr", "manufacturer": "Jidr",
            "security_patch": "2026-10-05",
        }}),
    )
    .await;
    assert_eq!(r["restart_required"], true, "{r}");

    // 2. A cold boot applies them; root still works.
    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.start", json!({ "device": name })).await;
    let a = adb().await;
    assert_eq!(getprop(&a, "ro.product.model").await, "Jidr One");
    assert_eq!(getprop(&a, "ro.product.brand").await, "jidr");
    assert_eq!(getprop(&a, "ro.product.manufacturer").await, "Jidr");
    assert_eq!(
        getprop(&a, "ro.build.version.security_patch").await,
        "2026-10-05"
    );
    let r = call(&daemon, &ctx, "root.verify", json!({ "device": name })).await;
    assert_eq!(r["uid"], 0, "root survives the props hook: {r}");
    let r = call(&daemon, &ctx, "device.props.get", json!({ "device": name })).await;
    assert_eq!(r["restart_required"], false, "{r}");

    // 3. Revert root: the stock ramdisk boots and the stock values return.
    call(
        &daemon,
        &ctx,
        "root.set",
        json!({ "device": name, "mode": "none" }),
    )
    .await;
    let a = adb().await;
    assert_eq!(getprop(&a, "ro.product.model").await, stock_model);
    assert_eq!(
        getprop(&a, "ro.build.version.security_patch").await,
        stock_patch
    );

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}
