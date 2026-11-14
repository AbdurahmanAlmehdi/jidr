//! Offline tests of `file.flash` routing (ADR-0004) and the pre-checks
//! `root.module.install` shares with it, against the fake backend and the
//! fake adb from `common`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Env, test_module_zip, zip_of};
use jidr_core::ErrorCode;
use serde_json::json;

const RECOVERY: &str = "META-INF/com/google/android/update-binary";

async fn magisk_device(env: &Env, name: &str) {
    env.create(name, "35:google_apis", None).await;
    env.call("device.start", json!({ "device": name }))
        .await
        .unwrap();
    let entry = env
        .call("cache.import", json!({ "path": env.apk }))
        .await
        .unwrap();
    env.call(
        "root.set",
        json!({ "device": name, "mode": "magisk", "magisk_sha256": entry["sha256"] }),
    )
    .await
    .unwrap();
}

async fn plain_device(env: &Env, name: &str, mode: &str) {
    env.create(name, "35:google_apis", Some(mode)).await;
    env.call("device.start", json!({ "device": name }))
        .await
        .unwrap();
}

fn pushed_to(env: &Env) -> Vec<String> {
    std::fs::read_to_string(env.adb_file("push-dests"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_module_installs_and_is_checked_on_a_full_root_device() {
    let env = Env::start().await;
    magisk_device(&env, "phone").await;
    let zip = test_module_zip(env.tmp.path());

    let r = env
        .call("file.flash", json!({ "device": "phone", "path": zip }))
        .await
        .unwrap();
    assert_eq!(r["kind"], "magisk_module", "{r}");
    assert_eq!(r["action"], "module_installed");
    assert_eq!(r["module"]["id"], "jidrtest_module");
    assert_eq!(r["module"]["name"], "Jidr test module");
    assert_eq!(r["module"]["zygisk"], false);
    assert_eq!(r["zygisk_enabled"], true, "Magisk's emulator default");
    assert!(r["note"].is_null(), "{r}");
    assert_eq!(r["verification"]["uid"], 0);
    assert_eq!(env.adb_count("module-installs"), 1);
    assert_eq!(
        pushed_to(&env).last().map(String::as_str),
        Some("/data/local/tmp/jidr-module.zip")
    );
}

#[tokio::test]
async fn a_zygisk_module_installs_and_warns_when_zygisk_is_off() {
    let env = Env::start().await;
    magisk_device(&env, "phone").await;
    let zip = zip_of(
        env.tmp.path(),
        "z.zip",
        &[
            ("module.prop", b"id=zmod\nname=Z\nversion=2"),
            ("zygisk/arm64-v8a.so", b"elf"),
        ],
    );

    let r = env
        .call("file.flash", json!({ "device": "phone", "path": zip }))
        .await
        .unwrap();
    assert_eq!(r["kind"], "zygisk_module", "{r}");
    assert_eq!(r["module"]["zygisk"], true);
    assert!(r["note"].is_null(), "Zygisk is on by default: {r}");

    std::fs::write(env.adb_file("zygisk-off"), b"1").unwrap();
    let r = env
        .call("file.flash", json!({ "device": "phone", "path": zip }))
        .await
        .unwrap();
    assert_eq!(r["zygisk_enabled"], false, "{r}");
    assert!(r["note"].as_str().unwrap().contains("Zygisk is off"), "{r}");
}

#[tokio::test]
async fn modules_are_copied_to_download_without_full_root() {
    let env = Env::start().await;
    plain_device(&env, "phone", "adb").await;
    let zip = test_module_zip(env.tmp.path());

    let r = env
        .call("file.flash", json!({ "device": "phone", "path": zip }))
        .await
        .unwrap();
    assert_eq!(r["kind"], "magisk_module", "{r}");
    assert_eq!(r["action"], "pushed");
    assert_eq!(r["remote"], "/sdcard/Download/jidrtest-module.zip");
    assert!(r["note"].as_str().unwrap().contains("Full root"), "{r}");
    assert_eq!(env.adb_count("module-installs"), 0);
    assert_eq!(pushed_to(&env), ["/sdcard/Download/"]);
}

#[tokio::test]
async fn other_zips_are_copied_to_download() {
    let env = Env::start().await;
    magisk_device(&env, "phone").await;
    let zip = zip_of(env.tmp.path(), "photos.zip", &[("a.png", b"png")]);

    let r = env
        .call("file.flash", json!({ "device": "phone", "path": zip }))
        .await
        .unwrap();
    assert_eq!(r["kind"], "other", "{r}");
    assert_eq!(r["action"], "pushed");
    assert!(r["note"].is_null());
    assert_eq!(env.adb_count("module-installs"), 0);
}

#[tokio::test]
async fn recovery_riru_and_image_zips_are_refused_on_every_device() {
    let env = Env::start().await;
    magisk_device(&env, "rooted").await;
    plain_device(&env, "plain", "none").await;
    let pushes_before = pushed_to(&env).len();
    let cases = [
        (
            zip_of(
                env.tmp.path(),
                "gapps.zip",
                &[(RECOVERY, b"#!/sbin/sh"), ("Core/x.tar.xz", b"x")],
            ),
            "recovery",
            "no recovery",
        ),
        (
            zip_of(env.tmp.path(), "ota.zip", &[("payload.bin", b"x")]),
            "image",
            "no fastboot",
        ),
        (
            zip_of(
                env.tmp.path(),
                "riru.zip",
                &[("module.prop", b"id=rmod"), ("riru/lib64/librmod.so", b"x")],
            ),
            "riru_module",
            "Zygisk build",
        ),
    ];
    for device in ["rooted", "plain"] {
        for (zip, kind, says) in &cases {
            let err = env
                .call("file.flash", json!({ "device": device, "path": zip }))
                .await
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::Usage, "{device} {kind}: {err}");
            assert_eq!(err.data.as_ref().unwrap()["kind"], *kind);
            assert!(err.message.contains(says), "{err}");
        }
    }
    assert_eq!(env.adb_count("module-installs"), 0);
    assert_eq!(pushed_to(&env).len(), pushes_before, "nothing was copied");
}

#[tokio::test]
async fn broken_modules_name_the_fix_before_touching_the_device() {
    let env = Env::start().await;
    magisk_device(&env, "phone").await;
    let nested = zip_of(
        env.tmp.path(),
        "Mod-main.zip",
        &[("Mod-main/", b""), ("Mod-main/module.prop", b"id=mod")],
    );
    let bad_id = zip_of(env.tmp.path(), "bad.zip", &[("module.prop", b"id=../x")]);
    let reserved = zip_of(
        env.tmp.path(),
        "res.zip",
        &[("module.prop", b"id=jidr_cacerts")],
    );
    for (zip, says) in [
        (&nested, "Zip the contents of Mod-main"),
        (&bad_id, "isn't a valid Magisk module id"),
        (&reserved, "reserved for Jidr"),
    ] {
        for method in ["file.flash", "root.module.install"] {
            let err = env
                .call(method, json!({ "device": "phone", "path": zip }))
                .await
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::Usage, "{method}: {err}");
            assert!(err.message.contains(says), "{method}: {err}");
        }
    }
    assert_eq!(env.adb_count("module-installs"), 0);
}

/// Regression (#72): `root.module.install` handed any zip to Magisk, which
/// failed on the device with "This zip is not a Magisk module!".
#[tokio::test]
async fn module_install_refuses_zips_that_arent_modules() {
    let env = Env::start().await;
    magisk_device(&env, "phone").await;
    let gapps = zip_of(env.tmp.path(), "gapps.zip", &[(RECOVERY, b"#!/sbin/sh")]);
    let photos = zip_of(env.tmp.path(), "photos.zip", &[("a.png", b"png")]);

    let err = env
        .call(
            "root.module.install",
            json!({ "device": "phone", "path": gapps }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert_eq!(err.data.unwrap()["kind"], "recovery");
    let err = env
        .call(
            "root.module.install",
            json!({ "device": "phone", "path": photos }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("isn't a Magisk module"), "{err}");
    assert_eq!(env.adb_count("module-installs"), 0);

    let r = env
        .call(
            "root.module.install",
            json!({ "device": "phone", "path": test_module_zip(env.tmp.path()) }),
        )
        .await
        .unwrap();
    assert_eq!(r["installed"], true, "{r}");
    assert_eq!(r["module"]["id"], "jidrtest_module");
}

#[tokio::test]
async fn flash_needs_a_running_device_and_an_existing_file() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    let zip = test_module_zip(env.tmp.path());
    let err = env
        .call("file.flash", json!({ "device": "phone", "path": zip }))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("not running"), "{err}");

    env.call("device.start", json!({ "device": "phone" }))
        .await
        .unwrap();
    let err = env
        .call(
            "file.flash",
            json!({ "device": "phone", "path": env.tmp.path().join("missing.zip") }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Usage);
    assert!(err.message.contains("no such file"), "{err}");
}

#[tokio::test]
async fn flash_command_is_destructive_and_locked() {
    let env = Env::start().await;
    let (_, spec) = env.daemon.registry().get("file.flash").unwrap();
    assert!(
        spec.destructive,
        "it can install a module that runs as root"
    );
    assert!(spec.locks_device);
    assert!(spec.long_running);
}
