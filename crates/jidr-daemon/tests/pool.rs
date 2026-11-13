//! Bases, the pool and leases (D42, Phase 13d) on the fake backend.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::{Duration, Utc};
use common::{Conn, Harness};
use serde_json::{Value, json};

/// A lean base `Base` made from a running device.
async fn base(c: &mut Conn) -> Value {
    c.ok(
        "device.create",
        json!({ "name": "Src", "image": "35", "profile": "agent-lean", "root_mode": "adb" }),
    )
    .await;
    c.run("device.start", json!({ "device": "Src" })).await;
    c.run(
        "base.create",
        json!({ "from_device": "Src", "name": "Base" }),
    )
    .await
}

fn avd_dir(h: &Harness, d: &Value) -> std::path::PathBuf {
    h.paths
        .avd_home()
        .join(format!("{}.avd", d["avd"].as_str().unwrap()))
}

#[tokio::test]
async fn pool_base_create_get_and_release() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let b = base(&mut c).await;
    assert_eq!(b["base"], true, "{b}");
    assert_eq!(b["root"]["mode"], "adb", "root travels (D29)");
    assert!(
        avd_dir(&h, &b)
            .join("snapshots/jidr-base/ram.bin")
            .is_file()
    );
    let src = c.ok("device.details", json!({ "device": "Src" })).await;
    assert_eq!(src["device"]["status"]["state"], "stopped");
    // A base is read-only.
    let job = c.ok("device.start", json!({ "device": "Base" })).await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert!(
        done["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pool.get"),
        "{done}"
    );

    let r = c
        .run("pool.get", json!({ "base": "Base", "ttl_s": 600 }))
        .await;
    let d = &r["device"];
    assert_eq!(d["status"]["state"], "running", "{r}");
    assert_eq!(d["agent_owned"], true);
    assert_eq!(d["pool_base"], b["id"]);
    assert!(r["serial"].as_str().unwrap().starts_with("emulator-"));
    assert_eq!(r["lease"]["holder"]["kind"], "ttl");
    let launches = h.backend.launches();
    let opts = &launches.last().unwrap().1;
    assert_eq!(opts.snapshot.as_deref(), Some("jidr-base"));
    assert!(!opts.snapshot_save && !opts.cold_boot);
    let list = c.ok("pool.list", json!({})).await;
    assert_eq!(list[0]["base"], "Base");
    assert_eq!(c.ok("base.list", json!({})).await[0]["leases"], 1);

    // A base with leased clones can't go.
    let e = c.err("base.delete", json!({ "base": "Base" })).await;
    assert_eq!(e["data"]["code"], "busy", "{e}");

    let dir = avd_dir(&h, d);
    assert!(dir.is_dir());
    c.ok("pool.release", json!({ "lease": r["lease"]["id"] }))
        .await;
    assert!(!dir.exists(), "the clone's AVD is deleted");
    assert!(h.backend.running().is_empty());
    let names: Vec<String> = c
        .ok("device.list", json!({}))
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Src", "Base"]);
    assert_eq!(c.ok("pool.list", json!({})).await, json!([]));
    c.ok("base.delete", json!({ "base": "Base" })).await;
    h.stop().await;
}

#[tokio::test]
async fn pool_ttl_lease_expires_and_renews() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    base(&mut c).await;
    let r = c
        .run("pool.get", json!({ "base": "Base", "ttl_s": 60 }))
        .await;
    let lease = r["lease"]["id"].clone();
    let state = h.daemon.state().clone();
    assert!(
        state
            .reap(Utc::now() + Duration::seconds(30))
            .await
            .is_empty()
    );
    let renewed = c
        .ok("pool.renew", json!({ "lease": lease, "ttl_s": 3600 }))
        .await;
    assert!(renewed["expires_at"].is_string());
    assert!(
        state
            .reap(Utc::now() + Duration::seconds(120))
            .await
            .is_empty(),
        "renewed"
    );
    let reaped = state.reap(Utc::now() + Duration::seconds(3601)).await;
    assert_eq!(reaped.len(), 1);
    assert!(h.backend.running().is_empty());
    assert_eq!(
        c.ok("device.list", json!({}))
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );
    h.stop().await;
}

/// A connection lease outlives its connection by 2 minutes, then is reaped.
#[tokio::test]
async fn pool_connection_lease_reaped_after_grace() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    base(&mut c).await;
    let mut agent = h.client().await;
    let r = agent.run("pool.get", json!({ "base": "Base" })).await;
    assert_eq!(r["lease"]["holder"]["kind"], "connection", "{r}");
    assert!(r["lease"]["expires_at"].is_null());
    let state = h.daemon.state().clone();
    assert!(
        state.reap(Utc::now() + Duration::days(1)).await.is_empty(),
        "connected"
    );
    drop(agent);
    // The daemon notices the closed socket.
    for _ in 0..100 {
        if !c.ok("pool.list", json!({})).await[0]["lease"]["expires_at"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        state
            .reap(Utc::now() + Duration::seconds(100))
            .await
            .is_empty(),
        "in grace"
    );
    assert_eq!(
        state.reap(Utc::now() + Duration::seconds(121)).await.len(),
        1
    );
    assert_eq!(c.ok("pool.list", json!({})).await, json!([]));
    assert!(h.backend.running().is_empty());
    h.stop().await;
}

/// 8 GB × 75% = 6 GB: one lean clone (4 GB) fits. A second pool.get waits,
/// then fails; one that waits long enough gets the slot when it frees up.
#[tokio::test]
async fn pool_full_waits_then_errors() {
    let h = Harness::start_with_memory(8192).await;
    let mut c = h.client().await;
    base(&mut c).await;
    let first = c
        .run("pool.get", json!({ "base": "Base", "ttl_s": 600 }))
        .await;
    let t = std::time::Instant::now();
    let job = c
        .ok(
            "pool.get",
            json!({ "base": "Base", "ttl_s": 600, "timeout_s": 1 }),
        )
        .await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert!(
        t.elapsed() >= std::time::Duration::from_secs(1),
        "it waited"
    );
    assert_eq!(done["error"]["code"], "resource_exhausted", "{done}");
    assert!(
        done["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pool is full")
    );
    assert_eq!(
        c.ok("pool.list", json!({})).await.as_array().unwrap().len(),
        1
    );
    assert_eq!(
        c.ok("device.list", json!({}))
            .await
            .as_array()
            .unwrap()
            .len(),
        3,
        "the failed clone was cleaned up"
    );

    let job = c
        .ok(
            "pool.get",
            json!({ "base": "Base", "ttl_s": 600, "timeout_s": 20 }),
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    c.ok("pool.release", json!({ "device": first["device"]["name"] }))
        .await;
    let done = c
        .ok("jobs.wait", json!({ "job": job["job"], "timeout_s": 8 }))
        .await;
    assert_eq!(done["state"], "succeeded", "{done}");
    h.stop().await;
}

/// D4: the daemon stays up while leases exist, even with no client.
#[tokio::test]
async fn pool_leases_keep_the_daemon_alive() {
    let h = Harness::start().await;
    {
        let mut c = h.client().await;
        base(&mut c).await;
        let r = c
            .run("pool.get", json!({ "base": "Base", "ttl_s": 600 }))
            .await;
        c.ok("device.stop", json!({ "device": r["device"]["name"] }))
            .await;
    }
    for _ in 0..100 {
        if h.daemon.client_count() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(h.daemon.client_count(), 0);
    assert_eq!(h.daemon.state().running_count(), 0);
    assert!(!h.daemon.is_idle(), "a lease keeps it up");
    h.daemon
        .state()
        .reap(Utc::now() + Duration::seconds(601))
        .await;
    assert!(h.daemon.is_idle());
    h.stop().await;
}

#[tokio::test]
async fn pool_refuses_stopped_sources_and_non_bases() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "Off", "image": "35" }))
        .await;
    let job = c
        .ok("base.create", json!({ "from_device": "Off", "name": "B" }))
        .await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert_eq!(done["error"]["code"], "not_found", "{done}");
    let e = c.err("pool.release", json!({})).await;
    assert_eq!(e["data"]["code"], "usage");
    let job = c.ok("pool.get", json!({ "base": "Off" })).await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert!(
        done["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not a base"),
        "{done}"
    );
    h.stop().await;
}

#[tokio::test]
async fn pool_refuses_a_writable_system_source() {
    use jidr_core::provider::DaemonContext;
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok(
        "device.create",
        json!({ "name": "Rw", "image": "35", "root_mode": "adb" }),
    )
    .await;
    let st = h.daemon.state();
    let d = st.device("Rw").await.unwrap();
    st.update_device(
        d.id,
        Box::new(|d: &mut jidr_core::Device| d.root.writable_system = true),
    )
    .await
    .unwrap();
    c.run("device.start", json!({ "device": "Rw" })).await;
    let job = c
        .ok("base.create", json!({ "from_device": "Rw", "name": "B" }))
        .await;
    let done = c.ok("jobs.wait", json!({ "job": job["job"] })).await;
    assert_eq!(done["error"]["code"], "usage", "{done}");
    assert!(
        done["error"]["message"]
            .as_str()
            .unwrap()
            .contains("writable /system"),
        "{done}"
    );
    h.stop().await;
}
