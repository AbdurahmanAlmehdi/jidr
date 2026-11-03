//! Offline tests of system CA certificates: `root.cacert.*` on a Full-root
//! device against the fake adb (which keeps the module's files), the
//! refusals on other modes and bad files, and the renewed-CA index.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;

use common::Env;
use jidr_core::ErrorCode;
use serde_json::json;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

async fn magisk_phone(env: &Env) {
    env.create("phone", "35:google_apis", None).await;
    let entry = env
        .call("cache.import", json!({ "path": env.apk }))
        .await
        .unwrap();
    env.call(
        "root.set",
        json!({ "device": "phone", "mode": "magisk", "magisk_sha256": entry["sha256"] }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn install_list_verify_and_remove() {
    let env = Env::start().await;
    magisk_phone(&env).await;

    let r = env
        .call("root.cacert.list", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(r["certs"], json!([]));
    assert_eq!(r["store"], "/apex/com.android.conscrypt/cacerts");

    let r = env
        .call(
            "root.cacert.install",
            json!({ "device": "phone", "path": fixture("jidr-test-ca.pem") }),
        )
        .await
        .unwrap();
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(r["cert"]["file"], "a4ac454c.0");
    assert_eq!(r["cert"]["subject"], "Jidr Test CA");
    assert_eq!(env.cacert_files(), ["a4ac454c.0"]);
    let d = env.device("phone").await;
    assert!(d.launch.force_cold_boot || d.status.is_active(), "rebooted");

    // The same certificate as DER: already trusted, nothing written.
    let applied = env.cacert_applied();
    let r = env
        .call(
            "root.cacert.install",
            json!({ "device": "phone", "path": fixture("jidr-test-ca.der") }),
        )
        .await
        .unwrap();
    assert_eq!(r["changed"], false, "{r}");
    assert_eq!(env.cacert_applied(), applied);

    // A renewed CA keeps the subject, so it gets the next index.
    let r = env
        .call(
            "root.cacert.install",
            json!({ "device": "phone", "path": fixture("jidr-test-ca-renewed.pem") }),
        )
        .await
        .unwrap();
    assert_eq!(r["cert"]["file"], "a4ac454c.1");
    let v = env
        .call("root.cacert.verify", json!({ "device": "phone" }))
        .await
        .unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["certs"].as_array().unwrap().len(), 2);

    let r = env
        .call(
            "root.cacert.remove",
            json!({ "device": "phone", "cert": "a4ac454c.0" }),
        )
        .await
        .unwrap();
    assert_eq!(r["removed"][0]["file"], "a4ac454c.0");
    assert_eq!(env.cacert_files(), ["a4ac454c.1"]);
    let r = env
        .call(
            "root.cacert.remove",
            json!({ "device": "phone", "all": true }),
        )
        .await
        .unwrap();
    assert_eq!(r["removed"].as_array().unwrap().len(), 1);
    assert!(env.cacert_files().is_empty());
    let r = env
        .call(
            "root.cacert.remove",
            json!({ "device": "phone", "all": true }),
        )
        .await
        .unwrap();
    assert_eq!(r["changed"], false, "nothing left to remove");
}

#[tokio::test]
async fn refusals() {
    let env = Env::start().await;
    env.create("plain", "35:google_apis", Some("adb")).await;
    let pem = fixture("jidr-test-ca.pem");
    let e = env
        .call(
            "root.cacert.install",
            json!({ "device": "plain", "path": pem }),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("Full-root"), "{e}");

    magisk_phone(&env).await;
    let bad = env.tmp.path().join("key.pem");
    std::fs::write(
        &bad,
        "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
    )
    .unwrap();
    for path in [bad, env.tmp.path().join("missing.pem")] {
        let e = env
            .call(
                "root.cacert.install",
                json!({ "device": "phone", "path": path }),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{e}");
    }
    for params in [
        json!({ "device": "phone" }),
        json!({ "device": "phone", "cert": "a4ac454c.0", "all": true }),
    ] {
        let e = env.call("root.cacert.remove", params).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{e}");
    }
    let e = env
        .call(
            "root.cacert.remove",
            json!({ "device": "phone", "cert": "a4ac454c.0" }),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound, "{e}");

    env.call("device.stop", json!({ "device": "phone" }))
        .await
        .unwrap();
    let e = env
        .call("root.cacert.list", json!({ "device": "phone" }))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage, "stopped: {e}");
}
