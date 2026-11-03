//! Phase 13d on real emulators: three agents lease clones of one lean base
//! at once (D42).
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::process::Command;
use std::time::Instant;

use common::emulator::{Emu, name};
use serde_json::{Value, json};

fn booted(info: &Value) -> bool {
    Command::new(info["adb_path"].as_str().unwrap())
        .args(["-s", info["adb_serial"].as_str().unwrap()])
        .args(["shell", "getprop", "sys.boot_completed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

/// Running emulator processes for `avd` (`-avd <avd>` in the command line).
fn emulators_of(avd: &str) -> usize {
    let out = Command::new("ps")
        .args(["-axo", "command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            let a: Vec<&str> = l.split_whitespace().collect();
            a.windows(2).any(|w| w[0] == "-avd" && w[1] == avd)
        })
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn pool_three_concurrent_clones() {
    let h = Emu::start().await;
    let mut c = h.client().await;
    // This test is about the pool; the budget has its own tests.
    c.ok("settings.set", json!({ "budget_percent": 100 })).await;
    let src = name("poolsrc");
    let base = name("poolbase");
    h.create(&mut c, &src, "agent-lean").await;
    let t = Instant::now();
    c.run_long("device.start", json!({ "device": src })).await;
    println!("source cold boot: {:.0} s", t.elapsed().as_secs_f32());
    let t = Instant::now();
    let b = c
        .run_long("base.create", json!({ "from_device": src, "name": base }))
        .await;
    println!("base.create: {:.1} s", t.elapsed().as_secs_f32());
    assert_eq!(b["base"], true);

    // Three agents, three connections, at once.
    let t = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..3 {
        let mut agent = h.client().await;
        let base = base.clone();
        tasks.push(tokio::spawn(async move {
            let r = agent
                .run_long("pool.get", json!({ "base": base, "ttl_s": 900 }))
                .await;
            (r, agent)
        }));
    }
    let mut got = Vec::new();
    for task in tasks {
        got.push(task.await.unwrap());
    }
    let wall = t.elapsed().as_secs_f32();
    println!("3 concurrent pool.get: {wall:.1} s wall");
    let mut avds = Vec::new();
    for (r, _) in &got {
        let avd = r["device"]["avd"].as_str().unwrap().to_string();
        h.guard(&avd);
        assert!(booted(&r["connect"]), "{} isn't booted", r["serial"]);
        avds.push((avd, r["lease"]["id"].clone()));
    }
    let serials: std::collections::HashSet<&str> = got
        .iter()
        .map(|(r, _)| r["serial"].as_str().unwrap())
        .collect();
    assert_eq!(serials.len(), 3, "three different devices");

    // Releasing leaves no AVD folders and no emulators behind.
    for (avd, lease) in &avds {
        c.ok("pool.release", json!({ "lease": lease })).await;
        assert!(
            !h.paths.avd_home().join(format!("{avd}.avd")).exists(),
            "{avd}.avd left behind"
        );
        assert!(!h.paths.avd_home().join(format!("{avd}.ini")).exists());
        assert_eq!(emulators_of(avd), 0, "{avd} still running");
    }
    assert_eq!(c.ok("pool.list", json!({})).await, json!([]));
    assert!(wall <= 30.0, "3 clones took {wall:.1} s (> 30 s)");
    c.ok("base.delete", json!({ "base": base })).await;
    c.ok("device.delete", json!({ "device": src })).await;
    h.stop().await;
}
