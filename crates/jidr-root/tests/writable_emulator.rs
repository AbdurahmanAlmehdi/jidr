//! #25 acceptance on a real emulator: an adb-root device gets a writable
//! `/system` (`-writable-system` + `adb remount`, with the reboot the first
//! remount asks for), keeps it across a stop/start, and is reverted to the
//! read-only stock system image. On images older than API 31 it checks that
//! the command is refused.
//!
//! The image comes from `JIDR_TEST_IMAGE` (default: API 31 `google_apis` for
//! the host ABI) in `JIDR_TEST_SDK` (default: Android Studio's SDK, opened
//! read-only). The AVD lives in a temp Jidr home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jidr_adb::Adb;
use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{ErrorCode, JidrPaths, RootMode};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

const MARKER: &str = "/system/etc/jidr-writable-e2e";

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

async fn adb(ctx: &CallCtx, name: &str) -> Adb {
    let d = ctx.daemon.device(name).await.unwrap();
    let info = ctx.daemon.connect_info(d.id).await.unwrap();
    Adb::new(info.adb_path)
        .device(info.adb_serial)
        .with_timeout(Duration::from_secs(30))
}

async fn marker_present(adb: &Adb) -> bool {
    let (_, out) = adb
        .shell_status(&format!("[ -f {MARKER} ] && echo yes || echo no"))
        .await
        .unwrap();
    out.trim() == "yes"
}

#[tokio::test(flavor = "multi_thread")]
async fn writable_system_on_survives_a_restart_then_reverts() {
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
    kill_orphans("rw-e2e");
    let name = unique_avd("rw-e2e");

    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": name, "image": image, "profile": "small-phone-5", "root_mode": "adb" }),
    )
    .await;
    let created = state.device(&name).await.unwrap();
    let _guard = AvdGuard::new(&created.avd);

    // Images older than API 31 are refused before anything boots.
    if created.image.api < 31 {
        let (p, _) = daemon.registry().get("root.writable_system").unwrap();
        let err = p
            .call(
                &ctx,
                "root.writable_system",
                json!({ "device": name, "enabled": true }),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Usage, "{err}");
        assert!(err.message.contains("API 31 or later"), "{err}");
        assert!(!state.device(&name).await.unwrap().root.writable_system);
        call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
        return;
    }

    // 1. Stock: /system is read-only even as root.
    call(&daemon, &ctx, "device.start", json!({ "device": name })).await;
    let a = adb(&ctx, &name).await;
    let (_, out) = a
        .shell_status(&format!("touch {MARKER} && echo wrote"))
        .await
        .unwrap();
    assert_ne!(out.trim(), "wrote", "stock /system must be read-only");

    // 2. On, live: restart with -writable-system, remount (with its reboot).
    let t = Instant::now();
    let r = call(
        &daemon,
        &ctx,
        "root.writable_system",
        json!({ "device": name, "enabled": true }),
    )
    .await;
    println!("writable /system on: {:.1} s", t.elapsed().as_secs_f64());
    assert_eq!(r["applied"], "live", "{r}");
    assert_eq!(r["system_writable"], true, "{r}");
    let a = adb(&ctx, &name).await;
    let (_, out) = a
        .shell_status(&format!("echo jidr > {MARKER} && echo wrote"))
        .await
        .unwrap();
    assert_eq!(out.trim(), "wrote", "{out}");

    // 3. Stop and start: still writable, the file is still there.
    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    let t = Instant::now();
    call(&daemon, &ctx, "device.start", json!({ "device": name })).await;
    println!(
        "restart with writable /system: {:.1} s",
        t.elapsed().as_secs_f64()
    );
    let a = adb(&ctx, &name).await;
    assert!(
        marker_present(&a).await,
        "the pushed file survives a restart"
    );
    let (_, out) = a
        .shell_status("touch /system/.jidr-rw && rm /system/.jidr-rw && echo writable")
        .await
        .unwrap();
    assert_eq!(out.trim(), "writable", "remounted after the restart: {out}");

    // 4. Off, live: back to the stock read-only system image.
    let r = call(
        &daemon,
        &ctx,
        "root.writable_system",
        json!({ "device": name, "enabled": false }),
    )
    .await;
    assert_eq!(r["applied"], "live", "{r}");
    assert_eq!(r["system_writable"], false, "{r}");
    let a = adb(&ctx, &name).await;
    assert!(!marker_present(&a).await, "revert drops the changes");
    let d = state.device(&name).await.unwrap();
    assert_eq!(d.root.mode, RootMode::Adb);
    assert!(!d.root.writable_system);
    let v = call(&daemon, &ctx, "root.verify", json!({ "device": name })).await;
    assert_eq!(v["uid"], 0, "still shell root: {v}");

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}
