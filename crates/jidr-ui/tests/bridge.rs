//! The GUI's daemon bridge against an in-process daemon: initial load,
//! event-driven reload, and reconnect after the daemon goes away.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use jidr_client::{Client, SpawnOptions};
use jidr_daemon::testing::TestDaemon;
use jidr_ui::bridge::{Bridge, Conn, Update};
use serde_json::json;

fn wait_for(rx: &mpsc::Receiver<Update>, what: &str, f: impl Fn(&Update) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(u) = rx.recv_timeout(Duration::from_millis(200))
            && f(&u)
        {
            return;
        }
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn loads_follows_events_and_reconnects() {
    let tmp = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let td = rt.block_on(TestDaemon::start(tmp.path())).unwrap();
    let paths = td.paths.clone();

    let (tx, rx) = mpsc::channel();
    let bridge = Bridge::new().unwrap();
    let no_spawn = SpawnOptions {
        program: Some("/nonexistent/jidr".into()),
        timeout: Some(Duration::from_millis(200)),
        ..Default::default()
    };
    bridge.connect_with(paths.clone(), no_spawn, move |u| {
        let _ = tx.send(u);
    });

    wait_for(&rx, "online", |u| matches!(u, Update::Conn(Conn::Online)));
    wait_for(
        &rx,
        "empty device list",
        |u| matches!(u, Update::Data(d) if d.devices.is_empty()),
    );

    // A device created by another client shows up through events.
    rt.block_on(async {
        let c = Client::connect(&paths.socket(), "test").await.unwrap();
        c.call(
            "device.create",
            json!({"name": "Pixel-ish 6.1", "image": "35"}),
        )
        .await
        .unwrap();
    });
    wait_for(
        &rx,
        "the new device",
        |u| matches!(u, Update::Data(d) if d.devices.iter().any(|i| i.device.name == "Pixel-ish 6.1")),
    );

    // The daemon goes away: the bridge notices and keeps retrying.
    rt.block_on(td.stop());
    wait_for(&rx, "reconnecting", |u| {
        matches!(u, Update::Conn(Conn::Reconnecting(_)))
    });

    // A new daemon on the same socket: back online with the same device.
    let td = rt.block_on(TestDaemon::start(tmp.path())).unwrap();
    wait_for(&rx, "online again", |u| {
        matches!(u, Update::Conn(Conn::Online))
    });
    wait_for(
        &rx,
        "the device after reconnect",
        |u| matches!(u, Update::Data(d) if d.devices.len() == 1),
    );
    rt.block_on(td.stop());
}
