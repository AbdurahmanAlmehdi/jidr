//! `root.helper.status` (ADR-0003) offline: a daemon with the fake backend
//! and a fake guest whose helper runs the real request loop behind a fake
//! `adb forward`. The real helper on an emulator is `helper_emulator.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// `JIDR_HELPER_DIR` points the daemon at the fake helper build. It is set
// before the daemon starts any thread, and nextest runs each test in its own
// process.
#![allow(unsafe_code)]

use jidr_client::Client;
use jidr_core::ErrorCode;
use jidr_daemon::testing::TestDaemon;
use jidr_helper::host::HELPER_DIR_ENV;
use jidr_helper::testing::FakeGuest;
use serde_json::json;

async fn device(root_mode: &str) -> (tempfile::TempDir, TestDaemon, Client, FakeGuest) {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: no other thread exists yet; see the file's note.
    unsafe { std::env::set_var(HELPER_DIR_ENV, tmp.path().join("helper")) };
    let d = TestDaemon::start_with_providers(tmp.path(), vec![jidr_root::provider()])
        .await
        .unwrap();
    let guest = FakeGuest::start(&d.studio.join("platform-tools/adb"), tmp.path()).unwrap();
    let c = Client::connect(&d.paths.socket(), "test").await.unwrap();
    c.call(
        "device.create",
        json!({ "name": "P", "image": "35", "root_mode": root_mode }),
    )
    .await
    .unwrap();
    (tmp, d, c, guest)
}

#[tokio::test]
async fn status_starts_the_helper_and_reports_it() {
    let (tmp, d, c, guest) = device("adb").await;
    // Stopped: refused before anything reaches the device.
    let e = c
        .call("root.helper.status", json!({ "device": "P" }))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    c.run("device.start", json!({ "device": "P" }), |_| {})
        .await
        .unwrap();

    let v = c
        .call("root.helper.status", json!({ "device": "P" }))
        .await
        .unwrap();
    assert_eq!(v["root_mode"], "adb");
    assert_eq!(v["helper"]["uid"], 0);
    assert_eq!(v["helper"]["protocol"], jidr_helper::rpc::PROTOCOL);
    assert_eq!(v["helper"]["selinux"], "u:r:su:s0");
    assert_eq!(v["port"], guest.port);
    let build = jidr_helper::sha256_file(&tmp.path().join("helper/aarch64/jidr-helper")).unwrap();
    assert_eq!(v["helper"]["sha256"], build.as_str());
    assert_eq!((guest.pushes(), guest.starts()), (1, 1));

    // Running: asked again, it is reused, not restarted.
    c.call("root.helper.status", json!({ "device": "P" }))
        .await
        .unwrap();
    assert_eq!((guest.pushes(), guest.starts()), (1, 1));
    d.stop().await;
}

#[tokio::test]
async fn a_device_without_root_is_refused_before_any_push() {
    let (_tmp, d, c, guest) = device("none").await;
    c.run("device.start", json!({ "device": "P" }), |_| {})
        .await
        .unwrap();
    let e = c
        .call("root.helper.status", json!({ "device": "P" }))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("root mode"), "{}", e.message);

    assert_eq!(guest.pushes(), 0);

    d.stop().await;
}
