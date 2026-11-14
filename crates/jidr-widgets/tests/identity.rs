//! `identity.*` (#19) through the rooted on-device helper (ADR-0003),
//! offline: a daemon with the fake backend, and a fake guest whose helper
//! runs the real request loop on a host port behind a fake `adb forward`.
//! The real helper on a real emulator is `identity_emulator.rs`.
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

async fn set(c: &Client, android_id: Value) -> jidr_core::JidrResult<Value> {
    c.run(
        "identity.set",
        json!({ "device": "P", "android_id": android_id }),
        |_| {},
    )
    .await
}

async fn get(c: &Client) -> Value {
    c.call("identity.get", json!({ "device": "P" }))
        .await
        .unwrap()
}

#[tokio::test]
async fn both_are_read_without_root() {
    let s = setup("none").await;
    assert_eq!(
        get(&s.c).await,
        json!({ "android_id": "1f3842600e7f0f09", "imei": "867400022047199" })
    );
    // Setting needs root, refused before anything is pushed.
    let e = set(&s.c, json!("0123456789abcdef")).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("root"), "{}", e.message);
    assert_eq!(s.guest.pushes(), 0);
    s.d.stop().await;
}

#[tokio::test]
async fn the_android_id_reaches_every_app_and_keeps_the_user_key() {
    let s = setup("adb").await;
    let list = s.c.call("commands.list", json!({})).await.unwrap();
    let spec = list
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "identity.set")
        .cloned()
        .unwrap();
    assert_eq!(spec["locksDevice"], true);
    assert_eq!(spec["longRunning"], true);

    // An SSAID table with a user key and an old entry for the app.
    s.guest.state().files.insert(
        jidr_helper::identity::SSAID_FILE.into(),
        br#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<settings version="-1">
<setting id="0" name="userkey" value="00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF" package="android" />
<setting id="1" name="10123" value="aaaaaaaaaaaaaaaa" package="com.example.app" />
</settings>"#
            .to_vec(),
    );
    let v = set(&s.c, json!("0123456789ABCDEF")).await.unwrap();
    // The global value too (written once the framework is back), and the
    // IMEI is still there.
    assert_eq!(
        v,
        json!({ "android_id": "0123456789abcdef", "imei": "867400022047199" })
    );
    let table = s.guest.ssaid().unwrap();
    assert!(
        table.contains(r#"name="userkey" value="00112233445566778899AABBCCDDEEFF"#),
        "{table}"
    );
    assert!(
        table.contains(r#"name="10123" value="0123456789abcdef" package="com.example.app""#),
        "{table}"
    );
    // Only app uids: not system (1000) or shell (2000).
    assert!(!table.contains(r#"name="1000""#) && !table.contains(r#"name="2000""#));
    // The framework was restarted (a new system_server).
    assert_eq!(s.guest.state().system_server, Some(501));
    // It lives in /data: a reboot keeps it.
    s.guest.reboot();
    assert_eq!(get(&s.c).await["android_id"], "0123456789abcdef");
    s.d.stop().await;
}

#[tokio::test]
async fn a_missing_ssaid_table_is_created() {
    let s = setup("magisk").await;
    set(&s.c, json!("00000000000000ff")).await.unwrap();
    let table = s.guest.ssaid().unwrap();
    assert!(!table.contains("userkey"), "{table}");
    assert!(
        table.contains(r#"name="10123" value="00000000000000ff""#),
        "{table}"
    );
    s.d.stop().await;
}

#[tokio::test]
async fn bad_ids_are_usage_errors_before_the_device_is_touched() {
    let s = setup("adb").await;
    for bad in [
        json!("123"),
        json!("0123456789abcdeg"),
        json!("0123456789abcdef0"),
        json!(1234),
    ] {
        let e = set(&s.c, bad.clone()).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{bad}: {e}");
    }
    let e = set(&s.c, json!("xyz")).await.unwrap_err();
    assert_eq!(e.data.unwrap()["field"], "android_id");
    assert_eq!(s.guest.pushes(), 0, "nothing reaches the device");
    assert!(s.guest.ssaid().is_none());
    s.d.stop().await;
}
