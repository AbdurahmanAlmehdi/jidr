//! Offline tests of build-prop spoofing (Phase 9f): `device.props.get/set`
//! on a Full-root device, the re-patched ramdisk carrying the boot hook, the
//! refusal on other modes, and the props dropping when root is reverted.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;
use jidr_core::ErrorCode;
use serde_json::json;

async fn to_magisk(env: &Env, name: &str) {
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
    .unwrap();
}

fn hook(env: &Env, d: &jidr_core::Device) -> Option<String> {
    let bytes = std::fs::read(env.patched_ramdisk(d)).unwrap();
    let ramdisk = jidr_cpio::Ramdisk::parse(&bytes).unwrap();
    let a = &ramdisk.segments[0].archive;
    a.get("overlay.d/jidr-props.rc")?;
    a.get("overlay.d/sbin/jidr-props.sh")
        .map(|e| String::from_utf8_lossy(&e.data).into_owned())
}

#[tokio::test]
async fn props_set_repatches_and_the_next_start_applies_them() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;

    let r = env
        .call("device.props.get", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(r["props"], json!({}), "stock until set");
    assert_eq!(r["applies"], true);
    assert_eq!(r["restart_required"], false);
    let d = env.device("phone").await;
    assert!(hook(&env, &d).is_none(), "no hook for stock props");

    let r = env
        .call(
            "device.props.set",
            json!({ "device": "phone", "props": {
                "model": " Pixel 8 ", "brand": "google", "manufacturer": "",
                "security_patch": "2026-10-05",
            }}),
        )
        .await
        .unwrap();
    assert_eq!(
        r["props"],
        json!({ "model": "Pixel 8", "brand": "google", "security_patch": "2026-10-05" }),
        "trimmed, empty means stock"
    );
    assert_eq!(r["restart_required"], true, "{r}");

    let d = env.device("phone").await;
    assert!(d.launch.force_cold_boot, "the next start is a cold boot");
    let script = hook(&env, &d).expect("the hook is in the patched ramdisk");
    assert!(
        script.contains("resetprop -n ro.product.model 'Pixel 8'"),
        "{script}"
    );
    assert!(script.contains("ro.build.version.security_patch '2026-10-05'"));

    // Back to stock: the hook goes away again.
    env.call(
        "device.props.set",
        json!({ "device": "phone", "props": {} }),
    )
    .await
    .unwrap();
    let d = env.device("phone").await;
    assert!(hook(&env, &d).is_none());
}

#[tokio::test]
async fn props_are_refused_off_full_root_and_bad_values_name_the_field() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    let e = env
        .call(
            "device.props.set",
            json!({ "device": "phone", "props": { "model": "Pixel 8" } }),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("Full-root"), "{}", e.message);

    to_magisk(&env, "phone").await;
    let e = env
        .call(
            "device.props.set",
            json!({ "device": "phone", "props": { "security_patch": "October" } }),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert_eq!(e.data.unwrap()["field"], "security_patch");
    let e = env
        .call(
            "device.props.set",
            json!({ "device": "phone", "props": { "hardware": "ranchu" } }),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage, "identity properties only");
}

#[tokio::test]
async fn reverting_root_drops_the_props() {
    let env = Env::start().await;
    env.create("phone", "35:google_apis", None).await;
    to_magisk(&env, "phone").await;
    env.call(
        "device.props.set",
        json!({ "device": "phone", "props": { "model": "Pixel 8" } }),
    )
    .await
    .unwrap();
    env.call("root.set", json!({ "device": "phone", "mode": "none" }))
        .await
        .unwrap();
    let r = env
        .call("device.props.get", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(r["props"], json!({}));
    assert_eq!(r["applies"], false);
}
