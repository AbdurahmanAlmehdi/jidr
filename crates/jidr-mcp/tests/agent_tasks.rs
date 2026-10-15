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

/// Reads attribute `name` of one `<node ...` fragment of a ui_dump.
fn attr(node: &str, name: &str) -> Option<String> {
    // The split leaves the first attribute without its leading space.
    let node = format!(" {node}");
    let start = node.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = node[start..].find('"')? + start;
    Some(node[start..end].to_string())
}

/// Center (device px) of the first node matching `pred`.
fn find_node(xml: &str, pred: impl Fn(&str) -> bool) -> Option<(u32, u32)> {
    xml.split("<node ").skip(1).find_map(|node| {
        if !pred(node) {
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

/// Center (device px) of the first node whose text or content-desc equals `label`.
fn find(xml: &str, label: &str) -> Option<(u32, u32)> {
    find_node(xml, |node| {
        [attr(node, "text"), attr(node, "content-desc")]
            .iter()
            .flatten()
            .any(|v| v.eq_ignore_ascii_case(label))
    })
}

/// The app drawer's search field. Its label differs by image (Pixel
/// launcher on arm64, Launcher3 on the x86_64 CI images), so an agent
/// would fall back to "the text field" or "the thing with search in its id".
fn find_search_field(xml: &str) -> Option<(u32, u32)> {
    find(xml, "Search your phone and more")
        .or_else(|| find(xml, "Search apps"))
        .or_else(|| {
            find_node(xml, |node| {
                attr(node, "class").is_some_and(|c| c.ends_with("EditText"))
            })
        })
        .or_else(|| {
            find_node(xml, |node| {
                attr(node, "resource-id").is_some_and(|id| id.contains("search"))
            })
        })
}

/// Whether a text field has keyboard focus (the tap on the search field took).
fn text_field_focused(xml: &str) -> bool {
    xml.split("<node ").skip(1).any(|node| {
        attr(node, "class").is_some_and(|c| c.ends_with("EditText"))
            && attr(node, "focused").as_deref() == Some("true")
    })
}

/// The "Wait" button of an "… isn't responding" dialog, if one is up. On a
/// slow first boot the launcher can hang for a while (API 30 CI); an agent
/// taps Wait and carries on.
fn anr_wait(xml: &str) -> Option<(u32, u32)> {
    if xml.contains("isn't responding") {
        find(xml, "Wait")
    } else {
        None
    }
}

/// Taps Wait on an "isn't responding" dialog. True if there was one.
async fn dismiss_anr(mcp: &Mcp, dev: &str, xml: &str) -> bool {
    let Some((x, y)) = anr_wait(xml) else {
        return false;
    };
    mcp.ok(
        "tap",
        json!({ "device": dev, "x": x, "y": y, "coords": "device" }),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    true
}

/// The start of a ui_dump, for a failure message.
fn excerpt(xml: &str) -> String {
    xml.chars().take(4000).collect()
}

/// Polls ui_dump for up to `secs` until `pred` holds, answering any "isn't
/// responding" dialog on the way. Err carries the last dump.
async fn poll_ui(
    mcp: &Mcp,
    dev: &str,
    secs: u64,
    pred: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let r = mcp.call("ui_dump", json!({ "device": dev })).await;
        let xml = common::text(&r);
        if r.is_error != Some(true) && pred(&xml) {
            return Ok(xml);
        }
        if Instant::now() > deadline {
            return Err(xml);
        }
        dismiss_anr(mcp, dev, &xml).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
}

/// Goes home and swipes up until the app drawer shows `label` (one swipe
/// doesn't always open it). Returns the last dump.
async fn open_drawer(mcp: &Mcp, dev: &str, (w, h): (f64, f64), label: &str) -> String {
    mcp.ok("press_key", json!({ "device": dev, "key": "GoHome" }))
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut drawer = String::new();
    for _ in 0..4 {
        mcp.ok("swipe", json!({ "device": dev, "x1": w / 2.0, "y1": h * 0.85, "x2": w / 2.0, "y2": h * 0.2, "duration_ms": 400 })).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        drawer = common::text(&mcp.call("ui_dump", json!({ "device": dev })).await);
        if find(&drawer, label).is_some() {
            break;
        }
        dismiss_anr(mcp, dev, &drawer).await;
    }
    drawer
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
    // A per-process device name, a guard that kills its emulator however
    // the test ends, and cleanup of orphans from earlier killed runs.
    jidr_backend::testing::kill_orphans("agent-phone-");
    let dev_name = jidr_backend::testing::unique_avd("agent-phone");
    let dev = dev_name.as_str();
    let _guard = jidr_backend::testing::AvdGuard::new(dev);

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

    // Task 2: open Settings from the app drawer, the way an agent would:
    // tap the icon, check that Settings came up, and try again if it didn't
    // (API 33 CI: one tap left Settings unopened for 30 s).
    let t = Instant::now();
    let size = (
        sc["width"].as_f64().unwrap(),
        sc["height"].as_f64().unwrap(),
    );
    let (w, h) = size;
    let mut settings = None;
    for attempt in 1..=3 {
        let drawer = open_drawer(&mcp, dev, size, "Settings").await;
        let (x, y) = find(&drawer, "Settings")
            .unwrap_or_else(|| panic!("no Settings icon:\n{}", excerpt(&drawer)));
        mcp.ok(
            "tap",
            json!({ "device": dev, "x": x, "y": y, "coords": "device" }),
        )
        .await;
        match poll_ui(&mcp, dev, 20, |xml| xml.contains("com.android.settings")).await {
            Ok(xml) => {
                settings = Some(xml);
                break;
            }
            Err(xml) => println!(
                "  task 2 attempt {attempt}: Settings didn't open; the screen:\n{}",
                excerpt(&xml)
            ),
        }
    }
    let settings = settings.expect("Settings never opened (screens above)");
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
        // As in task 2, one swipe doesn't always open the drawer.
        let mut xml = String::new();
        let mut field = None;
        let mut last = String::new();
        for _ in 0..3 {
            mcp.ok("swipe", json!({ "device": dev, "x1": w / 2.0, "y1": h * 0.85, "x2": w / 2.0, "y2": h * 0.2, "duration_ms": 400 })).await;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            xml = common::text(&mcp.call("ui_dump", json!({ "device": dev })).await);
            field = find_search_field(&xml);
            if field.is_some() {
                break;
            }
            dismiss_anr(&mcp, dev, &xml).await;
        }
        if let Some((fx, fy)) = field {
            // Tap the field and wait for it to take focus before typing; a
            // tap on a busy first-boot launcher can be dropped (API 33 CI),
            // so an agent taps again.
            for _ in 0..2 {
                mcp.ok(
                    "tap",
                    json!({ "device": dev, "x": fx, "y": fy, "coords": "device" }),
                )
                .await;
                let focused = poll_ui(&mcp, dev, 6, text_field_focused).await;
                if let Err(x) = &focused {
                    last = x.clone();
                } else {
                    break;
                }
            }
            mcp.ok("type_text", json!({ "device": dev, "text": "clock" }))
                .await;
            for _ in 0..10 {
                let xml = common::text(&mcp.call("ui_dump", json!({ "device": dev })).await);
                last = xml.clone();
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
        if field.is_none() {
            println!("  drawer dump:\n{}", excerpt(&xml));
        } else {
            println!("  screen after typing:\n{}", excerpt(&last));
        }
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

/// Regression: on the x86_64 CI images (Launcher3) the drawer search field
/// has neither Pixel label, so task 3 never found it.
#[test]
fn search_field_falls_back_to_the_text_field() {
    let pixel = r#"<node text="Search your phone and more" class="android.widget.TextView" bounds="[0,100][200,140]" />"#;
    assert_eq!(find_search_field(pixel), Some((100, 120)));
    let launcher3 = r#"<node text="" class="android.widget.FrameLayout" bounds="[0,0][400,800]" /><node text="" resource-id="com.android.launcher3:id/search_box_input" class="com.android.launcher3.ExtendedEditText" bounds="[20,40][380,80]" />"#;
    assert_eq!(find_search_field(launcher3), Some((200, 60)));
    let by_id = r#"<node text="" resource-id="com.example:id/search_container" class="android.view.View" bounds="[0,0][10,10]" />"#;
    assert_eq!(find_search_field(by_id), Some((5, 5)));
    assert_eq!(
        find_search_field(r#"<node text="Clock" bounds="[0,0][2,2]" />"#),
        None
    );
}

/// Regression: on API 30 CI the launcher hung after boot and its "isn't
/// responding" dialog hid the app drawer, so task 2 found no Settings icon.
#[test]
fn anr_dialog_is_answered_with_wait() {
    let anr = r#"<node text="Pixel Launcher isn't responding" bounds="[0,0][400,60]" /><node text="Close app" bounds="[0,100][400,140]" /><node text="Wait" bounds="[0,160][400,200]" />"#;
    assert_eq!(anr_wait(anr), Some((200, 180)));
    assert_eq!(
        anr_wait(r#"<node text="Wait" bounds="[0,0][2,2]" />"#),
        None
    );
}

/// Regression: on API 33 CI task 3 found the search field but the typed
/// text never showed; it now waits for the field to take focus.
#[test]
fn a_focused_text_field_is_seen() {
    let focused = r#"<node class="android.widget.EditText" focused="true" bounds="[0,0][2,2]" />"#;
    assert!(text_field_focused(focused));
    assert!(!text_field_focused(
        &focused.replace("\"true\"", "\"false\"")
    ));
    assert!(!text_field_focused(
        r#"<node class="android.widget.TextView" focused="true" />"#
    ));
}
