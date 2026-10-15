//! Phase 8b: a GPS fix shows in `dumpsys location`, and a 3-point GPX
//! replay moves the reported location through the points in order.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;

use serde_json::json;

const POINTS: [(f64, f64); 3] = [(32.8854, 13.1802), (32.8754, 13.1902), (32.8654, 13.2002)];

/// Builds the location probe app (tests/fixtures/build-hello-apk.sh).
fn build_probe(dir: &Path) -> std::path::PathBuf {
    let out = dir.join("locprobe.apk");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/build-hello-apk.sh");
    let st = std::process::Command::new("bash")
        .arg(script)
        .arg(&out)
        .arg("locprobe")
        .status()
        .unwrap();
    assert!(st.success(), "building the probe app failed");
    out
}

/// Probe fixes from logcat, oldest first.
fn fixes(env: &common::Env) -> Vec<(f64, f64)> {
    env.adb(&["logcat", "-d", "-s", "JidrLocProbe"])
        .lines()
        .filter_map(|l| {
            let (_, v) = l.split_once("JidrLocProbe: ")?;
            let (a, b) = v.trim().split_once(',')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .collect()
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 0.0015 && (a.1 - b.1).abs() < 0.0015
}

#[tokio::test(flavor = "multi_thread")]
async fn gps_fix_and_route_replay() {
    let env = common::Env::boot("widgets8b").await;
    let probe = build_probe(env.tmp.path());
    env.call(
        "file.install",
        json!({ "apk": probe, "grant_permissions": true }),
    )
    .await;
    env.adb(&["logcat", "-c"]);
    env.call("app.launch", json!({ "package": "ly.jidr.locprobe" }))
        .await;

    // Martyrs' Square, Tripoli.
    env.call(
        "gps.set",
        json!({ "latitude": 32.8954, "longitude": 13.1802, "altitude": 21 }),
    )
    .await;
    env.eventually("Tripoli in dumpsys location", |e| {
        let d = e.shell("dumpsys location");
        d.contains("32.8954") && d.contains("13.180")
    })
    .await;
    let g = env.call("gps.get", json!({})).await;
    assert!(
        (g["latitude"].as_f64().unwrap() - 32.8954).abs() < 1e-6,
        "{g}"
    );

    // Three-point route: fixes arrive near each point, in order.
    let gpx = env.tmp.path().join("route.gpx");
    let pts: String = POINTS
        .iter()
        .map(|(la, lo)| format!("<trkpt lat=\"{la}\" lon=\"{lo}\"><ele>20</ele></trkpt>"))
        .collect();
    std::fs::write(
        &gpx,
        format!("<gpx version=\"1.1\"><trk><trkseg>{pts}</trkseg></trk></gpx>"),
    )
    .unwrap();
    env.adb(&["logcat", "-c"]);
    let r = env
        .run(
            "gps.route",
            json!({ "path": gpx, "speed": 1.0, "mps": 600.0 }),
        )
        .await;
    assert!(r["total_m"].as_f64().unwrap() > 2500.0, "{r}");
    env.eventually("the last point reported", |e| {
        fixes(e).iter().any(|f| near(*f, POINTS[2]))
    })
    .await;
    let got = fixes(&env);
    let first = |p: (f64, f64), from: usize| {
        got.iter()
            .skip(from)
            .position(|f| near(*f, p))
            .map(|i| i + from)
    };
    let a = first(POINTS[0], 0).unwrap_or_else(|| panic!("point 1 missing in {got:?}"));
    let b = first(POINTS[1], a).unwrap_or_else(|| panic!("point 2 not after point 1 in {got:?}"));
    let c = first(POINTS[2], b).unwrap_or_else(|| panic!("point 3 not after point 2 in {got:?}"));
    assert!(a < b && b < c, "{got:?}");
    // Latitude only falls along this route.
    assert!(
        got[a..=c].windows(2).all(|w| w[1].0 <= w[0].0 + 1e-4),
        "{got:?}"
    );

    env.stop().await;
}
