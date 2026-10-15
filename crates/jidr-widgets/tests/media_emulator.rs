//! Phase 10a: a screenshot is a PNG of the screen, and a 5-minute
//! recording passes ffprobe (h264, 300 ± 2 s, ≥ 30 × 300 × 0.9 frames).
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn media_screenshot_and_five_minute_recording() {
    let env = common::Env::boot("widgets10a").await;

    // Screenshot.
    let png = env.tmp.path().join("shot.png");
    let s = env.call("media.screenshot", json!({ "path": png })).await;
    assert_eq!(s["path"], json!(png));
    let bytes = std::fs::read(&png).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        (s["width"].as_u64(), s["height"].as_u64()),
        (Some(720), Some(1280)),
        "{s}"
    );

    // Recording encodes with AVFoundation (VideoToolbox): macOS only.
    if !cfg!(target_os = "macos") {
        env.stop().await;
        return;
    }

    // Recording at a constant 30 fps (the last frame repeats while the screen is still).
    let mp4 = env.tmp.path().join("rec.mp4");
    let started = std::time::Instant::now();
    let rec = env
        .run(
            "media.record",
            json!({ "path": mp4, "max_seconds": 300, "fps": 30 }),
        )
        .await;
    println!(
        "recorded in {:.1} s: {rec}",
        started.elapsed().as_secs_f64()
    );
    assert!(rec["frames"].as_i64().unwrap() >= 8100, "{rec}");

    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=codec_name,nb_read_frames,duration,width,height",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ])
        .arg(&mp4)
        .output()
        .expect("ffprobe on PATH");
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let v: Value = serde_json::from_slice(&probe.stdout).unwrap();
    println!("ffprobe: {v}");
    let stream = &v["streams"][0];
    assert_eq!(stream["codec_name"], "h264");
    let duration: f64 = v["format"]["duration"].as_str().unwrap().parse().unwrap();
    assert!((298.0..=302.0).contains(&duration), "duration {duration}");
    let frames: u64 = stream["nb_read_frames"].as_str().unwrap().parse().unwrap();
    assert!(frames >= 8100, "frames {frames}");

    env.stop().await;
}
