//! Jobs, events and logcat streaming over the socket (fake backend).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::Harness;
use serde_json::{Value, json};

/// Reads notifications until one with `topic` arrives.
async fn next_event(c: &mut common::Conn, topic: &str) -> Value {
    loop {
        let v = c.read().await;
        if v["method"] == "event" && v["params"]["topic"] == topic {
            return v["params"]["data"].clone();
        }
    }
}

#[tokio::test]
async fn cancelling_a_boot_leaves_the_device_stopped() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "P", "image": "35" }))
        .await;
    let _gate = h.backend.gate_starts();
    let job = c.ok("device.start", json!({ "device": "P" })).await["job"].clone();
    let running = c.ok("jobs.list", json!({})).await;
    assert_eq!(running.as_array().unwrap().len(), 1);
    c.ok("jobs.cancel", json!({ "job": job })).await;
    let done = c
        .ok("jobs.wait", json!({ "job": job, "timeout_s": 10 }))
        .await;
    assert_eq!(done["state"], "cancelled", "{done}");
    let d = c.ok("device.details", json!({ "device": "P" })).await;
    assert_eq!(d["device"]["status"]["state"], "stopped");
    assert!(d["device"]["runtime"].is_null());
    assert!(h.backend.running().is_empty(), "nothing left running");
    assert_eq!(c.ok("jobs.list", json!({})).await, json!([]));
    assert_eq!(
        c.ok("jobs.list", json!({ "all": true }))
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The lock was released with the job.
    c.ok("device.edit", json!({ "device": "P", "ram_mb": 2048 }))
        .await;
    h.stop().await;
}

#[tokio::test]
async fn subscriber_receives_boot_progress_and_job_events() {
    let h = Harness::start().await;
    let mut sub = h.client().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "Ev", "image": "35" }))
        .await;
    sub.ok(
        "events.subscribe",
        json!({ "topics": ["device.boot", "job.finished"] }),
    )
    .await;
    let job = c.ok("device.start", json!({ "device": "Ev" })).await["job"].clone();
    let mut stages = Vec::new();
    while stages.last() != Some(&json!("booted")) {
        stages.push(next_event(&mut sub, "device.boot").await["stage"].clone());
    }
    assert_eq!(
        stages,
        vec![
            json!("spawned"),
            json!("grpc_ready"),
            json!("adb_online"),
            json!("booted")
        ]
    );
    let finished = next_event(&mut sub, "job.finished").await;
    assert_eq!(finished["id"], job);
    assert_eq!(finished["state"], "succeeded");
    h.stop().await;
}

#[tokio::test]
async fn topic_filters_and_resubscribe() {
    let h = Harness::start().await;
    let mut sub = h.client().await;
    let mut c = h.client().await;
    sub.ok("events.subscribe", json!({ "topics": ["device.created"] }))
        .await;
    c.ok("device.create", json!({ "name": "One", "image": "35" }))
        .await;
    assert_eq!(next_event(&mut sub, "device.created").await["name"], "One");
    // Replace the filter: now only deletions.
    sub.ok("events.subscribe", json!({ "topics": ["device.deleted"] }))
        .await;
    c.ok("device.create", json!({ "name": "Two", "image": "35" }))
        .await;
    c.ok("device.delete", json!({ "device": "One" })).await;
    let v = sub.read().await;
    assert_eq!(
        v["params"]["topic"], "device.deleted",
        "created events are filtered out: {v}"
    );
    h.stop().await;
}

#[tokio::test]
async fn logcat_follow_streams_lines() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "L", "image": "35" }))
        .await;
    let e = c.err("logcat.follow", json!({ "device": "L" })).await;
    assert_eq!(e["data"]["code"], "not_found", "stopped device");
    c.run("device.start", json!({ "device": "L" })).await;
    c.ok("logcat.follow", json!({ "device": "L" })).await;
    let line = tokio::time::timeout(Duration::from_secs(5), next_event(&mut c, "logcat"))
        .await
        .unwrap();
    assert_eq!(line["line"], "I/fake: hello");
    h.stop().await;
}

#[tokio::test]
async fn failed_jobs_carry_the_error() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "F", "image": "35" }))
        .await;
    h.backend
        .fail_next_start(jidr_core::JidrError::boot_timeout("too slow"));
    let job = c.ok("device.start", json!({ "device": "F" })).await["job"].clone();
    let done = c.ok("jobs.wait", json!({ "job": job })).await;
    assert_eq!(done["state"], "failed");
    assert_eq!(done["error"]["code"], "boot_timeout");
    h.stop().await;
}
