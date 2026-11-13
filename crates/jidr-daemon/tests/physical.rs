//! `physical.*` (#92) against a fake `adb` that knows two phones, an
//! emulator and a phone without USB permissions: listing, readiness,
//! screenshots, input, files, logcat, recording and the track-devices
//! events.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::Harness;
use serde_json::{Value, json};

const PHONES: &str = "List of devices attached\n\
R5CT1234ABC            device usb:1-1 product:shiba model:Pixel_8 device:shiba transport_id:3\n\
0A1B2C3D               unauthorized usb:2-1 transport_id:4\n\
emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
XYZ987                 no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html] usb:1-2 transport_id:5\n\n";

/// adb as a phone answers it. `track-devices` re-sends the listing each
/// time `phones.txt` changes; `screenrecord` runs until `pkill -2`.
const FAKE_ADB: &str = r#"#!/bin/bash
dir="$(dirname "$0")"
echo "$*" >> "$dir/calls.log"
serial=""
while [ "$1" = -s ]; do serial="$2"; shift 2; done
case "$1" in
  devices) cat "$dir/phones.txt";;
  track-devices)
    last=""
    while true; do
      body="$(tail -n +2 "$dir/phones.txt" | grep -v '^$')"
      if [ "$body" != "$last" ]; then
        payload="$body"$'\n'
        printf '%04x%s' "${#payload}" "$payload"
        last="$body"
      fi
      sleep 0.1
    done;;
  exec-out)
    printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x04\x38\x00\x00\x09\x60rest';;
  shell)
    case "$2" in
      getprop)
        printf '[ro.product.model]: [Pixel 8]\n[ro.product.manufacturer]: [Google]\n'
        printf '[ro.build.version.release]: [14]\n[ro.build.version.sdk]: [34]\n[ro.product.cpu.abi]: [arm64-v8a]\n';;
      "dumpsys battery") printf '  USB powered: true\n  status: 2\n  level: 87\n  scale: 100\n';;
      "wm size") echo "Physical size: 1080x2400";;
      "wm density") echo "Physical density: 420";;
      screenrecord) echo $$ > "$dir/rec.pid"; sleep 20 & wait;;
      "pkill -2 screenrecord") [ -f "$dir/rec.pid" ] && kill "$(cat "$dir/rec.pid")";;
      "exit 3") echo partial; exit 3;;
      *) echo "ran: $2";;
    esac;;
  install) echo "Performing Streamed Install"; echo Success;;
  push) echo "$2: 1 file pushed";;
  pull) echo "fake mp4" > "$3"; echo "$2: 1 file pulled";;
  logcat)
    if [ "$2" = "-d" ]; then echo "I/phone: one"; echo "I/phone: two";
    else echo "11-14 10:00:00.000  100  100 I phone   : live"; sleep 20; fi;;
  reboot) ;;
  *) echo "unknown command $1" >&2; exit 1;;
esac
"#;

fn fake_adb(h: &Harness) -> PathBuf {
    let dir = h.tmp.path().join("studio/platform-tools");
    let bin = dir.join("adb");
    std::fs::write(&bin, FAKE_ADB).unwrap();
    std::fs::write(dir.join("phones.txt"), PHONES).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn calls(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

/// Reads notifications until one with `topic` arrives.
async fn next_event(c: &mut common::Conn, topic: &str) -> Value {
    loop {
        let v = c.read().await;
        if v["method"] == "event" && v["params"]["topic"] == topic {
            return v["params"]["data"].clone();
        }
    }
}

#[tokio::test]
async fn phones_are_listed_without_emulators_and_with_their_details() {
    let h = Harness::start().await;
    fake_adb(&h);
    let mut c = h.client().await;
    let list = c.ok("physical.list", json!({})).await;
    let phones = list.as_array().unwrap();
    let serials: Vec<_> = phones
        .iter()
        .map(|p| p["serial"].as_str().unwrap())
        .collect();
    assert_eq!(serials, ["R5CT1234ABC", "0A1B2C3D", "XYZ987"], "{list}");

    let ready = &phones[0];
    assert_eq!(ready["state"], "ready");
    assert_eq!(ready["connection"], "usb");
    assert_eq!(ready["model"], "Pixel 8");
    assert_eq!(ready["manufacturer"], "Google");
    assert_eq!(ready["android"], "14");
    assert_eq!(ready["api"], 34);
    assert_eq!(ready["usb"], "1-1");
    assert_eq!(ready["battery"], json!({ "level": 87, "charging": true }));
    assert!(ready.get("hint").is_none());
    assert!(
        ready.get("screen").is_none(),
        "only details measure the screen"
    );

    let unauthorized = &phones[1];
    assert_eq!(unauthorized["state"], "unauthorized");
    assert!(unauthorized["hint"].as_str().unwrap().contains("Allow"));
    assert!(unauthorized.get("model").is_none());
    assert_eq!(phones[2]["state"], "no_permissions");

    let d = c
        .ok("physical.details", json!({ "device": "pixel 8" }))
        .await;
    assert_eq!(d["serial"], "R5CT1234ABC");
    assert_eq!(
        d["screen"],
        json!({ "width": 1080, "height": 2400, "density": 420 })
    );
    let info = c
        .ok("physical.connect_info", json!({ "device": "R5CT1234ABC" }))
        .await;
    assert_eq!(info["serial"], "R5CT1234ABC");
    assert!(
        info["adb_path"]
            .as_str()
            .unwrap()
            .ends_with("platform-tools/adb")
    );

    let e = c
        .err("physical.details", json!({ "device": "emulator-5554" }))
        .await;
    assert_eq!(
        e["data"]["code"], "not_found",
        "emulators aren't phones: {e}"
    );
    h.stop().await;
}

#[tokio::test]
async fn a_phone_that_is_not_ready_says_what_to_do() {
    let h = Harness::start().await;
    let dir = fake_adb(&h);
    let mut c = h.client().await;
    let e = c
        .err("physical.screenshot", json!({ "device": "0A1B2C3D" }))
        .await;
    assert_eq!(e["data"]["code"], "not_found", "{e}");
    assert!(
        e["message"].as_str().unwrap().contains("unauthorized"),
        "{e}"
    );
    assert!(e["message"].as_str().unwrap().contains("Allow"), "{e}");
    assert_eq!(e["data"]["details"]["state"], "unauthorized", "{e}");
    assert!(!calls(&dir).contains("-s 0A1B2C3D"), "nothing ran on it");
    h.stop().await;
}

#[tokio::test]
async fn screenshots_input_files_logcat_shell_and_reboot_go_through_adb() {
    let h = Harness::start().await;
    let dir = fake_adb(&h);
    let mut c = h.client().await;
    let d = "R5CT1234ABC";
    let out = h.tmp.path().join("shots/phone.png");
    let shot = c
        .ok("physical.screenshot", json!({ "device": d, "path": out }))
        .await;
    assert_eq!(
        (shot["width"].clone(), shot["height"].clone()),
        (json!(1080), json!(2400))
    );
    assert!(std::fs::read(&out).unwrap().starts_with(b"\x89PNG"));

    c.ok("physical.tap", json!({ "device": d, "x": 10, "y": 20 }))
        .await;
    c.ok(
        "physical.swipe",
        json!({ "device": d, "x1": 1, "y1": 2, "x2": 3, "y2": 4, "duration_ms": 250 }),
    )
    .await;
    let k = c
        .ok("physical.key", json!({ "device": d, "key": "home" }))
        .await;
    assert_eq!(k["keycode"], 3);
    c.ok("physical.key", json!({ "device": d, "key": "GoBack" }))
        .await;
    c.ok("physical.text", json!({ "device": d, "text": "hi there" }))
        .await;
    let e = c
        .err("physical.text", json!({ "device": d, "text": "مرحبا" }))
        .await;
    assert_eq!(e["data"]["code"], "usage");
    let e = c
        .err("physical.key", json!({ "device": d, "key": "F99" }))
        .await;
    assert_eq!(e["data"]["code"], "usage");

    let apk = h.tmp.path().join("app.apk");
    std::fs::write(&apk, b"apk").unwrap();
    c.ok(
        "physical.install",
        json!({ "device": d, "apk": apk, "replace": true }),
    )
    .await;
    let pushed = c
        .ok("physical.push", json!({ "device": d, "local": apk }))
        .await;
    assert_eq!(pushed["remote"], "/sdcard/Download/");
    let local = h.tmp.path().join("pulled.txt");
    c.ok(
        "physical.pull",
        json!({ "device": d, "remote": "/sdcard/Download/jidr-test/a.txt", "local": local }),
    )
    .await;
    assert!(local.exists());
    let log = c
        .ok("physical.logcat", json!({ "device": d, "lines": 50 }))
        .await;
    assert!(log["text"].as_str().unwrap().contains("I/phone: two"));
    let sh = c
        .ok(
            "physical.shell",
            json!({ "device": d, "command": "exit 3" }),
        )
        .await;
    assert_eq!(sh["exit_code"], 3);
    assert_eq!(sh["stdout"], "partial\n");
    c.ok("physical.reboot", json!({ "device": d })).await;

    let log = calls(&dir);
    for want in [
        "-s R5CT1234ABC exec-out screencap -p",
        "-s R5CT1234ABC shell input tap 10 20",
        "-s R5CT1234ABC shell input swipe 1 2 3 4 250",
        "-s R5CT1234ABC shell input keyevent 3",
        "-s R5CT1234ABC shell input keyevent 4",
        "-s R5CT1234ABC shell input text 'hi%sthere'",
        "-s R5CT1234ABC install -r",
        "-s R5CT1234ABC push",
        "-s R5CT1234ABC pull /sdcard/Download/jidr-test/a.txt",
        "-s R5CT1234ABC logcat -d -t 50",
        "-s R5CT1234ABC reboot",
    ] {
        assert!(log.contains(want), "missing {want:?} in\n{log}");
    }
    h.stop().await;
}

#[tokio::test]
async fn a_recording_runs_until_stopped_then_lands_on_this_mac() {
    let h = Harness::start().await;
    let dir = fake_adb(&h);
    let mut c = h.client().await;
    let out = h.tmp.path().join("rec/phone.mp4");
    let r = c
        .ok(
            "physical.record",
            json!({ "device": "R5CT1234ABC", "path": out, "max_seconds": 999 }),
        )
        .await;
    assert_eq!(r["pending"], true);
    // Wait for screenrecord to start, then stop it.
    for _ in 0..100 {
        if dir.join("rec.pid").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // A second recording of the same phone fails in its own job.
    let again = c
        .ok("physical.record", json!({ "device": "R5CT1234ABC" }))
        .await;
    let again = c.wait_job(&again["job"]).await;
    assert_eq!(again["state"], "failed", "{again}");
    assert_eq!(again["error"]["code"], "busy", "{again}");
    c.ok("physical.record_stop", json!({ "device": "R5CT1234ABC" }))
        .await;
    let job = c.wait_job(&r["job"]).await;
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(job["result"]["path"], json!(out));
    assert!(out.exists());
    let log = calls(&dir);
    assert!(
        log.contains("screenrecord --time-limit 180 /data/local/tmp/jidr-record-"),
        "{log}"
    );
    assert!(
        log.contains("shell rm -f /data/local/tmp/jidr-record-"),
        "{log}"
    );
    let e = c
        .err("physical.record_stop", json!({ "device": "R5CT1234ABC" }))
        .await;
    assert_eq!(e["data"]["code"], "not_found");
    h.stop().await;
}

#[tokio::test]
async fn plugging_phones_in_and_out_publishes_events() {
    let h = Harness::start().await;
    let dir = fake_adb(&h);
    let mut sub = h.client().await;
    sub.ok("events.subscribe", json!({ "topics": ["physical."] }))
        .await;
    let mut c = h.client().await;
    c.ok("physical.list", json!({})).await;
    let mut connected = std::collections::BTreeMap::new();
    while connected.len() < 3 {
        let ev = next_event(&mut sub, "physical.connected").await;
        connected.insert(
            ev["serial"].as_str().unwrap().to_string(),
            ev["state"].clone(),
        );
    }
    assert_eq!(connected["R5CT1234ABC"], "ready");
    assert_eq!(connected["0A1B2C3D"], "unauthorized");
    assert_eq!(connected["XYZ987"], "no_permissions");
    assert!(!connected.contains_key("emulator-5554"));
    // The phone accepts the RSA prompt, and the other one is unplugged.
    std::fs::write(
        dir.join("phones.txt"),
        PHONES
            .replace(
                "0A1B2C3D               unauthorized",
                "0A1B2C3D               device",
            )
            .replace("XYZ987", "#gone")
            .lines()
            .filter(|l| !l.starts_with("#gone"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let changed = next_event(&mut sub, "physical.changed").await;
    assert_eq!(changed, json!({ "serial": "0A1B2C3D", "state": "ready" }));
    let gone = next_event(&mut sub, "physical.disconnected").await;
    assert_eq!(gone, json!({ "serial": "XYZ987" }));
    h.stop().await;
}

#[tokio::test]
async fn logcat_follow_takes_a_phone_serial() {
    let h = Harness::start().await;
    fake_adb(&h);
    let mut c = h.client().await;
    let r = c
        .ok("logcat.follow", json!({ "device": "R5CT1234ABC" }))
        .await;
    assert_eq!(r["following"], "R5CT1234ABC");
    let ev = next_event(&mut c, "logcat").await;
    assert_eq!(ev["device"], "R5CT1234ABC");
    assert!(ev["line"].as_str().unwrap().ends_with("live"));
    let e = c
        .err("logcat.follow", json!({ "device": "0A1B2C3D" }))
        .await;
    assert_eq!(e["data"]["code"], "not_found");
    h.stop().await;
}

#[tokio::test]
async fn only_the_phone_shell_is_destructive() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let list = c.ok("commands.list", json!({})).await;
    let physical: Vec<&Value> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["id"].as_str().unwrap().starts_with("physical."))
        .collect();
    assert_eq!(physical.len(), 16);
    for s in physical {
        let destructive = s["destructive"] == true;
        assert_eq!(destructive, s["id"] == "physical.shell", "{s}");
        assert_eq!(s["locksDevice"], false, "phones have no Jidr lock: {s}");
    }
    h.stop().await;
}
