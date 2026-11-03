//! System CA certificates on a real Full-root emulator: install puts the
//! certificate in the store apps read (the Conscrypt APEX on API 34+) as
//! Zygote sees it, a second install of it changes nothing, and remove takes
//! it out again.
//!
//! Same setup as `magisk_emulator` (`JIDR_TEST_IMAGE`, `JIDR_TEST_SDK`,
//! optional `JIDR_TEST_MAGISK`); run it on an API 34+ image, e.g.
//! `JIDR_TEST_IMAGE='system-images;android-35;google_apis;arm64-v8a'`.
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
        format!("system-images;android-35;google_apis;{abi}")
    })
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

async fn call(daemon: &Arc<Daemon>, ctx: &CallCtx, method: &str, params: Value) -> Value {
    let (p, _) = daemon.registry().get(method).unwrap();
    p.call(ctx, method, params)
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

/// `su -c <cmd>` in Zygote's mount namespace: what every app sees.
async fn as_zygote(adb: &Adb, cmd: &str) -> String {
    // `pidof` may print several pids (API 30): use the first.
    adb.shell(&format!(
        "su -c 'Z=$(pidof zygote64); [ -n \"$Z\" ] || Z=$(pidof zygote); nsenter -t ${{Z%% *}} -m -- {cmd}'"
    ))
    .await
    .unwrap_or_else(|e| e.message)
}

#[tokio::test(flavor = "multi_thread")]
async fn cacert_install_verify_and_remove() {
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
    kill_orphans("root-cacert");
    let name = unique_avd("root-cacert");

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
    let api: u32 = adb()
        .await
        .getprop("ro.build.version.sdk")
        .await
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let store = if api >= 34 {
        "/apex/com.android.conscrypt/cacerts"
    } else {
        "/system/etc/security/cacerts"
    };
    let stock = as_zygote(&adb().await, &format!("ls {store}")).await;
    let stock_count = stock.lines().count();
    println!("API {api}: {stock_count} stock certificates in {store}");
    assert!(stock_count > 50, "{stock}");

    let r = call(&daemon, &ctx, "root.cacert.list", json!({ "device": name })).await;
    assert_eq!(r["certs"], json!([]), "{r}");
    assert_eq!(r["store"], store);

    // 1. Install: after the reboot the store apps read has it, byte for byte.
    let pem = fixture("jidr-test-ca.pem");
    let r = call(
        &daemon,
        &ctx,
        "root.cacert.install",
        json!({ "device": name, "path": pem }),
    )
    .await;
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(r["cert"]["file"], "a4ac454c.0", "{r}");
    assert_eq!(r["cert"]["subject"], "Jidr Test CA");
    let a = adb().await;
    let listing = as_zygote(&a, &format!("ls -Z {store}/a4ac454c.0")).await;
    assert!(
        listing.contains("a4ac454c.0")
            && listing.contains(if api >= 34 {
                "system_security_cacerts_file"
            } else {
                "system_file"
            }),
        "{listing}"
    );
    let all = as_zygote(&a, &format!("ls {store}")).await;
    assert_eq!(
        all.lines().count(),
        stock_count + 1,
        "stock certificates kept"
    );
    // An app process sees it too, not just Zygote.
    let app = a
        .shell(&format!(
            "su -c 'P=$(pidof com.android.systemui); nsenter -t ${{P%% *}} -m -- ls {store}/a4ac454c.0'"
        ))
        .await
        .unwrap_or_else(|e| e.message);
    assert!(app.contains("a4ac454c.0"), "{app}");
    let v = call(
        &daemon,
        &ctx,
        "root.cacert.verify",
        json!({ "device": name }),
    )
    .await;
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["certs"][0]["trusted"], true, "{v}");
    let r = call(&daemon, &ctx, "root.verify", json!({ "device": name })).await;
    assert_eq!(r["uid"], 0, "root survives the module: {r}");

    // 2. The same certificate again (as DER): nothing to do, no reboot.
    let r = call(
        &daemon,
        &ctx,
        "root.cacert.install",
        json!({ "device": name, "path": fixture("jidr-test-ca.der") }),
    )
    .await;
    assert_eq!(r["changed"], false, "{r}");

    // 3. Remove: gone from the store after the reboot, stock ones untouched.
    let r = call(
        &daemon,
        &ctx,
        "root.cacert.remove",
        json!({ "device": name, "cert": "a4ac454c.0" }),
    )
    .await;
    assert_eq!(r["removed"][0]["file"], "a4ac454c.0", "{r}");
    let a = adb().await;
    let all = as_zygote(&a, &format!("ls {store}")).await;
    assert_eq!(all.lines().count(), stock_count, "{all}");
    assert!(!all.contains("a4ac454c.0"));
    let r = call(&daemon, &ctx, "root.cacert.list", json!({ "device": name })).await;
    assert_eq!(r["certs"], json!([]), "{r}");
    let module = a
        .shell("su -c 'ls /data/adb/modules/jidr_cacerts' 2>&1")
        .await
        .unwrap_or_else(|e| e.message);
    assert!(
        module.contains("No such file"),
        "the module is gone: {module}"
    );

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}
