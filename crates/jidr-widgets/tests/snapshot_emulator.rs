//! Phase 10b: save → modify → load restores the state, and a snapshot
//! taken under another root mode is refused (D29).
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use jidr_core::RootMode;
use jidr_core::provider::DaemonContext;
use serde_json::{Value, json};

fn marker(env: &common::Env) -> String {
    env.shell("settings get global jidr_marker")
        .trim()
        .to_string()
}

fn find<'a>(list: &'a Value, name: &str) -> Option<&'a Value> {
    list["snapshots"]
        .as_array()?
        .iter()
        .find(|s| s["name"] == name)
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_round_trip_and_cross_root_refusal() {
    let env = common::Env::boot("widgets10b").await;

    env.shell("settings put global jidr_marker before");
    assert_eq!(marker(&env), "before");
    env.run("snapshot.save", json!({ "name": "Clean install" }))
        .await;
    let list = env.call("snapshot.list", json!({})).await;
    let s = find(&list, "Clean install").unwrap_or_else(|| panic!("{list}"));
    assert_eq!(s["root_mode"], "none");
    assert_eq!(s["restorable"], true);
    assert!(s["size_bytes"].as_u64().unwrap() > 0, "{s}");

    env.shell("settings put global jidr_marker after");
    assert_eq!(marker(&env), "after");
    env.run("snapshot.load", json!({ "name": "Clean install" }))
        .await;
    env.eventually("the marker back to before", |e| marker(e) == "before")
        .await;

    // What root.set does: the device's root mode changes.
    let id = env.daemon.state().device(&env.device).await.unwrap().id;
    env.daemon
        .state()
        .update_device(id, Box::new(|d| d.root.mode = RootMode::Adb))
        .await
        .unwrap();
    let err = env
        .client
        .run(
            "snapshot.load",
            json!({ "device": env.device, "name": "Clean install" }),
            |_| {},
        )
        .await
        .unwrap_err();
    assert!(
        err.message.contains("Not rooted") && err.message.contains("Shell root"),
        "{err}"
    );
    assert_eq!(
        err.data.as_ref().unwrap()["details"]["refused"]
            .as_str()
            .or(err.data.as_ref().unwrap()["refused"].as_str()),
        Some("root_mode"),
        "{err:?}"
    );
    let list = env.call("snapshot.list", json!({})).await;
    assert_eq!(find(&list, "Clean install").unwrap()["restorable"], false);

    env.call("snapshot.delete", json!({ "name": "Clean install" }))
        .await;
    let list = env.call("snapshot.list", json!({})).await;
    assert!(find(&list, "Clean install").is_none(), "{list}");

    env.stop().await;
}
