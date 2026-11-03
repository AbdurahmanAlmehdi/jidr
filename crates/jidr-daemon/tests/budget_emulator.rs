//! Phase 13a on a real emulator: the lean profile's footprint (D42).
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::emulator::{Emu, name};
use serde_json::json;

/// The lean profile boots the test image (API 35 by default) and stays under
/// 4.5 GB of host memory; the daemon measures it and keeps the peak.
#[tokio::test(flavor = "multi_thread")]
async fn budget_lean_footprint() {
    let h = Emu::start().await;
    let mut c = h.client().await;
    let lean = name("lean");
    h.create(&mut c, &lean, "agent-lean").await;
    let t = Instant::now();
    c.run_long("device.start", json!({ "device": lean })).await;
    println!("lean first boot: {:.0} s", t.elapsed().as_secs_f32());
    let mut peak = 0;
    for _ in 0..3 {
        h.daemon.state().sample_footprints().await;
        let b = c.ok("host.budget", json!({})).await;
        let mb = b["devices"][0]["measured_mb"].as_u64().unwrap_or(0);
        println!("lean footprint: {mb} MB (budget {b})");
        peak = peak.max(mb);
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    let d = c.ok("device.stop", json!({ "device": lean })).await;
    println!(
        "lean peak footprint: {peak} MB; stored {}",
        d["footprint_mb"]
    );
    assert!(peak > 0, "the footprint was measured");
    // The daemon's own 30 s sampler may have caught a higher sample.
    let stored = d["footprint_mb"].as_u64().unwrap_or(0);
    assert!(stored >= peak, "stored {stored} < sampled {peak}");
    assert!(stored <= 4608, "lean device used {stored} MB (> 4.5 GB)");
    c.ok("device.delete", json!({ "device": lean })).await;
    h.stop().await;
}
