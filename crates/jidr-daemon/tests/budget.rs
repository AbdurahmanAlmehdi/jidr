//! The host RAM budget (D42, Phase 13a) on the fake backend.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Harness;
use serde_json::json;

/// 16 GB at 75%: a 12 GB budget, three lean devices (4 GB each).
#[tokio::test]
async fn budget_fits_refuses_and_force_overrides() {
    let h = Harness::start_with_memory(16384).await;
    let mut c = h.client().await;
    for n in ["L1", "L2", "L3", "L4"] {
        c.ok(
            "device.create",
            json!({ "name": n, "image": "35", "profile": "agent-lean" }),
        )
        .await;
    }
    let b = c.ok("host.budget", json!({})).await;
    assert_eq!(
        (
            b["total_mb"].clone(),
            b["budget_mb"].clone(),
            b["used_mb"].clone()
        ),
        (json!(16384), json!(12288), json!(0)),
        "{b}"
    );
    for n in ["L1", "L2", "L3"] {
        c.run("device.start", json!({ "device": n })).await;
    }
    let b = c.ok("host.budget", json!({})).await;
    assert_eq!(b["used_mb"], 12288, "{b}");
    assert_eq!(b["devices"].as_array().unwrap().len(), 3);
    assert_eq!(b["devices"][0]["expected_mb"], 4096);

    // The fourth is refused, naming the devices that use the budget.
    let job = c.ok("device.start", json!({ "device": "L4" })).await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert_eq!(done["state"], "failed", "{done}");
    let e = &done["error"];
    assert_eq!(e["code"], "resource_exhausted", "{e}");
    let msg = e["message"].as_str().unwrap();
    assert!(
        msg.contains("L1") && msg.contains("L3") && msg.contains("hibernate"),
        "{msg}"
    );
    assert_eq!(e["data"]["needed_mb"], 4096);
    assert_eq!(e["data"]["devices"].as_array().unwrap().len(), 3);
    let d = c.ok("device.details", json!({ "device": "L4" })).await;
    assert_eq!(d["device"]["status"]["state"], "stopped", "left as it was");
    assert_eq!(h.backend.launches().len(), 3, "never launched");

    // --force starts it anyway.
    let d = c
        .run("device.start", json!({ "device": "L4", "force": true }))
        .await;
    assert_eq!(d["status"]["state"], "running");
    assert_eq!(c.ok("host.budget", json!({})).await["used_mb"], 16384);

    // Stopping frees the budget.
    c.ok("device.stop", json!({ "device": "L1" })).await;
    c.ok("device.stop", json!({ "device": "L4" })).await;
    c.run("device.start", json!({ "device": "L1" })).await;
    h.stop().await;
}

/// The lean profile launches without audio or cameras.
#[tokio::test]
async fn budget_lean_profile_launch_options() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let d = c
        .ok(
            "device.create",
            json!({ "name": "Lean", "image": "35", "profile": "agent-lean" }),
        )
        .await;
    assert_eq!(d["launch"]["audio"], false);
    assert_eq!(d["launch"]["camera_back"], "none");
    c.ok("device.create", json!({ "name": "Default", "image": "35" }))
        .await;
    c.run("device.start", json!({ "device": "Lean" })).await;
    c.run("device.start", json!({ "device": "Default" })).await;
    let launches = h.backend.launches();
    let (lean, default) = (&launches[0].1, &launches[1].1);
    assert!(!lean.audio);
    assert_eq!(lean.camera_back.as_deref(), Some("none"));
    assert_eq!(lean.camera_front.as_deref(), Some("none"));
    assert!(default.audio);
    assert_eq!(default.camera_back, None);
    let cfg = c.ok("device.details", json!({ "device": "Lean" })).await["config"].clone();
    assert_eq!(cfg["hw.ramSize"], "2048");
    assert_eq!(cfg["hw.lcd.width"], "720");
    assert_eq!(cfg["disk.dataPartition.size"], "4096M");
    h.stop().await;
}

/// The measured footprint corrects the cost model: live while running, and
/// the last run's peak afterwards.
#[tokio::test]
async fn measured_footprint_corrects_the_budget() {
    let h = Harness::start_with_memory(16384).await;
    let mut c = h.client().await;
    let d = c
        .ok("device.create", json!({ "name": "Big", "image": "35" }))
        .await;
    let avd = d["avd"].as_str().unwrap().to_string();
    c.ok("device.create", json!({ "name": "Other", "image": "35" }))
        .await;
    h.backend.set_footprint(&avd, 5000);
    c.run("device.start", json!({ "device": "Big" })).await;
    h.daemon.state().sample_footprints().await;
    let b = c.ok("host.budget", json!({})).await;
    assert_eq!(b["devices"][0]["expected_mb"], 6144, "4096 + 2048");
    assert_eq!(b["devices"][0]["measured_mb"], 5000);
    assert_eq!(b["used_mb"], 6144, "the larger of the two counts");
    h.backend.set_footprint(&avd, 7800);
    h.daemon.state().sample_footprints().await;
    h.backend.set_footprint(&avd, 7000);
    h.daemon.state().sample_footprints().await;
    let b = c.ok("host.budget", json!({})).await;
    assert_eq!(b["devices"][0]["measured_mb"], 7000, "latest sample");
    assert_eq!(b["used_mb"], 7000);
    let stopped = c.ok("device.stop", json!({ "device": "Big" })).await;
    assert_eq!(stopped["footprint_mb"], 7800, "the run's peak is kept");

    // Next time Big is expected to need 7.8 GB: with Other running (6 GB),
    // it no longer fits in 12 GB.
    c.run("device.start", json!({ "device": "Other" })).await;
    let job = c.ok("device.start", json!({ "device": "Big" })).await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert_eq!(done["error"]["code"], "resource_exhausted", "{done}");
    assert_eq!(done["error"]["data"]["needed_mb"], 7800);
    h.stop().await;
}

/// Two starts racing for the last slot: exactly one gets it.
#[tokio::test]
async fn budget_parallel_starts_cannot_both_take_the_last_slot() {
    // 8 GB × 75% = 6 GB: one lean device (4 GB) fits, two don't.
    let h = Harness::start_with_memory(8192).await;
    let mut c = h.client().await;
    for n in ["A", "B"] {
        c.ok(
            "device.create",
            json!({ "name": n, "image": "35", "profile": "agent-lean" }),
        )
        .await;
    }
    let gate = h.backend.gate_starts();
    let a = c.ok("device.start", json!({ "device": "A" })).await;
    let b = c.ok("device.start", json!({ "device": "B" })).await;
    gate.add_permits(2);
    let a = c.ok("jobs.wait", json!({ "job": a["job"] })).await;
    let b = c.ok("jobs.wait", json!({ "job": b["job"] })).await;
    let states = [a["state"].clone(), b["state"].clone()];
    assert!(
        states.contains(&json!("succeeded")) && states.contains(&json!("failed")),
        "{a}\n{b}"
    );
    let failed = if a["state"] == "failed" { &a } else { &b };
    assert_eq!(failed["error"]["code"], "resource_exhausted");
    assert_eq!(h.backend.launches().len(), 1);
    h.stop().await;
}

#[tokio::test]
async fn budget_percent_setting() {
    let h = Harness::start_with_memory(16384).await;
    let mut c = h.client().await;
    assert_eq!(c.ok("settings.get", json!({})).await["budget_percent"], 75);
    let s = c.ok("settings.set", json!({ "budget_percent": 50 })).await;
    assert_eq!(s["budget_percent"], 50);
    assert_eq!(c.ok("host.budget", json!({})).await["budget_mb"], 8192);
    for bad in [5, 101] {
        let e = c
            .err("settings.set", json!({ "budget_percent": bad }))
            .await;
        assert_eq!(e["data"]["code"], "usage", "{e}");
    }
    h.stop().await;
}
