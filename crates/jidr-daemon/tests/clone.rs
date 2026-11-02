//! Snapshot-keeping clones (D42, Phase 13b) on the fake backend.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Harness;
use serde_json::json;

/// Fakes what the emulator leaves after `snapshot.save "Logged in"`.
fn fake_snapshot(avd_dir: &std::path::Path, id: &str, label: &str, ram: &[u8]) {
    let dir = avd_dir.join("snapshots").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("snapshot.pb"), b"pb").unwrap();
    std::fs::write(dir.join("ram.bin"), ram).unwrap();
    std::fs::create_dir_all(avd_dir.join("snapshots/default_boot")).unwrap();
    std::fs::create_dir_all(avd_dir.join("jidr")).unwrap();
    std::fs::write(
        avd_dir.join("jidr/snapshots.json"),
        json!({ id: { "label": label, "root_mode": "adb", "created_at": "2026-11-02T00:00:00Z" } })
            .to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn clone_keeping_a_snapshot_resumes_it_once() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "Src", "image": "35" }))
        .await;
    let src_dir = c.ok("device.details", json!({ "device": "Src" })).await["avd_dir"]
        .as_str()
        .unwrap()
        .to_string();
    fake_snapshot(
        std::path::Path::new(&src_dir),
        "Logged_in",
        "Logged in",
        &[1; 64],
    );

    let d = c
        .ok(
            "device.clone",
            json!({ "device": "Src", "name": "Copy", "keep_snapshot": "Logged in" }),
        )
        .await;
    assert_eq!(d["launch"]["resume_snapshot"], "Logged_in", "{d}");
    assert_eq!(d["launch"]["force_cold_boot"], false);
    assert_eq!(d["root"]["mode"], "adb", "root travels with the copy (D29)");
    let copy_dir = c.ok("device.details", json!({ "device": "Copy" })).await["avd_dir"]
        .as_str()
        .unwrap()
        .to_string();
    let copy_dir = std::path::Path::new(&copy_dir);
    assert!(copy_dir.join("snapshots/Logged_in/ram.bin").is_file());
    assert!(!copy_dir.join("snapshots/default_boot").exists());

    // First start: -snapshot, and no exit save into it. Then normal boots.
    c.run("device.start", json!({ "device": "Copy" })).await;
    c.ok("device.stop", json!({ "device": "Copy" })).await;
    c.run("device.start", json!({ "device": "Copy" })).await;
    let launches = h.backend.launches();
    let (first, second) = (&launches[0].1, &launches[1].1);
    assert_eq!(first.snapshot.as_deref(), Some("Logged_in"));
    assert!(!first.snapshot_save && !first.cold_boot);
    assert_eq!(second.snapshot, None);
    assert!(second.snapshot_save, "quick boot is back on");
    let d = c.ok("device.details", json!({ "device": "Copy" })).await;
    assert!(d["device"]["launch"].get("resume_snapshot").is_none());
    h.stop().await;
}

#[tokio::test]
async fn clone_without_a_snapshot_still_cold_boots() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "Src", "image": "35" }))
        .await;
    let d = c
        .ok("device.clone", json!({ "device": "Src", "name": "Plain" }))
        .await;
    assert_eq!(d["launch"]["force_cold_boot"], true);
    assert!(d["launch"].get("resume_snapshot").is_none());
    // An unusable snapshot is refused and leaves nothing behind.
    let src_dir = c.ok("device.details", json!({ "device": "Src" })).await["avd_dir"]
        .as_str()
        .unwrap()
        .to_string();
    fake_snapshot(std::path::Path::new(&src_dir), "empty", "empty", b"");
    let e = c
        .err(
            "device.clone",
            json!({ "device": "Src", "name": "Bad", "keep_snapshot": "empty" }),
        )
        .await;
    assert_eq!(e["data"]["code"], "sdk", "{e}");
    let e = c
        .err(
            "device.clone",
            json!({ "device": "Src", "name": "Bad", "keep_snapshot": "nope" }),
        )
        .await;
    assert_eq!(e["data"]["code"], "not_found", "{e}");
    let names: Vec<String> = c
        .ok("device.list", json!({}))
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Src", "Plain"]);
    h.stop().await;
}
