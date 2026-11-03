//! Player touches on a real emulator (#51): a tablet turned through all four
//! rotations, a Settings row touched the way the player touches it, and
//! checked by what the tap opened.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jidr_backend::AospBackend;
use jidr_client::Client;
use jidr_core::{ConnectInfo, JidrPaths};
use jidr_daemon::{Daemon, DaemonConfig};
use jidr_proto::control as pb;
use jidr_ui::player::input::{Gesture, Screen, panel_touches, touch_event};
use jidr_ui::player::session::{connect, oriented};
use serde_json::json;
use tokio_stream::wrappers::UnboundedReceiverStream;

fn test_sdk() -> PathBuf {
    std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap().join("Library/Android/sdk"))
}

fn test_image() -> String {
    std::env::var("JIDR_TEST_IMAGE").unwrap_or_else(|_| {
        let abi = if cfg!(target_arch = "aarch64") {
            "arm64-v8a"
        } else {
            "x86_64"
        };
        format!("system-images;android-31;google_apis;{abi}")
    })
}

/// Kills the emulator however the test ends.
struct Kill(ConnectInfo);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.0.adb_path)
            .args(["-s", &self.0.adb_serial, "emu", "kill"])
            .output();
    }
}

fn shell(info: &ConnectInfo, cmd: &str) -> String {
    let out = std::process::Command::new(&info.adb_path)
        .args(["-s", &info.adb_serial, "shell", cmd])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The panel size from `wm size` ("Physical size: 1600x2560").
fn panel(info: &ConnectInfo) -> Screen {
    let out = shell(info, "wm size");
    let size = out
        .lines()
        .find_map(|l| l.strip_prefix("Physical size: "))
        .unwrap_or_else(|| panic!("wm size: {out}"));
    let (w, h) = size.trim().split_once('x').unwrap();
    Screen {
        width: w.parse().unwrap(),
        height: h.parse().unwrap(),
    }
}

/// The UI hierarchy (`uiautomator dump`), empty if it failed.
fn ui_dump(info: &ConnectInfo) -> String {
    shell(
        info,
        "uiautomator dump /sdcard/jidr_ui.xml >/dev/null && cat /sdcard/jidr_ui.xml",
    )
}

/// Every non-empty text and its bounds in a hierarchy, for failure messages.
fn texts(xml: &str) -> Vec<String> {
    xml.split("<node")
        .filter_map(|n| {
            let t = n.split("text=\"").nth(1)?.split('"').next()?;
            let b = n.split("bounds=\"").nth(1)?.split('"').next()?;
            (!t.is_empty()).then(|| format!("{t} {b}"))
        })
        .collect()
}

/// Bounds (left, top, right, bottom) in display pixels of the node whose
/// text is exactly `text`.
fn bounds_of(xml: &str, text: &str) -> Option<(f32, f32, f32, f32)> {
    let at = xml.find(&format!("text=\"{text}\""))?;
    let start = xml[..at].rfind("<node")?;
    let end = at + xml[at..].find('>')?;
    let b = xml[start..end]
        .split("bounds=\"[")
        .nth(1)?
        .split('"')
        .next()?;
    let n: Vec<f32> = b
        .split([',', '[', ']'])
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().ok())
        .collect::<Option<_>>()?;
    (n.len() == 4).then(|| (n[0], n[1], n[2], n[3]))
}

async fn eventually<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The next streamed frame: (width, height, quarter turns).
async fn frame(ctl: &mut jidr_proto::Controller) -> (u32, u32, u32) {
    let fmt = pb::ImageFormat {
        format: pb::image_format::ImgFormat::Rgba8888 as i32,
        width: 400,
        height: 400,
        ..Default::default()
    };
    let mut s = ctl.stream_screenshot(fmt).await.unwrap().into_inner();
    loop {
        let img = s.message().await.unwrap().unwrap();
        if let Some(f) = img.format
            && !img.image.is_empty()
        {
            let q = f.rotation.map_or(0, |r| r.rotation.rem_euclid(4) as u32);
            return (f.width, f.height, q);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn taps_land_under_the_pointer_in_every_rotation() {
    let tmp = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    // Short root: the daemon socket path must fit in sun_path.
    let paths = JidrPaths::under(tmp.path().join("j"));
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("repository2-3.xml"),
        "<sdk:sdk-repository xmlns:sdk=\"x\"/>",
    )
    .unwrap();
    let daemon = Daemon::builder(
        DaemonConfig {
            paths: paths.clone(),
            studio_sdk: Some(test_sdk()),
            idle_timeout: None,
            sdk_repo: Some(format!("file://{}/", repo.display())),
        },
        Arc::new(AospBackend::new()),
    )
    .build()
    .unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let d = daemon.clone();
    let serve = tokio::spawn(async move {
        d.serve(async {
            let _ = stopped.await;
        })
        .await
    });
    for _ in 0..500 {
        if paths.socket().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let client = Client::connect(&paths.socket(), "test").await.unwrap();
    client
        .call(
            "device.create",
            json!({ "name": "Tilt", "image": test_image(), "profile": "tablet-11", "root_mode": "none" }),
        )
        .await
        .unwrap();
    client
        .run(
            "device.start",
            json!({ "device": "Tilt", "cold_boot": true }),
            |_| {},
        )
        .await
        .unwrap();
    let info: ConnectInfo = client
        .call_as("device.connect_info", json!({ "device": "Tilt" }))
        .await
        .unwrap();
    let _kill = Kill(info.clone());
    let panel = panel(&info);
    shell(&info, "settings put system accelerometer_rotation 1");

    // The player's input path: one long-lived streamInputEvent.
    let mut ctl = connect(&info).await.unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<pb::InputEvent>();
    let mut input = ctl.clone();
    tokio::spawn(async move {
        let _ = input
            .stream_input_event(UnboundedReceiverStream::new(rx))
            .await;
    });

    for q in [1u32, 2, 3, 0] {
        ctl.set_physical_model(pb::PhysicalModelValue {
            target: pb::physical_model_value::PhysicalType::Rotation as i32,
            value: Some(pb::ParameterValue {
                data: vec![0.0, 0.0, (90 * q) as f32],
            }),
            ..Default::default()
        })
        .await
        .unwrap();
        eventually(&format!("display rotation {q}"), || {
            shell(&info, "dumpsys input")
                .lines()
                .any(|l| {
                    l.contains("displayId=0,")
                        && l.contains(&format!("orientation={q},"))
                        && l.contains("isActive=[1]")
                })
                .then_some(())
        })
        .await;
        let deadline = Instant::now() + Duration::from_secs(30);
        let (w, h, quarter) = loop {
            let f = frame(&mut ctl).await;
            if f.2 == q || Instant::now() > deadline {
                break f;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        assert_eq!(quarter, q, "frame rotation");
        let shown = oriented(panel, (w, h));
        // A human tap: Settings' "Connected devices" row (on screen in every
        // orientation), found by its bounds and touched at its centre
        // through the player's mapping, must open that page.
        shell(
            &info,
            "am start -W -f 0x10008000 -a android.settings.SETTINGS",
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        let row = loop {
            let last = ui_dump(&info);
            // Laid out for this rotation, and not already on that page.
            let found = bounds_of(&last, "Connected devices").filter(|b| {
                b.2 <= shown.width as f32
                    && b.3 <= shown.height as f32
                    && !last.contains("text=\"Pair new device\"")
            });
            if let Some(b) = found {
                break b;
            }
            assert!(
                Instant::now() < deadline,
                "no Connected devices row in Settings turned {q} quarter turns ({}×{}); on screen: {:?}",
                shown.width,
                shown.height,
                texts(&last)
            );
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let n = (
            (row.0 + row.2) / 2.0 / shown.width as f32,
            (row.1 + row.3) / 2.0 / shown.height as f32,
        );
        let pts = panel_touches(Gesture::Single, panel, shown, quarter, n);
        tx.send(touch_event(&pts, true)).unwrap();
        tx.send(touch_event(&pts, false)).unwrap();
        eventually(
            &format!(
                "the Connected devices page after a tap at {n:?} of a {}×{} screen turned {q} quarter turns (panel {pts:?})",
                shown.width, shown.height
            ),
            || ui_dump(&info).contains("text=\"Pair new device\"").then_some(()),
        )
        .await;
    }

    let _ = client
        .call("device.stop", json!({ "device": "Tilt" }))
        .await;
    let _ = stop.send(());
    let _ = serve.await;
}
