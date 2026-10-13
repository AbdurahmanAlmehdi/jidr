//! Cache, log bundles and uninstall (Phase 5b) through the `jidr` binary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;
use serde_json::{Value, json};

#[test]
fn cache_and_magisk_import() {
    let e = Env::new();
    std::fs::create_dir_all(&e.home).unwrap();
    let apk = e.home.join("Magisk-v27.0.apk");
    std::fs::write(&apk, b"magisk").unwrap();
    let entry = e.ok(&["cache", "import", apk.to_str().unwrap()]);
    assert_eq!(entry["sha256"].as_str().unwrap().len(), 64);
    e.fail(
        &[
            "cache",
            "import",
            apk.to_str().unwrap(),
            "--sha256",
            &"0".repeat(64),
        ],
        2,
    );
    assert_eq!(e.ok(&["cache", "list"]).as_array().unwrap().len(), 1);
    // --magisk imports the APK before calling root.set (which this build lacks).
    e.ok(&["device", "create", "M", "--image", "35"]);
    let other = e.home.join("other.apk");
    std::fs::write(&other, b"other magisk").unwrap();
    e.fail(
        &[
            "root",
            "set",
            "M",
            "magisk",
            "--magisk",
            other.to_str().unwrap(),
        ],
        2,
    );
    assert_eq!(e.ok(&["cache", "list"]).as_array().unwrap().len(), 2);
    assert_eq!(e.ok(&["cache", "prune"])["removed"], 2);
    assert_eq!(e.ok(&["cache", "list"]), json!([]));
}

#[test]
fn device_log_bundle() {
    let e = Env::new();
    e.ok(&["device", "create", "Logs", "--image", "35"]);
    e.ok(&["device", "start", "Logs"]);
    let out = e.home.join("bundle.zip");
    let v = e.ok(&["device", "logs", "Logs", "--zip", out.to_str().unwrap()]);
    assert!(out.is_file());
    assert!(
        v["files"].as_u64().unwrap() >= 4,
        "device.json, config.ini, logcat, about: {v}"
    );
    let default = e.ok(&["device", "logs", "Logs"]);
    assert!(default["path"].as_str().unwrap().ends_with(".zip"));
}

#[test]
fn uninstall_purge_removes_everything_jidr_and_nothing_else() {
    let e = Env::new();
    let home = tempfile::tempdir().unwrap();
    let cursor = home.path().join(".cursor/mcp.json");
    std::fs::create_dir_all(cursor.parent().unwrap()).unwrap();
    std::fs::write(
        &cursor,
        r#"{"mcpServers":{"jidr":{"command":"jidr","args":["mcp"]},"keep":{"command":"x"}}}"#,
    )
    .unwrap();
    // Something that looks like Studio's files must survive.
    let android = home.path().join(".android/avd/Pixel.avd");
    std::fs::create_dir_all(&android).unwrap();

    e.ok(&["device", "create", "Gone", "--image", "35"]);
    e.ok(&["device", "start", "Gone"]);
    // Off a terminal, uninstall needs --yes.
    let (code, _) = e.json_with_home(&["uninstall", "--purge"], home.path());
    assert_eq!(code, 2);

    let (code, v) = e.json_with_home(&["uninstall", "--purge", "--yes"], home.path());
    assert_eq!(code, 0, "{v}");
    let r = &v["result"];
    assert_eq!(r["stopped_devices"], json!(["Gone"]));
    assert_eq!(r["daemon_stopped"], true);
    assert_eq!(r["purged"], true);
    assert_eq!(r["mcp"][0]["removed"], json!(["jidr"]));
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(&cursor).unwrap()).unwrap();
    assert!(cfg["mcpServers"].get("jidr").is_none() && cfg["mcpServers"].get("keep").is_some());
    for d in ["data", "cache", "logs"] {
        assert!(!e.home.join(d).exists(), "{d} remains");
    }
    assert!(android.is_dir(), "never touches ~/.android");
    assert!(e.backend().running().is_empty(), "devices stopped");
}

#[test]
fn uninstall_without_purge_keeps_devices() {
    let e = Env::new();
    let home = tempfile::tempdir().unwrap();
    e.ok(&["device", "create", "Kept", "--image", "35"]);
    let (code, v) = e.json_with_home(&["uninstall", "--yes"], home.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["purged"], false);
    assert!(e.home.join("data/devices.json").is_file());
    assert_eq!(v["result"]["kept"][0], json!(e.home.join("data")));
}
