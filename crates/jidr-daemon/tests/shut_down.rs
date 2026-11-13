//! `shut_down_device`, the stop before a cold boot (#78), on the fake backend.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Harness;
use jidr_core::DeviceStatus;
use jidr_core::provider::DaemonContext;
use serde_json::json;

async fn running(h: &Harness, name: &str) -> (uuid::Uuid, String) {
    let mut c = h.client().await;
    let d = c
        .ok("device.create", json!({ "name": name, "image": "35" }))
        .await;
    c.run("device.start", json!({ "device": name })).await;
    let id = d["id"].as_str().unwrap().parse().unwrap();
    (id, d["avd"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn shut_down_powers_android_off_and_skips_the_exit_save() {
    let h = Harness::start().await;
    let (id, avd) = running(&h, "S").await;
    let d = h.daemon.state().shut_down_device(id).await.unwrap();
    assert_eq!(d.status, DeviceStatus::Stopped);
    assert_eq!(h.backend.power_offs(), vec![avd.clone()]);
    assert_eq!(h.backend.unsaved_stops(), vec![avd]);
    assert!(h.backend.running().is_empty());
}

#[tokio::test]
async fn a_guest_that_stays_on_gets_the_plain_stop() {
    let h = Harness::start().await;
    let (id, avd) = running(&h, "S").await;
    h.backend.stick_power_off();
    let d = h.daemon.state().shut_down_device(id).await.unwrap();
    assert_eq!(d.status, DeviceStatus::Stopped);
    assert_eq!(h.backend.power_offs(), vec![avd]);
    assert!(
        h.backend.unsaved_stops().is_empty(),
        "the emulator's own stop"
    );
    assert!(h.backend.running().is_empty());
}

#[tokio::test]
async fn shut_down_of_a_stopped_device_is_a_no_op() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let d = c
        .ok("device.create", json!({ "name": "S", "image": "35" }))
        .await;
    let id = d["id"].as_str().unwrap().parse().unwrap();
    let d = h.daemon.state().shut_down_device(id).await.unwrap();
    assert_eq!(d.status, DeviceStatus::Stopped);
    assert!(h.backend.power_offs().is_empty());
}
