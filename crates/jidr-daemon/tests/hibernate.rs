//! Hibernate and auto-hibernate (D42, Phase 13c) on the fake backend.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::Harness;
use serde_json::json;

#[tokio::test]
async fn hibernate_saves_stops_and_resumes() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let d = c
        .ok(
            "device.create",
            json!({ "name": "H", "image": "35", "root_mode": "adb" }),
        )
        .await;
    let avd = d["avd"].as_str().unwrap().to_string();
    c.run("device.start", json!({ "device": "H" })).await;
    let d = c.run("device.hibernate", json!({ "device": "H" })).await;
    assert_eq!(d["status"]["state"], "hibernated", "{d}");
    assert_eq!(d["launch"]["resume_snapshot"], "jidr-hibernate");
    assert!(d["runtime"].is_null());
    assert_eq!(
        h.backend.saves(),
        vec![(avd.clone(), "jidr-hibernate".to_string())]
    );
    assert_eq!(h.backend.unsaved_stops(), vec![avd.clone()], "no exit save");
    assert!(h.backend.running().is_empty(), "frees all its RAM");
    assert_eq!(c.ok("host.budget", json!({})).await["used_mb"], 0);

    // The root tag travels with the snapshot (D29).
    let dir = c.ok("device.details", json!({ "device": "H" })).await["avd_dir"]
        .as_str()
        .unwrap()
        .to_string();
    let tags: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::Path::new(&dir).join("jidr/snapshots.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(tags["jidr-hibernate"]["root_mode"], "adb");

    // Stop leaves it hibernated; start resumes the snapshot once.
    let s = c.ok("device.stop", json!({ "device": "H" })).await;
    assert_eq!(s["status"]["state"], "hibernated");
    let d = c.run("device.start", json!({ "device": "H" })).await;
    assert_eq!(d["status"]["state"], "running");
    let opts = &h.backend.launches()[1].1;
    assert_eq!(opts.snapshot.as_deref(), Some("jidr-hibernate"));
    assert!(!opts.cold_boot);
    assert!(d["launch"].get("resume_snapshot").is_none());

    // The resumed run has no exit save: stop saves quick-boot state itself,
    // so the next start quick-boots.
    c.ok("device.stop", json!({ "device": "H" })).await;
    assert_eq!(
        h.backend.saves().last().unwrap(),
        &(avd.clone(), "default_boot".to_string())
    );
    c.run("device.start", json!({ "device": "H" })).await;
    let launches = h.backend.launches();
    let third = &launches[2].1;
    assert!(third.snapshot.is_none() && third.snapshot_save && !third.cold_boot);
    // A normal run's stop leaves it to the emulator's own exit save.
    let n = h.backend.saves().len();
    c.ok("device.stop", json!({ "device": "H" })).await;
    assert_eq!(h.backend.saves().len(), n);
    h.stop().await;
}

#[tokio::test]
async fn hibernate_keeps_running_when_the_save_is_empty() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "E", "image": "35" }))
        .await;
    c.run("device.start", json!({ "device": "E" })).await;
    h.backend.fail_saves();
    let job = c.ok("device.hibernate", json!({ "device": "E" })).await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert_eq!(done["state"], "failed");
    assert_eq!(done["error"]["code"], "sdk");
    assert!(
        done["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ram.bin"),
        "{done}"
    );
    let d = c.ok("device.details", json!({ "device": "E" })).await["device"].clone();
    assert_eq!(
        d["status"]["state"], "running",
        "never stopped on a bad save"
    );
    assert!(h.backend.unsaved_stops().is_empty());
    // Not running: not_found.
    c.ok("device.stop", json!({ "device": "E" })).await;
    let job = c.ok("device.hibernate", json!({ "device": "E" })).await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert_eq!(done["error"]["code"], "not_found");
    h.stop().await;
}

#[tokio::test]
async fn hibernate_is_cleared_by_reset_and_root_changes() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "R", "image": "35" }))
        .await;
    c.run("device.start", json!({ "device": "R" })).await;
    c.run("device.hibernate", json!({ "device": "R" })).await;
    let d = c.ok("device.reset", json!({ "device": "R" })).await;
    assert_eq!(d["status"]["state"], "stopped");
    assert!(d["launch"].get("resume_snapshot").is_none());
    c.run("device.start", json!({ "device": "R" })).await;
    let opts = &h.backend.launches()[1].1;
    assert!(opts.cold_boot && opts.snapshot.is_none());
    h.stop().await;
}

/// The idle timer: off by default, global or per device, a player counts
/// as activity, commands reset it.
#[tokio::test]
async fn hibernate_auto_after_idle_minutes() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let a = c
        .ok("device.create", json!({ "name": "A", "image": "35" }))
        .await;
    c.ok("device.create", json!({ "name": "B", "image": "35" }))
        .await;
    c.run("device.start", json!({ "device": "A" })).await;
    c.run("device.start", json!({ "device": "B" })).await;
    let state = h.daemon.state().clone();
    let later = |m: u64| Instant::now() + Duration::from_secs(m * 60 + 1);

    assert!(
        state.auto_hibernate(later(600)).await.is_empty(),
        "off by default"
    );
    c.ok("settings.set", json!({ "auto_hibernate_minutes": 10 }))
        .await;
    c.ok(
        "device.edit",
        json!({ "device": "B", "auto_hibernate_minutes": 0 }),
    )
    .await;
    assert!(
        state.auto_hibernate(later(5)).await.is_empty(),
        "not idle long enough"
    );

    // A player attached to A keeps it awake.
    let avd_a = a["avd"].as_str().unwrap();
    h.backend.set_clients(avd_a, true);
    assert!(state.auto_hibernate(later(11)).await.is_empty());
    h.backend.set_clients(avd_a, false);
    let done = state.auto_hibernate(later(22)).await;
    assert_eq!(done.len(), 1, "only A; B opted out");
    let list = c.ok("device.list", json!({})).await;
    let st = |n: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == n)
            .unwrap()["status"]["state"]
            .clone()
    };
    assert_eq!(st("A"), "hibernated");
    assert_eq!(st("B"), "running");

    // A command naming a device is activity; null follows the setting again.
    c.ok(
        "device.edit",
        json!({ "device": "B", "auto_hibernate_minutes": null }),
    )
    .await;
    c.ok("device.details", json!({ "device": "B" })).await;
    assert!(
        state
            .auto_hibernate(Instant::now() + Duration::from_secs(60))
            .await
            .is_empty()
    );
    assert_eq!(state.auto_hibernate(later(10)).await.len(), 1);
    h.stop().await;
}

/// A full budget names the longest-idle running devices to hibernate.
#[tokio::test]
async fn hibernate_candidates_in_resource_exhausted() {
    let h = Harness::start_with_memory(8192).await;
    let mut c = h.client().await;
    for n in ["Idle", "New"] {
        c.ok(
            "device.create",
            json!({ "name": n, "image": "35", "profile": "agent-lean" }),
        )
        .await;
    }
    c.run("device.start", json!({ "device": "Idle" })).await;
    let job = c.ok("device.start", json!({ "device": "New" })).await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    let cands = &done["error"]["data"]["hibernate_candidates"];
    assert_eq!(cands[0]["name"], "Idle", "{done}");
    h.stop().await;
}
