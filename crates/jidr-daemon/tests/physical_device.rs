//! `physical.*` against a real phone (#92). Skipped unless
//! `JIDR_TEST_PHONE_SERIAL` names a phone plugged in with USB debugging on
//! and authorized. Uses the real `adb` (`JIDR_TEST_SDK`, default
//! `~/Library/Android/sdk`).
//!
//! Harmless to the phone: it reads, takes a screenshot and copies one file
//! into `/sdcard/Download/jidr-test/`, which it removes again. No installs,
//! input, reboot or other writes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;

use common::Harness;
use serde_json::json;

fn phone() -> Option<String> {
    std::env::var("JIDR_TEST_PHONE_SERIAL")
        .ok()
        .filter(|s| !s.is_empty())
}

fn real_adb() -> PathBuf {
    let sdk = std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join("Library/Android/sdk"));
    sdk.join("platform-tools/adb")
}

fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

#[tokio::test]
async fn a_real_phone_lists_screenshots_and_copies_files() {
    let Some(serial) = phone() else {
        eprintln!("skipped: set JIDR_TEST_PHONE_SERIAL to a connected phone's serial");
        return;
    };
    let h = Harness::start().await;
    // The fake SDK's adb becomes the real one.
    let adb = h.tmp.path().join("studio/platform-tools/adb");
    std::fs::remove_file(&adb).unwrap();
    std::os::unix::fs::symlink(real_adb(), &adb).unwrap();
    let mut c = h.client().await;

    let list = c.ok("physical.list", json!({})).await;
    let me = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["serial"] == serial.as_str())
        .unwrap_or_else(|| panic!("{serial} isn't listed: {list}"))
        .clone();
    assert_eq!(me["state"], "ready", "{me}");
    assert!(me["model"].is_string(), "{me}");
    assert!(me["api"].as_u64().unwrap() >= 21, "{me}");
    assert!(me["battery"]["level"].as_u64().unwrap() <= 100, "{me}");
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|p| !p["serial"].as_str().unwrap().starts_with("emulator-")),
        "no emulators: {list}"
    );

    let d = c.ok("physical.details", json!({ "device": serial })).await;
    assert!(d["screen"]["width"].as_u64().unwrap() > 0, "{d}");

    let shot = h.tmp.path().join("phone.png");
    let s = c
        .ok(
            "physical.screenshot",
            json!({ "device": serial, "path": shot }),
        )
        .await;
    assert!(std::fs::read(&shot).unwrap().starts_with(b"\x89PNG"));
    assert_eq!(s["width"], d["screen"]["width"], "{s} vs {d}");

    let local = h.tmp.path().join("jidr-phone-test.txt");
    std::fs::write(&local, "jidr phone test\n").unwrap();
    let dir = "/sdcard/Download/jidr-test/";
    c.ok(
        "physical.shell",
        json!({ "device": serial, "command": format!("mkdir -p {dir}") }),
    )
    .await;
    c.ok(
        "physical.push",
        json!({ "device": serial, "local": local, "remote": dir }),
    )
    .await;
    let back = h.tmp.path().join("back.txt");
    c.ok(
        "physical.pull",
        json!({ "device": serial, "remote": format!("{dir}jidr-phone-test.txt"), "local": back }),
    )
    .await;
    assert_eq!(std::fs::read_to_string(&back).unwrap(), "jidr phone test\n");
    let rm = c
        .ok(
            "physical.shell",
            json!({ "device": serial, "command": format!("rm -r {dir}") }),
        )
        .await;
    assert_eq!(rm["exit_code"], 0, "{rm}");

    let log = c
        .ok("physical.logcat", json!({ "device": serial, "lines": 20 }))
        .await;
    assert!(!log["text"].as_str().unwrap().is_empty());
    h.stop().await;
}
