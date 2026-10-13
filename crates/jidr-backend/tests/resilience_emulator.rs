//! Resilience on a real emulator: double launch refused, console commands,
//! re-adoption by a fresh backend (daemon restart), crash detection.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use jidr_backend::console::{self, GsmLine};
use jidr_backend::{AospBackend, Backend, Console, CrashKind, LaunchOptions};
use jidr_core::{Cancel, ErrorCode};

#[tokio::test(flavor = "multi_thread")]
async fn double_launch_console_adopt_crash() {
    let f = common::fixture("jidr_resilience");
    let quiet = LaunchOptions {
        audio: false,
        snapshot_save: false,
        ..Default::default()
    };
    let backend = AospBackend::new();
    let inst = backend
        .start(&f.spec, &quiet, &|_| {}, &Cancel::new())
        .await
        .unwrap();

    // Double launch refused while it runs.
    let err = backend
        .start(&f.spec, &quiet, &|_| {}, &Cancel::new())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Busy, "{err}");

    // Telnet console.
    let token = console::default_token_path();
    let mut c = Console::connect(inst.console_port, token.as_deref())
        .await
        .unwrap();
    assert_eq!(c.avd_name().await.unwrap(), "jidr_resilience");
    c.network_speed("edge").await.unwrap();
    c.network_delay("gprs").await.unwrap();
    c.gsm_signal(10, None).await.unwrap();
    c.gsm_state(GsmLine::Data, "roaming").await.unwrap();
    c.power_ac(false).await.unwrap();
    drop(c);

    // A fresh backend (daemon restart) re-adopts the running emulator.
    let restarted = AospBackend::new();
    let adopted = restarted.adopt_running(&f.paths.avd_home()).await;
    let again = adopted
        .iter()
        .find(|i| i.pid == inst.pid)
        .expect("instance re-adopted");
    assert_eq!(again.grpc_port, inst.grpc_port);
    assert_eq!(again.grpc_token, inst.grpc_token);
    assert_eq!(again.adb_serial, inst.adb_serial);
    assert!(restarted.is_alive(again).await);

    // Crash: kill -9 behind the backend's back.
    let crash_dir = f.paths.device_logs("jidr_resilience");
    let watcher = {
        let (inst, dir) = (inst.clone(), crash_dir.clone());
        tokio::spawn(async move { backend.crash_watch(&inst, &dir).await })
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    std::process::Command::new("kill")
        .args(["-9", &inst.pid.to_string()])
        .status()
        .unwrap();
    let report = tokio::time::timeout(Duration::from_secs(30), watcher)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.kind, CrashKind::Exited);
    assert_eq!(report.pid, inst.pid);
    let log = report.log.expect("crash log saved");
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("reason: emulator was killed"), "{text}");
    assert!(text.contains("--- emulator log ---"));
    println!("crash report: {} ({})", report.reason, log.display());

    // The adopted path (no exit watcher) also notices the death.
    let adopted_report = tokio::time::timeout(
        Duration::from_secs(30),
        restarted.crash_watch(again, &crash_dir),
    )
    .await
    .unwrap();
    assert_eq!(adopted_report.kind, CrashKind::Exited);

    f.avds.delete("jidr_resilience").unwrap();
}
