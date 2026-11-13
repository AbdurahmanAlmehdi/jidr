//! `network.shaping.*` through the rooted on-device helper (ADR-0003),
//! offline: a daemon with the fake backend, and a fake guest whose helper
//! runs the real request loop on a host port behind a fake `adb forward`.
//! Packet loss and bad DNS (#17), and how they are kept across cold boots.
//! The real helper on a real emulator is `jidr-root`'s `helper_emulator.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// `JIDR_HELPER_DIR` points the daemon at the fake helper build. It is set
// before the daemon starts any thread, and nextest runs each test in its own
// process.
#![allow(unsafe_code)]

use jidr_client::Client;
use jidr_core::provider::DaemonContext;
use jidr_core::{ErrorCode, RootMode};
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

/// `network.shaping.set` on `P` with extra params (`dns`, `dns_delay_ms`…).
async fn dns(c: &Client, extra: Value) -> jidr_core::JidrResult<Value> {
    let mut p = json!({ "device": "P" });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    c.call("network.shaping.set", p).await
}

/// `P`'s stored shaping in `device.list` (null when off).
async fn stored(s: &Setup) -> Value {
    let list = s.c.call("device.list", json!({})).await.unwrap();
    list.as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "P")
        .unwrap()
        .get("network_shaping")
        .cloned()
        .unwrap_or(Value::Null)
}

/// A cold boot: the device stops, the guest's kernel and helper are gone,
/// and it starts again (running the boot hooks).
async fn cold_boot(s: &Setup) {
    s.c.call("device.stop", json!({ "device": "P" }))
        .await
        .unwrap();
    s.guest.reboot();
    s.c.run("device.start", json!({ "device": "P" }), |_| {})
        .await
        .unwrap();
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
        json!(["device"])
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
        s.guest.state().kernel.qdiscs["wlan0"],
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
    // Nothing given, a delay with the wrong mode, an unknown mode.
    for bad in [
        json!({ "device": "P" }),
        json!({ "device": "P", "dns": "fail", "dns_delay_ms": 100 }),
        json!({ "device": "P", "dns": "flaky" }),
    ] {
        let e =
            s.c.call("network.shaping.set", bad.clone())
                .await
                .unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{bad}: {e}");
    }
    s.d.stop().await;

    let s = setup("adb").await;
    s.guest.state().kernel.netem = false;
    let e = set(&s.c, json!(10)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert_eq!(e.data.unwrap()["helper_error"], "unsupported");
    let e = dns(&s.c, json!({ "dns": "slow" })).await.unwrap_err();
    assert_eq!(e.data.unwrap()["helper_error"], "unsupported");
    // A failed set stores nothing.
    assert_eq!(stored(&s).await, Value::Null);

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

#[tokio::test]
async fn bad_dns_is_set_alongside_the_loss_and_cleared() {
    let s = setup("adb").await;
    set(&s.c, json!(10)).await.unwrap();
    // DNS changes keep the loss; slow defaults to Genymotion's 3000 ms.
    let v = dns(&s.c, json!({ "dns": "slow" })).await.unwrap();
    assert_eq!(
        (&v["loss_pct"], &v["dns"], &v["dns_delay_ms"]),
        (&json!(10.0), &json!("slow"), &json!(3000))
    );
    {
        let g = s.guest.state();
        assert!(g.kernel.qdiscs["wlan0"].starts_with("qdisc prio 1d: root"));
        assert!(g.kernel.qdiscs["wlan0"].contains("parent 1d:1 limit 1000 delay 3s loss 10%"));
        assert!(g.kernel.qdiscs["wlan0"].contains("parent 1d:2 limit 1000 loss 10%"));
        assert_eq!(g.kernel.filters["radio0"].len(), 4);
    }
    // A delay alone means slow.
    let v = dns(&s.c, json!({ "dns_delay_ms": 800 })).await.unwrap();
    assert_eq!(
        (&v["dns"], &v["dns_delay_ms"]),
        (&json!("slow"), &json!(800))
    );
    let v = dns(&s.c, json!({ "dns": "fail" })).await.unwrap();
    assert_eq!(
        (&v["dns"], &v["dns_delay_ms"]),
        (&json!("fail"), &Value::Null)
    );
    assert_eq!(stored(&s).await, json!({ "loss_pct": 10.0, "dns": "fail" }));
    // The loss to 0 keeps DNS failing; DNS back to normal clears it all.
    let v = set(&s.c, json!(0)).await.unwrap();
    assert_eq!((&v["loss_pct"], &v["dns"]), (&json!(0.0), &json!("fail")));
    let v = dns(&s.c, json!({ "dns": "normal" })).await.unwrap();
    assert_eq!((&v["loss_pct"], &v["dns"]), (&json!(0.0), &json!("normal")));
    assert_eq!(s.guest.state().kernel.qdiscs["wlan0"], "qdisc mq 0: root");
    assert_eq!(stored(&s).await, Value::Null, "clearing forgets it");
    s.d.stop().await;
}

/// Regression for #17 (D44): the stored loss and bad DNS come back after a
/// cold boot, which empties the guest kernel.
#[tokio::test]
async fn shaping_is_reapplied_after_a_cold_boot() {
    let s = setup("adb").await;
    dns(
        &s.c,
        json!({ "loss_pct": 7.5, "dns": "slow", "dns_delay_ms": 1500 }),
    )
    .await
    .unwrap();
    cold_boot(&s).await;
    let v =
        s.c.call("network.shaping.get", json!({ "device": "P" }))
            .await
            .unwrap();
    assert_eq!(
        (&v["loss_pct"], &v["dns"], &v["dns_delay_ms"]),
        (&json!(7.5), &json!("slow"), &json!(1500)),
        "{v}"
    );
    assert_eq!(
        s.guest.starts(),
        2,
        "the boot hook started the helper again"
    );

    // Cleared, a cold boot leaves the kernel alone and doesn't even start
    // the helper.
    set(&s.c, json!(0)).await.unwrap();
    dns(&s.c, json!({ "dns": "normal" })).await.unwrap();
    cold_boot(&s).await;
    assert_eq!(s.guest.starts(), 2);
    assert_eq!(s.guest.state().kernel.qdiscs["wlan0"], "qdisc mq 0: root");
    s.d.stop().await;
}

/// A quick boot (or a snapshot) keeps the kernel's shaping: the hook sees it
/// is already there and doesn't apply it twice.
#[tokio::test]
async fn a_quick_boot_that_kept_the_shaping_is_left_alone() {
    let s = setup("adb").await;
    dns(&s.c, json!({ "loss_pct": 10, "dns": "fail" }))
        .await
        .unwrap();
    s.guest.state().kernel.calls.clear();
    s.c.call("device.stop", json!({ "device": "P" }))
        .await
        .unwrap();
    s.c.run("device.start", json!({ "device": "P" }), |_| {})
        .await
        .unwrap();
    let calls = s.guest.state().kernel.calls.clone();
    assert!(
        calls
            .iter()
            .all(|c| c.contains(" show ") || c.contains("/ip ")),
        "only reads: {calls:?}"
    );
    assert_eq!(s.guest.starts(), 1);
    s.d.stop().await;
}

/// A device whose root mode became `none` keeps its stored shaping, skips
/// it at boot with a warning, and can still forget it without root.
#[tokio::test]
async fn without_root_the_stored_shaping_is_skipped_and_can_be_cleared() {
    let s = setup("adb").await;
    set(&s.c, json!(10)).await.unwrap();
    let state = s.d.daemon.state().clone();
    let id = state.device("P").await.unwrap().id;
    state
        .update_device(id, Box::new(|d| d.root.mode = RootMode::None))
        .await
        .unwrap();
    let mut events = s.c.subscribe(&["device.hook_failed"]).await.unwrap();
    cold_boot(&s).await;
    let ev = events.recv().await.unwrap();
    assert_eq!(ev.data["provider"], "widgets");
    assert!(
        ev.data["error"]["message"]
            .as_str()
            .unwrap()
            .contains("root mode is none"),
        "{}",
        ev.data
    );
    assert_eq!(s.guest.starts(), 1, "nothing was started without root");
    assert_eq!(stored(&s).await["loss_pct"], 10.0, "still stored");

    let v = set(&s.c, json!(0)).await.unwrap();
    assert_eq!(v["loss_pct"], 0.0);
    assert_eq!(stored(&s).await, Value::Null);
    s.d.stop().await;
}
