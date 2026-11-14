//! #72 / ADR-0004 on a real emulator: a Magisk module flashed through
//! `file.flash` (the command a dropped zip and `jidr flash` use) is listed by
//! Magisk after the reboot and its effects are live; marked for removal, it
//! is gone after the next cold boot together with its effects.
//!
//! The module is Jidr's own (`tests/fixtures/jidrtest-module`, Apache-2.0):
//! a `service.sh` that writes a marker, a `system.prop` with one property and
//! an `uninstall.sh` that deletes the marker.
//!
//! Same setup as `magisk_emulator` (`JIDR_TEST_IMAGE`, `JIDR_TEST_SDK`,
//! optional `JIDR_TEST_MAGISK`); everything lives in a temp Jidr home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jidr_adb::Adb;
use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::JidrPaths;
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

const MODULE: &str = "jidrtest_module";
const MARKER: &str = "/data/local/tmp/jidrtest_module.marker";
const PROP: &str = "debug.jidrtest.module";

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

/// `su -c <cmd>`'s output, trimmed ("" when it fails).
async fn su(adb: &Adb, cmd: &str) -> String {
    adb.shell_status(&format!("su -c '{cmd}'"))
        .await
        .map(|(_, out)| out.trim().to_string())
        .unwrap_or_default()
}

/// Polls `su -c <cmd>` until it prints `want`: `service.sh` runs in
/// late_start, in parallel with the end of boot.
async fn wait_for(adb: &Adb, cmd: &str, want: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let out = su(adb, cmd).await;
        if out == want || Instant::now() > deadline {
            return out;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn flashed_module_loads_then_is_removed() {
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
    kill_orphans("root-module");
    let name = unique_avd("root-module");

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

    // 1. Flash: install, cold boot, Magisk loaded it, root still verifies.
    let zip = common::test_module_zip(tmp.path());
    let r = call(
        &daemon,
        &ctx,
        "file.flash",
        json!({ "device": name, "path": zip }),
    )
    .await;
    println!("file.flash: {r}");
    assert_eq!(r["kind"], "magisk_module", "{r}");
    assert_eq!(r["action"], "module_installed", "{r}");
    assert_eq!(r["module"]["id"], MODULE, "{r}");
    assert_eq!(r["verification"]["uid"], 0, "{r}");
    // ADR-0004: Magisk turns Zygisk on by default on an emulator.
    assert_eq!(r["zygisk_enabled"], true, "{r}");

    let d = state.device(&name).await.unwrap();
    let info = state.connect_info(d.id).await.unwrap();
    let adb = Adb::new(info.adb_path)
        .device(info.adb_serial)
        .with_timeout(Duration::from_secs(30));

    // 2. Magisk lists it, and its effects are live.
    assert_eq!(
        su(
            &adb,
            &format!("cat /data/adb/modules/{MODULE}/module.prop | grep ^id=")
        )
        .await,
        format!("id={MODULE}")
    );
    assert_eq!(
        wait_for(&adb, &format!("cat {MARKER}"), "loaded").await,
        "loaded"
    );
    assert_eq!(wait_for(&adb, &format!("getprop {PROP}"), "1").await, "1");
    assert_eq!(
        su(&adb, "ls /data/local/tmp/jidr-module.zip 2>/dev/null").await,
        "",
        "the pushed zip was cleaned up"
    );
    println!(
        "native bridge (Zygisk loader): {:?}",
        su(&adb, "getprop ro.dalvik.vm.native.bridge").await
    );

    // 3. Mark it for removal (what the Magisk app's Remove does), cold boot.
    su(&adb, &format!("touch /data/adb/modules/{MODULE}/remove")).await;
    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(
        &daemon,
        &ctx,
        "device.start",
        json!({ "device": name, "cold_boot": true }),
    )
    .await;
    let d = state.device(&name).await.unwrap();
    let info = state.connect_info(d.id).await.unwrap();
    let adb = Adb::new(info.adb_path)
        .device(info.adb_serial)
        .with_timeout(Duration::from_secs(30));
    wait_for_su(&adb).await;

    // 4. Gone: the folder, the property and (through uninstall.sh) the marker.
    assert_eq!(
        su(
            &adb,
            &format!("ls -d /data/adb/modules/{MODULE} 2>/dev/null")
        )
        .await,
        "",
        "Magisk removed the module"
    );
    assert_eq!(su(&adb, &format!("getprop {PROP}")).await, "");
    assert_eq!(su(&adb, &format!("ls {MARKER} 2>/dev/null")).await, "");
    let v = call(&daemon, &ctx, "root.verify", json!({ "device": name })).await;
    assert_eq!(v["uid"], 0, "{v}");

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}

/// Waits until `su` answers after the cold boot (Magisk's daemon starts a
/// little after `sys.boot_completed`).
async fn wait_for_su(adb: &Adb) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while su(adb, "id -u").await != "0" {
        assert!(
            Instant::now() < deadline,
            "su never answered after the cold boot"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
