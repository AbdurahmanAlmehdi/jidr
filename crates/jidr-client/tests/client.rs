//! `jidr-client` against an in-process daemon (fake backend).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use jidr_client::{Client, SpawnOptions, shutdown_daemon};
use jidr_core::{ErrorCode, JidrPaths, JobState};
use jidr_daemon::testing::TestDaemon;
use serde_json::json;

#[tokio::test]
async fn calls_jobs_and_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let d = TestDaemon::start(tmp.path()).await.unwrap();
    let c = Client::connect(&d.paths.socket(), "test").await.unwrap();
    assert_eq!(c.daemon().protocol, "0.1");
    assert_eq!(c.call("device.list", json!({})).await.unwrap(), json!([]));
    c.call("device.create", json!({ "name": "P", "image": "35" }))
        .await
        .unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let dev = c
        .run("device.start", json!({ "device": "P" }), move |j| {
            s.lock().unwrap().push(j.state)
        })
        .await
        .unwrap();
    assert_eq!(dev["status"]["state"], "running");
    assert!(!seen.lock().unwrap().is_empty(), "progress reported");

    let e = c
        .call("device.start", json!({ "device": "nope" }))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(e.code.exit_code(), 3);
    let e = c
        .call("device.start", json!({ "devise": "P" }))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("devise"));

    // A failed job surfaces its error code.
    c.call("device.stop", json!({ "device": "P" }))
        .await
        .unwrap();
    d.backend
        .fail_next_start(jidr_core::JidrError::boot_timeout("slow"));
    let e = c
        .run("device.start", json!({ "device": "P" }), |_| {})
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::BootTimeout);

    // Cancel through the client.
    let _gate = d.backend.gate_starts();
    let v = c
        .call("device.start", json!({ "device": "P" }))
        .await
        .unwrap();
    let job = jidr_client::pending_job(&v).unwrap();
    c.cancel_job(job).await.unwrap();
    assert_eq!(
        c.wait_job(job, None).await.unwrap().state,
        JobState::Cancelled
    );
    d.stop().await;
}

#[tokio::test]
async fn events_arrive_on_the_subscription() {
    let tmp = tempfile::tempdir().unwrap();
    let d = TestDaemon::start(tmp.path()).await.unwrap();
    let c = Client::connect(&d.paths.socket(), "test").await.unwrap();
    let mut rx = c.subscribe(&["device.created"]).await.unwrap();
    c.call("device.create", json!({ "name": "E", "image": "35" }))
        .await
        .unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ev.topic, "device.created");
    assert_eq!(ev.data["name"], "E");
    d.stop().await;
}

#[tokio::test]
async fn stale_daemon_is_detected_and_can_be_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = JidrPaths::under(tmp.path().join("j"));
    let studio = jidr_daemon::testing::fake_studio(&tmp.path().join("studio")).unwrap();
    let daemon = jidr_daemon::Daemon::builder(
        jidr_daemon::DaemonConfig {
            paths: paths.clone(),
            studio_sdk: Some(studio),
            idle_timeout: None,
        },
        Arc::new(jidr_backend::fake::FakeBackend::new()),
    )
    .version("0.0.0-old")
    .build()
    .unwrap();
    let serving = tokio::spawn(daemon.serve(std::future::pending()));
    while !paths.socket().exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let e = Client::connect(&paths.socket(), "test").await.unwrap_err();
    assert_eq!(e.code, ErrorCode::DaemonStale);
    assert_eq!(e.data.unwrap()["daemon_version"], "0.0.0-old");
    shutdown_daemon(&paths.socket()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!paths.socket().exists());
}

#[tokio::test]
async fn spawns_jidr_daemon_when_none_is_running() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = JidrPaths::under(tmp.path().join("j"));
    // A stand-in `jidr` that records how it was called and never listens.
    let marker = tmp.path().join("args.txt");
    let fake = tmp.path().join("jidr");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$@\" > {}\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let opts = SpawnOptions {
        program: Some(fake),
        restart_stale: false,
        timeout: Some(Duration::from_secs(1)),
    };
    let e = Client::connect_or_spawn(&paths, "test", &opts)
        .await
        .unwrap_err();
    assert!(e.message.contains("did not start"), "{e}");
    // The spawned process may lag behind under load.
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let log = std::fs::read_to_string(paths.logs.join("daemon.log")).unwrap_or_default();
    let args =
        std::fs::read_to_string(&marker).unwrap_or_else(|_| format!("no marker; log: {log}"));
    assert_eq!(args.trim(), "daemon");
    assert!(paths.logs.join("daemon.log").exists());
}

#[tokio::test]
async fn connect_or_spawn_uses_a_running_daemon() {
    let tmp = tempfile::tempdir().unwrap();
    let d = TestDaemon::start(tmp.path()).await.unwrap();
    let opts = SpawnOptions {
        program: Some("/nonexistent/jidr".into()),
        ..Default::default()
    };
    let c = Client::connect_or_spawn(&d.paths, "test", &opts)
        .await
        .unwrap();
    c.call("daemon.status", json!({})).await.unwrap();
    d.stop().await;
    // The connection is gone: calls fail instead of hanging.
    let e = tokio::time::timeout(Duration::from_secs(5), c.call("device.list", json!({})))
        .await
        .unwrap();
    assert!(e.is_err());
}
