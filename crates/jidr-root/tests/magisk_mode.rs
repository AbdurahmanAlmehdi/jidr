//! Offline tests of Magisk mode: the patched ramdisk, the full `root.set
//! magisk` flow, revert, re-patch, the SDK hash staying untouched, and
//! `root.module.install` refusal on non-Magisk devices.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;
use jidr_core::{ErrorCode, RootMode};
use serde_json::json;

async fn to_magisk(env: &Env, name: &str) -> serde_json::Value {
    // The CLI imports a local APK (`--magisk <path>`) and passes its SHA-256.
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
    .unwrap()
}

#[tokio::test]
async fn magisk_flow_roots_and_verifies_a_device() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;

    let r = to_magisk(&env, "phone").await;
    assert_eq!(r["mode"], "magisk");
    assert_eq!(r["changed"], true);
    assert_eq!(r["applied"], "live");
    assert_eq!(r["verification"]["uid"], 0, "{r}");
    assert_eq!(r["verification"]["command"], "su -c id");
    assert_eq!(r["verification"]["magisk_version"], "30.7:MAGISK:R");
    assert!(
        r["play_integrity"]
            .as_str()
            .unwrap()
            .contains("Play Integrity")
    );

    let d = env.device("phone").await;
    assert_eq!(d.root.mode, RootMode::Magisk);
    assert_eq!(
        d.root.magisk_version.as_deref(),
        Some(common::MAGISK_VERSION)
    );
    assert!(d.root.patched_ramdisk_path.is_some());
    assert!(d.launch.ramdisk.is_some());
    assert!(env.su_granted(), "the shell su was granted");

    let status = env
        .call("root.status", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(status["root"]["mode"], "magisk");
    assert_eq!(status["root"]["health"], "ok");
    assert!(
        status["play_integrity"]
            .as_str()
            .unwrap()
            .contains("Play Integrity")
    );
}

#[tokio::test]
async fn sdk_ramdisk_hash_is_unchanged_by_magisk() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    let before = env.pristine_sha256();
    to_magisk(&env, "phone").await;
    assert_eq!(
        env.pristine_sha256(),
        before,
        "the shared SDK image is untouched"
    );
}

#[tokio::test]
async fn re_patch_when_the_pristine_image_changes() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;

    let d = env.device("phone").await;
    let patched = env.patched_ramdisk(&d);
    let first = std::fs::read(&patched).unwrap();

    // Simulate an image update: the pristine ramdisk gains an entry.
    let image = jidr_daemon::testing::fake_image();
    let pristine = env.studio.join(image.sysdir()).join("ramdisk.img");
    let mut a = jidr_cpio::Archive::new();
    a.insert(jidr_cpio::Entry::file(
        "init",
        0o750,
        b"#!stock init".to_vec(),
    ));
    a.insert(jidr_cpio::Entry::file("extra", 0o644, b"new".to_vec()));
    let ramdisk = jidr_cpio::Ramdisk {
        segments: vec![jidr_cpio::Segment {
            compression: jidr_cpio::Compression::None,
            archive: a,
        }],
        trailing: Vec::new(),
    };
    std::fs::write(&pristine, ramdisk.to_bytes().unwrap()).unwrap();

    // Re-patch: the patched copy is rebuilt from the new pristine.
    to_magisk(&env, "phone").await;
    let second = std::fs::read(&patched).unwrap();
    assert_ne!(first, second, "the patched ramdisk was rebuilt");
    let d = env.device("phone").await;
    assert_eq!(
        d.root.magisk_version.as_deref(),
        Some(common::MAGISK_VERSION)
    );
}

#[tokio::test]
async fn revert_deletes_the_patched_file_and_clears_launch() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;

    let d = env.device("phone").await;
    let patched = env.patched_ramdisk(&d);
    assert!(patched.exists());

    let r = env
        .call("root.set", json!({ "device": "phone", "mode": "none" }))
        .await
        .unwrap();
    assert_eq!(r["applied"], "live");
    assert_eq!(r["verification"]["uid"], 2000, "back to the non-root shell");

    let d = env.device("phone").await;
    assert_eq!(d.root.mode, RootMode::None);
    assert!(d.root.patched_ramdisk_path.is_none());
    assert!(d.launch.ramdisk.is_none(), "launch.ramdisk cleared");
    assert!(!patched.exists(), "the patched file was deleted");
    // The provenance sidecar is deleted together with the patched file.
    assert!(!env.patched_ramdisk(&d).with_extension("img.json").exists());
}

#[tokio::test]
async fn module_install_refuses_non_magisk_devices() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", Some("adb")).await;
    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();

    let err = env
        .call(
            "root.module.install",
            json!({ "device": "phone", "path": "module.zip" }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("Magisk"), "{err}");
}

#[tokio::test]
async fn module_install_command_is_destructive_and_locked() {
    let env = Env::start().await;
    let (_, spec) = env.daemon.registry().get("root.module.install").unwrap();
    assert!(spec.destructive, "D35: destructive for MCP");
    assert!(spec.locks_device);
    assert!(spec.long_running);
    // The magisk override is an optional param of root.set (D30, offline).
    let (_, set) = env.daemon.registry().get("root.set").unwrap();
    let schema = set.params.to_string();
    assert!(schema.contains("magisk_sha256"), "{schema}");
}
