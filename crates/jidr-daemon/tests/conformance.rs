//! Protocol conformance over a real Unix socket (fake backend, no emulator).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::Harness;
use serde_json::json;

#[tokio::test]
async fn bad_json_is_parse_error() {
    let h = Harness::start().await;
    let mut c = h.conn().await;
    c.send_raw("{not json").await;
    let r = c.read().await;
    assert_eq!(r["error"]["code"], -32700);
    assert_eq!(r["id"], serde_json::Value::Null);
    // The connection survives.
    c.send_raw(r#"{"jsonrpc":"1.0","id":7,"method":"hello"}"#)
        .await;
    let r = c.read().await;
    assert_eq!(r["error"]["code"], -32600);
    assert_eq!(r["id"], 7);
    h.stop().await;
}

#[tokio::test]
async fn hello_is_required_and_stale_clients_are_refused() {
    let h = Harness::start().await;
    let mut c = h.conn().await;
    assert_eq!(c.err("device.list", json!({})).await["code"], -32600);
    let e = c.err("hello", json!({ "version": "0.0.0-old" })).await;
    assert_eq!(e["code"], -32008);
    assert_eq!(e["data"]["code"], "daemon_stale");
    assert_eq!(e["data"]["exit_code"], 8);
    assert_eq!(e["data"]["details"]["daemon_version"], jidr_daemon::VERSION);
    let ok = c
        .ok("hello", json!({ "version": jidr_daemon::VERSION }))
        .await;
    assert_eq!(ok["protocol"], "0.1");
    c.ok("device.list", json!({})).await;
    h.stop().await;
}

#[tokio::test]
async fn unknown_method_and_params() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    assert_eq!(c.err("device.explode", json!({})).await["code"], -32601);
    let e = c
        .err("device.create", json!({ "nmae": "x", "image": "35" }))
        .await;
    assert_eq!(e["code"], -32602);
    assert!(e["message"].as_str().unwrap().contains("nmae"), "{e}");
    let e = c
        .err("device.start", json!({ "device": "p", "cold_boot": "yes" }))
        .await;
    assert_eq!(e["code"], -32602, "type errors are usage errors too: {e}");
    let e = c.err("hello", json!({ "version": "x", "extra": 1 })).await;
    assert!(e["message"].as_str().unwrap().contains("extra"), "{e}");
    h.stop().await;
}

#[tokio::test]
async fn commands_list_describes_every_command() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let list = c.ok("commands.list", json!({})).await;
    let list = list.as_array().unwrap();
    let start = list.iter().find(|s| s["id"] == "device.start").unwrap();
    assert_eq!(start["longRunning"], true);
    assert_eq!(start["locksDevice"], true);
    assert_eq!(start["params"]["properties"]["device"]["type"], "string");
    let delete = list.iter().find(|s| s["id"] == "device.delete").unwrap();
    assert_eq!(delete["destructive"], true);
    for s in list {
        assert!(!s["label"].as_str().unwrap().is_empty(), "{s}");
        assert_eq!(s["params"]["type"], "object", "{s}");
    }
    h.stop().await;
}

#[tokio::test]
async fn device_lifecycle() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let d = c
        .ok(
            "device.create",
            json!({ "name": "Phone A", "image": "35:google_apis", "ram_mb": 3072 }),
        )
        .await;
    assert_eq!(d["root"]["mode"], "adb", "D7: adb where allowed");
    assert_eq!(d["avd"], "Phone_A");
    let e = c
        .err("device.create", json!({ "name": "phone a", "image": "35" }))
        .await;
    assert_eq!(e["data"]["code"], "usage");
    let e = c
        .err(
            "device.create",
            json!({ "name": "PS", "image": "35:google_apis_playstore", "root_mode": "adb" }),
        )
        .await;
    assert_eq!(e["data"]["code"], "usage");
    let ps = c
        .ok(
            "device.create",
            json!({ "name": "PS", "image": "35:google_apis_playstore" }),
        )
        .await;
    assert_eq!(ps["root"]["mode"], "none");
    let e = c
        .err("device.create", json!({ "name": "B", "image": "29" }))
        .await;
    assert_eq!(e["data"]["code"], "sdk", "image not installed: {e}");

    let details = c.ok("device.details", json!({ "device": "Phone A" })).await;
    assert_eq!(details["config"]["hw.ramSize"], "3072");

    let e = c
        .err("device.connect_info", json!({ "device": "Phone A" }))
        .await;
    assert_eq!(e["data"]["code"], "not_found");
    let started = c.run("device.start", json!({ "device": "Phone A" })).await;
    assert_eq!(started["status"]["state"], "running");
    let ci = c
        .ok("device.connect_info", json!({ "device": "Phone A" }))
        .await;
    assert!(ci["grpc_token"].as_str().unwrap().starts_with("fake-token"));
    assert!(
        ci["grpc_endpoint"]
            .as_str()
            .unwrap()
            .starts_with("127.0.0.1:")
    );

    // adb-backed commands go through the fake adb.
    let apk = h.tmp.path().join("app.apk");
    std::fs::write(&apk, b"apk").unwrap();
    c.ok("file.install", json!({ "device": "Phone A", "apk": apk }))
        .await;
    let log = c
        .ok("logcat.dump", json!({ "device": "Phone A", "lines": 10 }))
        .await;
    assert!(log["text"].as_str().unwrap().contains("hello"));

    let e = c
        .err(
            "device.edit",
            json!({ "device": "Phone A", "ram_mb": 4096 }),
        )
        .await;
    assert_eq!(
        e["data"]["code"], "busy",
        "hardware edits need a stopped device"
    );
    c.ok("device.stop", json!({ "device": "Phone A" })).await;
    c.ok(
        "device.edit",
        json!({ "device": "Phone A", "ram_mb": 4096, "name": "Phone B" }),
    )
    .await;
    let copy = c
        .ok(
            "device.clone",
            json!({ "device": "Phone B", "name": "Phone C" }),
        )
        .await;
    assert_eq!(copy["launch"]["force_cold_boot"], true);
    c.run("device.start", json!({ "device": "Phone C" })).await;
    let launches = h.backend.launches();
    assert!(
        launches.last().unwrap().1.cold_boot,
        "a clone's first boot is cold"
    );
    c.ok("device.reset", json!({ "device": "Phone C" })).await;
    c.ok("device.delete", json!({ "device": "Phone C" })).await;
    c.ok("device.delete", json!({ "device": "Phone B" })).await;
    c.ok("device.delete", json!({ "device": "PS" })).await;
    assert_eq!(c.ok("device.list", json!({})).await, json!([]));
    assert!(h.backend.running().is_empty());
    h.stop().await;
}

#[tokio::test]
async fn busy_lock_names_the_holder() {
    let h = Harness::start().await;
    let mut a = h.client().await;
    let mut b = h.client().await;
    a.ok("device.create", json!({ "name": "Slow", "image": "35" }))
        .await;
    let gate = h.backend.gate_starts();
    // A starts: the job holds the lock while the backend blocks.
    let started = a.ok("device.start", json!({ "device": "Slow" })).await;
    assert_eq!(started["pending"], true);
    let job = started["job"].clone();
    let e = b.err("device.stop", json!({ "device": "Slow" })).await;
    assert_eq!(e["code"], -32004);
    assert_eq!(e["data"]["code"], "busy");
    assert_eq!(e["data"]["details"]["op"], "device.start");
    assert_eq!(e["data"]["details"]["holder"], job, "the holder is the job");
    // Reads don't take the lock.
    b.ok("device.details", json!({ "device": "Slow" })).await;
    gate.add_permits(1);
    let done = a.ok("jobs.wait", json!({ "job": job })).await;
    assert_eq!(done["state"], "succeeded");
    assert_eq!(done["result"]["status"]["state"], "running");
    b.ok("device.stop", json!({ "device": "Slow" })).await;
    h.stop().await;
}

#[tokio::test]
async fn crash_is_reported_not_restarted() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    c.ok("device.create", json!({ "name": "Crashy", "image": "35" }))
        .await;
    let d = c.run("device.start", json!({ "device": "Crashy" })).await;
    let pid = d["runtime"]["pid"].as_u64().unwrap() as u32;
    h.backend.crash(pid);
    let mut state = json!(null);
    for _ in 0..100 {
        let list = c.ok("device.list", json!({})).await;
        state = list[0]["status"].clone();
        if state["state"] == "crashed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(state["state"], "crashed", "{state}");
    assert_eq!(state["reason"], "fake crash");
    assert_eq!(h.backend.launches().len(), 1, "D9: no auto-restart");
    // A crashed device can be started again.
    c.run("device.start", json!({ "device": "Crashy" })).await;
    h.stop().await;
}

#[tokio::test]
async fn socket_is_private_and_single_instance() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::start().await;
    let mode = std::fs::metadata(h.paths.socket())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let err = jidr_daemon::server::bind(&h.paths.socket()).unwrap_err();
    assert_eq!(err.code, jidr_core::ErrorCode::Busy);
    let socket = h.paths.socket();
    h.stop().await;
    assert!(!socket.exists(), "socket removed on shutdown");
}
