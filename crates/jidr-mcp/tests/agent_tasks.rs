//! Five scripted agent tasks over MCP on a real emulator (in-process daemon
//! with the AOSP backend). Latency per tool is printed, not gated.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::Mcp;
use jidr_core::{ImageRef, JidrPaths};
use jidr_daemon::{Daemon, DaemonConfig};
use jidr_mcp::McpOptions;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};

fn test_sdk() -> PathBuf {
    std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Android/sdk")
        })
}

fn test_image() -> String {
    let abi = if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else {
        "x86_64"
    };
    let pkg = std::env::var("JIDR_TEST_IMAGE")
        .unwrap_or_else(|_| format!("system-images;android-31;google_apis;{abi}"));
    ImageRef::parse_package(&pkg).unwrap().package()
}

/// Center (device px) of the first node whose text or content-desc equals `label`.
fn find(xml: &str, label: &str) -> Option<(u32, u32)> {
    let attr = |node: &str, name: &str| -> Option<String> {
        let start = node.find(&format!(" {name}=\""))? + name.len() + 3;
        let end = node[start..].find('"')? + start;
        Some(node[start..end].to_string())
    };
    xml.split("<node ").skip(1).find_map(|node| {
        let matches = [attr(node, "text"), attr(node, "content-desc")]
            .iter()
            .flatten()
            .any(|v| v.eq_ignore_ascii_case(label));
        if !matches {
            return None;
        }
        let b = attr(node, "bounds")?;
        let nums: Vec<u32> = b
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        match nums.as_slice() {
            [x1, y1, x2, y2] => Some(((x1 + x2) / 2, (y1 + y2) / 2)),
            _ => None,
        }
    })
}

/// Polls ui_dump until `pred` holds.
async fn wait_ui(mcp: &Mcp, dev: &str, secs: u64, pred: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let r = mcp.call("ui_dump", json!({ "device": dev })).await;
        let xml = common::text(&r);
        if r.is_error != Some(true) && pred(&xml) {
            return xml;
        }
        if Instant::now() > deadline {
            let path = std::env::temp_dir().join("jidr-agent-task-ui.xml");
            let _ = std::fs::write(&path, &xml);
            panic!("UI never matched; last dump saved to {}", path.display());
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn five_agent_tasks() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = JidrPaths::under(tmp.path().join("j"));
    let daemon = Daemon::builder(
        DaemonConfig {
            paths: paths.clone(),
            studio_sdk: Some(test_sdk()),
            idle_timeout: None,
            sdk_repo: None,
        },
        Arc::new(jidr_backend::AospBackend::new()),
    )
    .build()
    .unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(daemon.serve(async {
        let _ = stop_rx.await;
    }));
    while !paths.socket().exists() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let human = jidr_client::Client::connect(&paths.socket(), "test")
        .await
        .unwrap();
    let mcp = Mcp::connect(
        jidr_client::Client::connect(&paths.socket(), "mcp")
            .await
            .unwrap(),
        McpOptions::default(),
    )
    .await;
    let dev = "agent-phone";

    // Task 1: create and boot a device, then look at it.
    let t = Instant::now();
    let created = mcp
        // The default 4 GB profile: on 2 GB, API 31's first-boot background work
        // gets Settings' search process killed ("app died") mid-task.
        .ok(
            "command_run",
            json!({ "command": "device.create", "params": { "name": dev, "image": test_image() } }),
        )
        .await;
    assert_eq!(created["agent_owned"], true);
    let booted = mcp
        .ok(
            "command_run",
            json!({ "command": "device.start", "params": { "device": dev } }),
        )
        .await;
    assert_eq!(booted["status"]["state"], "running");
    let shot = mcp.call("screenshot", json!({ "device": dev })).await;
    let png = shot
        .content
        .iter()
        .find_map(|c| match c {
            ContentBlock::Image(i) => Some(i.data.clone()),
            _ => None,
        })
        .expect("an image");
    let sc = shot.structured_content.clone().unwrap();
    assert!(
        sc["width"].as_u64().unwrap() <= 1024 && sc["height"].as_u64().unwrap() <= 1024,
        "{sc}"
    );
    assert!(png.len() > 1000);
    println!(
        "task 1 (create, boot, screenshot): {:.1} s",
        t.elapsed().as_secs_f64()
    );

    // Task 2: open Settings from the app drawer, the way an agent would.
    let t = Instant::now();
    mcp.ok("press_key", json!({ "device": dev, "key": "GoHome" }))
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (w, h) = (
        sc["width"].as_f64().unwrap(),
        sc["height"].as_f64().unwrap(),
    );
    let mut drawer = String::new();
    for _ in 0..4 {
        mcp.ok("swipe", json!({ "device": dev, "x1": w / 2.0, "y1": h * 0.85, "x2": w / 2.0, "y2": h * 0.2, "duration_ms": 400 })).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        drawer = common::text(&mcp.call("ui_dump", json!({ "device": dev })).await);
        if find(&drawer, "Settings").is_some() {
            break;
        }
    }
    let (x, y) = find(&drawer, "Settings")
        .unwrap_or_else(|| panic!("no Settings icon:\n{}", &drawer[..drawer.len().min(3000)]));
    mcp.ok(
        "tap",
        json!({ "device": dev, "x": x, "y": y, "coords": "device" }),
    )
    .await;
    let settings = wait_ui(&mcp, dev, 30, |xml| xml.contains("com.android.settings")).await;
    println!("task 2 (open Settings): {:.1} s", t.elapsed().as_secs_f64());

    // Task 3: type into a search field: the launcher's app search. (Settings'
    // own search lives in Settings Intelligence, which a freshly booted device
    // restarts during first-boot updates: "app died" mid-task. The launcher
    // stays up.) Like an agent: look, act, check, retry.
    let _ = settings;
    let t = Instant::now();
    let mut typed = None;
    for attempt in 1..=3 {
        mcp.ok("press_key", json!({ "device": dev, "key": "GoHome" }))
            .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        mcp.ok("swipe", json!({ "device": dev, "x1": w / 2.0, "y1": h * 0.85, "x2": w / 2.0, "y2": h * 0.2, "duration_ms": 400 })).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let xml = common::text(&mcp.call("ui_dump", json!({ "device": dev })).await);
        let field = find(&xml, "Search your phone and more").or_else(|| find(&xml, "Search apps"));
        if let Some((fx, fy)) = field {
            mcp.ok(
                "tap",
                json!({ "device": dev, "x": fx, "y": fy, "coords": "device" }),
            )
            .await;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            mcp.ok("type_text", json!({ "device": dev, "text": "clock" }))
                .await;
            for _ in 0..10 {
                let xml = common::text(&mcp.call("ui_dump", json!({ "device": dev })).await);
                // The field now holds the typed text (lowercase, unlike the "Clock" app label).
                if xml.contains("text=\"clock\"") {
                    typed = Some(xml);
                    break;
                }
                // First use of the launcher search asks "Always show keyboard
                // here?": answer it like an agent would, then type again.
                if xml.contains("Always show keyboard here?")
                    && let Some((bx, by)) = find(&xml, "Yes")
                {
                    mcp.ok(
                        "tap",
                        json!({ "device": dev, "x": bx, "y": by, "coords": "device" }),
                    )
                    .await;
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                    mcp.ok("type_text", json!({ "device": dev, "text": "clock" }))
                        .await;
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
        }
        if typed.is_some() {
            break;
        }
        println!(
            "  task 3 attempt {attempt} didn't land (field found: {})",
            field.is_some()
        );
    }
    let results = typed.expect("typing into the launcher search never showed the text");
    assert!(results.contains("Clock"), "the search lists the Clock app");
    println!(
        "task 3 (type into search): {:.1} s",
        t.elapsed().as_secs_f64()
    );

    // Task 4: read the device log.
    let t = Instant::now();
    let log = mcp.call("logcat_tail", json!({ "device": dev, "lines": 500, "filter": ["ActivityTaskManager:I", "ActivityManager:I", "*:S"] })).await;
    let log = common::text(&log);
    assert!(
        log.contains("ActivityTaskManager") || log.contains("ActivityManager"),
        "{}",
        &log[..log.len().min(2000)]
    );
    println!("task 4 (logcat): {:.1} s", t.elapsed().as_secs_f64());

    // Task 5: clean up under the D5 policy.
    let t = Instant::now();
    human
        .call(
            "device.create",
            json!({ "name": "humans", "image": test_image() }),
        )
        .await
        .unwrap();
    let e = mcp
        .err(
            "command_run",
            json!({ "command": "device.delete", "params": { "device": "humans" } }),
        )
        .await;
    assert_eq!(e["data"]["policy"], "D5");
    mcp.ok(
        "command_run",
        json!({ "command": "device.stop", "params": { "device": dev } }),
    )
    .await;
    mcp.ok(
        "command_run",
        json!({ "command": "device.delete", "params": { "device": dev } }),
    )
    .await;
    let jobs = mcp.ok("jobs_list", json!({ "all": true })).await;
    assert!(
        jobs["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|j| j["state"] == "succeeded")
    );
    let left: Value = human.call("device.list", json!({})).await.unwrap();
    assert_eq!(
        left.as_array().unwrap().len(),
        1,
        "only the human's device remains"
    );
    println!(
        "task 5 (stop, delete, policy): {:.1} s",
        t.elapsed().as_secs_f64()
    );

    mcp.report();
    let _ = stop_tx.send(());
    let _ = serving.await;
}

#[test]
fn ui_finder() {
    let xml = r#"<hierarchy><node index="0" text="" content-desc="Settings" bounds="[10,20][30,60]" /><node text="Other" bounds="[0,0][1,1]"/></hierarchy>"#;
    assert_eq!(find(xml, "settings"), Some((20, 40)));
    assert_eq!(find(xml, "Missing"), None);
}
