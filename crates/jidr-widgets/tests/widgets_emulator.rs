//! Phase 8a: every core widget command has an adb-observable effect on a
//! real emulator.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use serde_json::json;

/// The display rotation the window manager reports (0–3).
fn rotation(env: &common::Env) -> Option<u32> {
    env.shell("dumpsys window").lines().find_map(|l| {
        l.trim()
            .strip_prefix("mRotation=")
            .and_then(|r| r.split_whitespace().next()?.parse().ok())
    })
}

/// Telephony's current service state line (`mServiceState={…}`); the rest of
/// `dumpsys telephony.registry` holds older states.
fn service_state(registry: &str) -> &str {
    registry
        .lines()
        .find(|l| l.trim_start().starts_with("mServiceState="))
        .unwrap_or("")
}

#[test]
fn service_state_is_the_current_line() {
    let reg = "  mServiceState={mVoiceRegState=0(IN_SERVICE), mDataRegState=1(OUT_OF_SERVICE)}\n  2026 - notifyServiceStateForSubscriber: state={mVoiceRegState=3(POWER_OFF), mDataRegState=0(IN_SERVICE)}";
    assert!(service_state(reg).contains("mVoiceRegState=0("));
    assert!(!service_state(reg).contains("mDataRegState=0("));
    assert_eq!(service_state("nothing"), "");
}

/// The signal level (0–4) telephony reports: the highest level of any radio
/// in the `mSignalStrength` line. Images differ in which radio carries the
/// signal (WCDMA on API 31 arm64, LTE on API 30 x86_64), and the others report
/// level 0.
fn signal_level(registry: &str) -> Option<u32> {
    let line = registry.lines().find(|l| l.contains("mSignalStrength="))?;
    line.replace(" = ", "=")
        .split([' ', ',', '{', '}'])
        .filter_map(|t| {
            t.strip_prefix("level=")
                .or_else(|| t.strip_prefix("mLevel="))
        })
        .filter_map(|v| v.parse().ok())
        .max()
}

#[test]
fn signal_level_reads_any_radio() {
    // API 30 x86_64 after `gsm signal-profile 0` (CI run 38592875474): LTE is primary.
    let lte = "  mSignalStrength=SignalStrength:{mCdma=CellSignalStrengthCdma: cdmaDbm=2147483647 level=0,mGsm=CellSignalStrengthGsm: rssi=2147483647 ber=2147483647 mTa=2147483647 mLevel=0,mWcdma=CellSignalStrengthWcdma: ss=2147483647 level=0,mLte=CellSignalStrengthLte: rssi=2147483647 rsrp=-140 rsrq=-3 rssnr=-20 cqi=0 ta=500 level=0 parametersUseForLevel=0,mNr=CellSignalStrengthNr:{ ssRsrp = 2147483647 level = 0 parametersUseForLevel = 0 },primary=CellSignalStrengthLte}";
    assert_eq!(signal_level(lte), Some(0));
    assert_eq!(
        signal_level(&lte.replace(
            "rsrp=-140 rsrq=-3 rssnr=-20 cqi=0 ta=500 level=0",
            "rsrp=-85 level=4"
        )),
        Some(4)
    );
    assert_eq!(signal_level(&lte.replace("mLevel=0", "mLevel=3")), Some(3));
    assert_eq!(
        signal_level(&lte.replace("level = 0", "level = 2")),
        Some(2)
    );
    assert_eq!(signal_level("mServiceState=x"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn widgets_core_commands() {
    let env = common::Env::boot("widgets8a").await;

    // Battery: level and charger reach Android's battery service.
    env.call("battery.set", json!({ "level": 42, "charging": false }))
        .await;
    env.eventually("battery level 42, unplugged", |e| {
        let b = e.shell("dumpsys battery");
        b.contains("level: 42") && b.contains("AC powered: false")
    })
    .await;
    env.call("battery.set", json!({ "charging": true })).await;
    env.eventually("AC powered", |e| {
        e.shell("dumpsys battery").contains("AC powered: true")
    })
    .await;
    assert_eq!(env.call("battery.get", json!({})).await["level"], 42);

    // Rotation: the window manager follows the physical model.
    env.call("app.launch", json!({ "package": "com.android.settings" }))
        .await;
    env.call("sensor.rotate", json!({ "angle": 90 })).await;
    env.eventually("rotation 90°", |e| rotation(e) == Some(1))
        .await;
    env.call("sensor.rotate", json!({ "angle": 0 })).await;
    env.eventually("rotation 0°", |e| rotation(e) == Some(0))
        .await;

    // Motion/environment sensors read back what was set.
    env.call(
        "sensor.set",
        json!({ "sensor": "light", "values": [123.0] }),
    )
    .await;
    let light = env.call("sensor.get", json!({ "sensor": "light" })).await;
    assert!(
        (light["values"][0].as_f64().unwrap() - 123.0).abs() < 0.5,
        "{light}"
    );

    // Network: Wi-Fi/mobile toggles and a speed/delay profile.
    env.call(
        "network.set",
        json!({ "wifi": false, "mobile": false, "profile": "edge" }),
    )
    .await;
    env.eventually("Wi-Fi off", |e| {
        e.shell("settings get global wifi_on").trim() == "0"
    })
    .await;
    // Mobile data is switched at the emulated radio (console `gsm data`).
    let registry = "dumpsys telephony.registry";
    env.eventually_shell(
        "mobile data off",
        registry,
        &["mServiceState="],
        "",
        |out| !service_state(out).contains("mDataRegState=0("),
    )
    .await;
    let net = env.call("network.get", json!({})).await;
    assert_eq!(net["mobile"], false, "{net}");
    let status = env.adb(&["emu", "network", "status"]);
    assert!(status.contains("473600 bits/s"), "edge speed: {status}");
    env.call(
        "network.set",
        json!({ "wifi": true, "mobile": true, "profile": "full" }),
    )
    .await;
    env.eventually("Wi-Fi on", |e| {
        e.shell("settings get global wifi_on").trim() == "1"
    })
    .await;
    env.eventually_shell("mobile data on", registry, &["mServiceState="], "", |out| {
        service_state(out).contains("mDataRegState=0(")
    })
    .await;
    let net = env.call("network.get", json!({})).await;
    assert_eq!(net["mobile"], true, "{net}");

    // An incoming call is dropped while the radio isn't in service (on API 30
    // CI it was restarting when the call came), so wait for voice service.
    env.eventually_shell(
        "voice in service",
        registry,
        &["mServiceState="],
        "",
        |out| service_state(out).contains("mVoiceRegState=0("),
    )
    .await;

    // Phone: an incoming call rings, ending it goes idle; SMS lands in the inbox.
    env.call("phone.call", json!({ "number": "+218 91 234 5678" }))
        .await;
    env.eventually_shell(
        "ringing",
        "dumpsys telephony.registry",
        &["mCallState", "mCallIncomingNumber", "mVoiceRegState"],
        "",
        |out| out.contains("mCallState=1"),
    )
    .await;
    env.call(
        "phone.call",
        json!({ "number": "+218912345678", "action": "end" }),
    )
    .await;
    env.eventually("idle", |e| {
        e.shell("dumpsys telephony.registry")
            .contains("mCallState=0")
    })
    .await;
    env.call(
        "phone.sms",
        json!({ "from": "+218911111111", "text": "Courier is 4 min away" }),
    )
    .await;
    env.eventually("SMS in the inbox", |e| {
        e.shell("content query --uri content://sms/inbox --projection address:body")
            .contains("Courier is 4 min away")
    })
    .await;

    // Mobile signal bars (R3): more bars, stronger reported signal.
    env.call("phone.gsm", json!({ "signal": 0 })).await;
    env.eventually_shell("weak signal", registry, &["mSignalStrength"], "", |out| {
        signal_level(out).is_some_and(|l| l <= 1)
    })
    .await;
    env.call("phone.gsm", json!({ "signal": 4 })).await;
    env.eventually_shell("strong signal", registry, &["mSignalStrength"], "", |out| {
        signal_level(out).is_some_and(|l| l >= 3)
    })
    .await;

    // Android controls: Home shows the launcher, volume shows the volume panel.
    env.call("key.press", json!({ "key": "home" })).await;
    env.eventually("launcher focused", |e| {
        let w = e.shell("dumpsys window");
        w.lines()
            .any(|l| l.contains("mCurrentFocus") && l.to_lowercase().contains("launcher"))
    })
    .await;
    // The volume panel shows for a few seconds after a press and then hides,
    // and one `dumpsys window` can take that long on a CI runner. So press,
    // watch for up to 4 s, and press again if it was missed.
    let panel = |e: &common::Env| {
        e.shell("dumpsys window windows")
            .split("Window #")
            .any(|win| win.contains("VolumeDialogImpl") && win.contains("isVisible=true"))
    };
    let mut shown = false;
    'press: for _ in 0..5 {
        env.call("key.press", json!({ "key": "volume_up" })).await;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while std::time::Instant::now() < until {
            if panel(&env) {
                shown = true;
                break 'press;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    assert!(shown, "volume panel never showed after 5 volume_up presses");

    // Developer toggles.
    env.call(
        "dev.set",
        json!({ "show_taps": true, "pointer_location": true, "layout_bounds": true, "animation_scale": 0.5 }),
    )
    .await;
    assert_eq!(env.shell("settings get system show_touches").trim(), "1");
    assert_eq!(
        env.shell("settings get system pointer_location").trim(),
        "1"
    );
    assert_eq!(env.shell("getprop debug.layout").trim(), "true");
    assert_eq!(
        env.shell("settings get global window_animation_scale")
            .trim(),
        "0.5"
    );
    env.call("dev.set", json!({ "show_taps": false, "pointer_location": false, "layout_bounds": false, "animation_scale": 1.0 })).await;
    assert_eq!(env.shell("settings get system show_touches").trim(), "0");

    // Clipboard round trip.
    env.call("clipboard.set", json!({ "text": "hello from jidr" }))
        .await;
    assert_eq!(
        env.call("clipboard.get", json!({})).await["text"],
        "hello from jidr"
    );

    // Fingerprint (R8): the HAL sees the finger land and lift.
    env.adb(&["logcat", "-c"]);
    env.call("fingerprint.touch", json!({ "finger": 1 })).await;
    env.call("fingerprint.remove", json!({})).await;
    // On timeout, every fingerprint-related log line is printed: API 35 CI
    // showed no `FingerprintHal` lines (run 38631805653), so its HAL may log
    // under another tag.
    env.eventually_shell(
        "fingerprint HAL events",
        "logcat -d",
        &["inger"],
        "",
        |log| {
            log.lines()
                .any(|l| l.contains("FingerprintHal") && l.contains("off"))
        },
    )
    .await;

    // Host webcam (R7): stored in the device's launch args for the next start.
    let cam = env.call("camera.set", json!({ "back": "webcam0" })).await;
    assert_eq!(cam["back"], "webcam0");
    let det = env.call("device.details", json!({})).await;
    let args = det["device"]["launch"]["extra_args"].as_array().unwrap();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-camera-back" && w[1] == "webcam0"),
        "{args:?}"
    );

    env.stop().await;
}
