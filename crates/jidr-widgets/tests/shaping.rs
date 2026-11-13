//! `network.shaping.*` through the rooted on-device helper (ADR-0003),
//! offline: a daemon with the fake backend, and a fake guest whose helper
//! runs the real request loop on a host port behind a fake `adb forward`.
//! The real helper on a real emulator is `shaping_emulator.rs`.
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
use serde_json::{Value, json};

struct Setup {
    _tmp: tempfile::TempDir,
    d: TestDaemon,
    c: Client,
    guest: FakeGuest,
}

/// A running device `P` in `root_mode` on a fake guest.
async fn setup(root_mode: &str) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: no other thread exists yet; see the file's note.
    unsafe { std::env::set_var(HELPER_DIR_ENV, tmp.path().join("helper")) };
    let d = TestDaemon::start_with_providers(tmp.path(), vec![jidr_widgets::provider()])
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
    c.run("device.start", json!({ "device": "P" }), |_| {})
        .await
        .unwrap();
    Setup {
        _tmp: tmp,
        d,
        c,
        guest,
    }
}

async fn set(c: &Client, loss_pct: Value) -> jidr_core::JidrResult<Value> {
    c.call(
        "network.shaping.set",
        json!({ "device": "P", "loss_pct": loss_pct }),
    )
    .await
}

#[tokio::test]
async fn loss_is_set_and_cleared_through_the_helper() {
    let s = setup("adb").await;
    let list = s.c.call("commands.list", json!({})).await.unwrap();
    let spec = |id: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} not listed"))
    };
    assert_eq!(spec("network.shaping.set")["locksDevice"], true);
    assert_eq!(
        spec("network.shaping.set")["params"]["required"],
        json!(["device", "loss_pct"])
    );

    let v = set(&s.c, json!(10)).await.unwrap();
    assert_eq!(v["loss_pct"], 10.0);
    let names: Vec<&str> = v["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["radio0", "wlan0"]);
    assert_eq!(v["interfaces"][1]["qdisc"], "netem");
    assert_eq!(
        s.guest.state().qdiscs["wlan0"],
        "qdisc netem 8001: root refcnt 2 limit 1000 loss 10%"
    );
    // Pushed and started once; later calls reuse the running helper and
    // the device's forward.
    assert_eq!((s.guest.pushes(), s.guest.starts()), (1, 1));
    let v =
        s.c.call("network.shaping.get", json!({ "device": "P" }))
            .await
            .unwrap();
    assert_eq!(v["loss_pct"], 10.0);
    let v = set(&s.c, json!(0)).await.unwrap();
    assert_eq!(v["loss_pct"], 0.0);
    assert_eq!(v["interfaces"][1]["qdisc"], "mq");
    assert_eq!((s.guest.pushes(), s.guest.starts()), (1, 1));
    let forwards = std::fs::read_to_string(s.guest.dir.join("forwards")).unwrap();
    assert_eq!(forwards.lines().count(), 1, "{forwards}");
    s.d.stop().await;
}

/// After a reboot (or a snapshot taken before the helper ran) nothing
/// answers; the next call starts it again. An older build is replaced.
#[tokio::test]
async fn the_helper_is_restarted_after_a_reboot_and_replaced_when_stale() {
    let s = setup("magisk").await;
    set(&s.c, json!(5)).await.unwrap();
    s.guest.reboot();
    let v =
        s.c.call("network.shaping.get", json!({ "device": "P" }))
            .await
            .unwrap();
    assert_eq!(v["loss_pct"], 0.0, "a reboot drops the shaping");
    assert_eq!(s.guest.starts(), 2);

    s.guest.run_older_build().unwrap();
    set(&s.c, json!(2.5)).await.unwrap();
    assert_eq!(s.guest.starts(), 3);
    assert_eq!(s.guest.pushes(), 3);
    s.d.stop().await;
}

#[tokio::test]
async fn bad_input_no_root_and_no_netem_are_typed_errors() {
    let s = setup("none").await;
    for bad in [json!(101), json!(-1), json!("ten")] {
        let e = set(&s.c, bad.clone()).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{bad}: {e}");
    }
    // No root: refused before anything is pushed.
    let e = set(&s.c, json!(10)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("root"), "{}", e.message);
    assert_eq!(s.guest.pushes(), 0);
    s.d.stop().await;

    let s = setup("adb").await;
    s.guest.state().netem = false;
    let e = set(&s.c, json!(10)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert_eq!(e.data.unwrap()["helper_error"], "unsupported");

    // A helper that didn't come up as root is refused.
    s.guest.reboot();
    s.guest.state().uid = 2000;
    let e = set(&s.c, json!(10)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::RootFailed, "{e}");
    s.d.stop().await;
}

#[tokio::test]
async fn a_stopped_device_is_not_found() {
    let s = setup("adb").await;
    s.c.call("device.stop", json!({ "device": "P" }))
        .await
        .unwrap();
    let e =
        s.c.call("network.shaping.get", json!({ "device": "P" }))
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    s.d.stop().await;
}
