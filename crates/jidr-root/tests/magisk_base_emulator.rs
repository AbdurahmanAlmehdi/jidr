//! Phase 13f acceptance on a real emulator (D42): three pool clones of a
//! Magisk base each answer `su -c id` with uid 0 and `su -c 'magisk -v'`
//! after resuming the base snapshot, without a cold boot, and the SDK's
//! pristine `ramdisk.img` hash is unchanged.
//!
//! The image and SDK come from `JIDR_TEST_IMAGE` / `JIDR_TEST_SDK` as in
//! `magisk_emulator.rs`; Magisk is the pinned release (or `JIDR_TEST_MAGISK`).
//! Everything lives in a temp Jidr home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use jidr_adb::Adb;
use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{JidrPaths, RootMode};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

/// Lives in tmpfs: only a RAM-snapshot resume keeps it, a cold boot can't.
const MARKER: &str = "/dev/.jidr-base-marker";

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
async fn magisk_base_three_clones_resume_rooted() {
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
    kill_orphans("root-mbase");
    let src = unique_avd("root-mbase");
    let base = format!("{src}-base");
    // This test is about Magisk clones; the budget has its own tests.
    call(
        &daemon,
        &ctx,
        "settings.set",
        json!({ "budget_percent": 100 }),
    )
    .await;

    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": src, "image": image, "profile": "agent-lean", "root_mode": "none" }),
    )
    .await;
    let device = state.device(&src).await.unwrap();
    let _src_guard = AvdGuard::new(&device.avd);
    let pristine = device
        .sdk_root
        .join(device.image.sysdir())
        .join("ramdisk.img");
    let before = jidr_sdk::cache::sha256_file(&pristine).unwrap();

    let mut params = json!({ "device": src, "mode": "magisk" });
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
    let t = Instant::now();
    let r = call(&daemon, &ctx, "root.set", params).await;
    println!("root.set magisk: {:.0} s", t.elapsed().as_secs_f32());
    assert_eq!(r["verification"]["uid"], 0, "{r}");
    let version = r["magisk_version"].as_str().unwrap().to_string();

    let info = state.connect_info(device.id).await.unwrap();
    let adb = Adb::new(info.adb_path).device(info.adb_serial);
    adb.shell(&format!("su -c 'touch {MARKER}'")).await.unwrap();

    let t = Instant::now();
    let b = call(
        &daemon,
        &ctx,
        "base.create",
        json!({ "from_device": src, "name": base }),
    )
    .await;
    println!("base.create: {:.1} s", t.elapsed().as_secs_f32());
    let _base_guard = AvdGuard::new(b["avd"].as_str().unwrap());
    assert_eq!(b["root"]["mode"], "magisk", "{b}");

    // One clone at a time: on a shared Mac only about one emulator fits.
    // Holding three at once is the pool test's job (pool_emulator.rs).
    let mut ramdisks = std::collections::HashSet::new();
    for i in 1..=3 {
        let t = Instant::now();
        let r = call(
            &daemon,
            &ctx,
            "pool.get",
            json!({ "base": base, "ttl_s": 900 }),
        )
        .await;
        println!("pool.get #{i}: {:.1} s", t.elapsed().as_secs_f32());
        let name = r["device"]["name"].as_str().unwrap().to_string();
        let _guard = AvdGuard::new(r["device"]["avd"].as_str().unwrap());
        let d = state.device(&name).await.unwrap();
        assert_eq!(d.root.mode, RootMode::Magisk);
        assert_eq!(d.root.magisk_version.as_deref(), Some(version.as_str()));
        let own = d.launch.ramdisk.clone().unwrap();
        assert!(
            own.starts_with(state.paths().avd_home().join(format!("{}.avd", d.avd))),
            "{} boots its own ramdisk copy: {}",
            d.name,
            own.display()
        );
        assert!(ramdisks.insert(own));

        let adb = Adb::new(r["connect"]["adb_path"].as_str().unwrap())
            .device(r["serial"].as_str().unwrap());
        let out = adb.shell(&format!("ls {MARKER}")).await.unwrap();
        assert!(out.contains(MARKER), "{name} was cold-booted: {out}");
        let id = adb.shell("su -c id").await.unwrap();
        assert!(id.contains("uid=0("), "{name}: su -c id → {id}");
        let v = adb.shell("su -c 'magisk -v'").await.unwrap();
        assert!(v.contains(&version), "{name}: magisk -v → {v}");
        call(
            &daemon,
            &ctx,
            "pool.release",
            json!({ "lease": r["lease"]["id"] }),
        )
        .await;
    }
    assert_eq!(jidr_sdk::cache::sha256_file(&pristine).unwrap(), before);
    call(&daemon, &ctx, "base.delete", json!({ "base": base })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": src })).await;
}
