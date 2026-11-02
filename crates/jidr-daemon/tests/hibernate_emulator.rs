//! Phase 13c on a real emulator: hibernate and resume, and the quick-boot
//! save on stop (D42).
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use common::emulator::{Emu, name};
use serde_json::{Value, json};

fn adb(info: &Value, args: &[&str]) -> String {
    Command::new(info["adb_path"].as_str().unwrap())
        .args(["-s", info["adb_serial"].as_str().unwrap()])
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// The activity on top, e.g. `com.android.settings/.Settings`.
fn top_activity(info: &Value) -> String {
    let out = adb(info, &["shell", "dumpsys activity activities"]);
    out.lines()
        .find(|l| l.contains("topResumedActivity") || l.contains("mResumedActivity"))
        .and_then(|l| l.split_whitespace().find(|w| w.contains('/')))
        .unwrap_or_default()
        .trim_end_matches('}')
        .to_string()
}

fn newest_log(dir: &PathBuf) -> String {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    logs.sort();
    std::fs::read_to_string(logs.last().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn hibernate_resume_and_quick_boot() {
    let h = Emu::start().await;
    let mut c = h.client().await;
    let dev = name("hib");
    let d = h.create(&mut c, &dev, "agent-lean").await;
    let avd = d["avd"].as_str().unwrap().to_string();
    let avd_dir = h.paths.avd_home().join(format!("{avd}.avd"));
    let logs = h.paths.device_logs(&avd);
    let t = Instant::now();
    c.run_long("device.start", json!({ "device": dev })).await;
    println!("cold boot: {:.0} s", t.elapsed().as_secs_f32());

    // An app in the foreground.
    let info = c.ok("device.connect_info", json!({ "device": dev })).await;
    // Right after a first boot the launcher may still take focus back.
    let mut before = String::new();
    for _ in 0..10 {
        adb(
            &info,
            &["shell", "am start -W -n com.android.settings/.Settings"],
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
        before = top_activity(&info);
        if before.contains("settings") {
            break;
        }
    }
    println!("on top before: {before}");
    assert!(
        before.contains("settings"),
        "Settings didn't open: {before:?}"
    );

    // Hibernate, resume, same activity on top.
    let t = Instant::now();
    let d = c
        .run_long("device.hibernate", json!({ "device": dev }))
        .await;
    println!("hibernate: {:.1} s", t.elapsed().as_secs_f32());
    assert_eq!(d["status"]["state"], "hibernated");
    let ram = std::fs::metadata(avd_dir.join("snapshots/jidr-hibernate/ram.bin"))
        .unwrap()
        .len();
    println!("hibernate ram.bin: {} MB", ram / 1_000_000);
    let t = Instant::now();
    c.run_long("device.start", json!({ "device": dev })).await;
    let resumed = t.elapsed().as_secs_f32();
    println!("resume: {resumed:.1} s");
    let info = c.ok("device.connect_info", json!({ "device": dev })).await;
    let after = top_activity(&info);
    println!("on top after: {after}");
    assert_eq!(after, before, "the same activity is on top");
    assert!(resumed <= 15.0, "resume took {resumed:.1} s (> 15 s)");

    // Regression: a plain stop saves quick-boot state, and the next start
    // loads it instead of cold booting.
    let t = Instant::now();
    c.ok("device.stop", json!({ "device": dev })).await;
    println!(
        "stop with quick-boot save: {:.1} s",
        t.elapsed().as_secs_f32()
    );
    let ram = std::fs::metadata(avd_dir.join("snapshots/default_boot/ram.bin"))
        .map(|m| m.len())
        .unwrap_or(0);
    println!("default_boot ram.bin: {} MB", ram / 1_000_000);
    assert!(ram > 0, "the quick-boot save left an empty ram.bin");
    let t = Instant::now();
    c.run_long("device.start", json!({ "device": dev })).await;
    println!("quick boot: {:.1} s", t.elapsed().as_secs_f32());
    let log = newest_log(&logs);
    assert!(
        !log.contains("performing a cold boot"),
        "the start after a stop cold booted:\n{}",
        log.lines()
            .filter(|l| l.contains("napshot"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    c.ok("device.stop", json!({ "device": dev })).await;
    c.ok("device.delete", json!({ "device": dev })).await;
    h.stop().await;
}
