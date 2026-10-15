//! Dev-only measurement for the Phase 7c player check. With
//! `JIDR_PLAYER_BENCH=<report file>` the player opens Settings and drives
//! 30 s of scrolling through its own input path (the same channel the
//! mouse uses), then writes fps and touch→frame numbers. With
//! `JIDR_PLAYER_DROP=<file>` it then handles that file as if dropped.

use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::json;
use slint::ComponentHandle;

use super::input::{self, Screen};
use super::session::{lock, percentile};
use crate::app::Ctx;

/// Seconds of scrolling (default 30, `JIDR_PLAYER_BENCH_SECS`).
fn seconds() -> u64 {
    std::env::var("JIDR_PLAYER_BENCH_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
}

fn cpu_seconds(pid: u32) -> f64 {
    std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .split(':')
                .fold(0.0, |acc, part| {
                    acc * 60.0 + part.parse::<f64>().unwrap_or(0.0)
                })
        })
        .unwrap_or(0.0)
}

pub fn start(ctx: &Rc<Ctx>) {
    let report_path = std::env::var_os("JIDR_PLAYER_BENCH");
    let drop_path = std::env::var_os("JIDR_PLAYER_DROP");
    let Some((tx, stats, native, device)) = ctx
        .player
        .borrow()
        .session
        .as_ref()
        .map(|s| (s.input.clone(), s.stats.clone(), s.native, s.device))
    else {
        return;
    };
    let bridge = ctx.bridge.clone();
    let scale = ctx.app.window().scale_factor();
    let drop_token = crate::app::defer(move |path: std::path::PathBuf| {
        if let Some(c) = crate::app::ctx() {
            super::handle_drop(&c, &path);
        }
    });
    ctx.bridge.handle().spawn(async move {
        // Wait for the first frame.
        for _ in 0..100 {
            if stats.received.load(Ordering::Relaxed) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if let Some(out) = report_path {
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            bridge.call(
                "app.launch",
                json!({ "device": device, "package": "com.android.settings" }),
                move |r| {
                    let _ = done_tx.send(r);
                },
            );
            let _ = done_rx.await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            lock(&stats.arrivals).clear();
            lock(&stats.latencies).clear();
            lock(&stats.ages).clear();
            let (r0, s0, b0) = (
                stats.received.load(Ordering::Relaxed),
                stats.shown.load(Ordering::Relaxed),
                stats.bytes.load(Ordering::Relaxed),
            );
            let pid = std::process::id();
            let cpu0 = cpu_seconds(pid);
            let screen = Screen {
                width: native.width,
                height: native.height,
            };
            let secs = seconds();
            let t0 = Instant::now();
            let mut up = true;
            let mut intervals_ms: Vec<f32> = Vec::new();
            while t0.elapsed() < Duration::from_secs(secs) {
                let (y0, y1) = if up { (0.78, 0.30) } else { (0.30, 0.78) };
                up = !up;
                let steps = 30;
                for i in 0..=steps {
                    let y = y0 + (y1 - y0) * i as f32 / steps as f32;
                    let p = screen.to_px(0.5, y);
                    if i == 1 {
                        let mut t = lock(&stats.touch_sent);
                        if t.is_none() {
                            *t = Some(Instant::now());
                        }
                    }
                    let _ = tx.send(input::touch_event(&[(1, p.0, p.1)], true));
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let p = screen.to_px(0.5, y1);
                let _ = tx.send(input::touch_event(&[(1, p.0, p.1)], false));
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            let dt = t0.elapsed().as_secs_f64();
            {
                let q = lock(&stats.arrivals);
                intervals_ms.extend(
                    q.iter()
                        .zip(q.iter().skip(1))
                        .map(|(a, b)| b.duration_since(*a).as_secs_f32() * 1000.0)
                        .filter(|ms| *ms < 100.0),
                );
            }
            let received = stats.received.load(Ordering::Relaxed) - r0;
            let shown = stats.shown.load(Ordering::Relaxed) - s0;
            let mb = (stats.bytes.load(Ordering::Relaxed) - b0) as f64 / 1e6;
            let cpu = (cpu_seconds(pid) - cpu0) / dt * 100.0;
            let mut lat: Vec<f32> = lock(&stats.latencies).iter().copied().collect();
            let mut ages: Vec<f32> = lock(&stats.ages).iter().copied().collect();
            let frame = *lock(&stats.frame);
            let med = percentile(&mut intervals_ms.clone(), 0.5)
                .map(|m| 1000.0 / m)
                .unwrap_or(0.0);
            let p10 = percentile(&mut intervals_ms, 0.9)
                .map(|m| 1000.0 / m)
                .unwrap_or(0.0);
            let text = format!(
                "player bench: {secs} s of scrolling Settings through the player's input path\n\
                 window scale {scale:.1}, frames {}×{} RGBA8888\n\
                 active fps: median {med:.1}, p10 {p10:.1}\n\
                 received {:.1} fps, shown {:.1} fps, {:.0} MB/s\n\
                 frame age (emulator ts → shown): median {:.1} ms, p90 {:.1} ms\n\
                 touch → next frame: median {:.1} ms, p90 {:.1} ms ({} drags)\n\
                 player CPU {cpu:.0}%\n",
                frame.0,
                frame.1,
                received as f64 / dt,
                shown as f64 / dt,
                mb / dt,
                percentile(&mut ages.clone(), 0.5).unwrap_or(f32::NAN),
                percentile(&mut ages, 0.9).unwrap_or(f32::NAN),
                percentile(&mut lat.clone(), 0.5).unwrap_or(f32::NAN),
                percentile(&mut lat, 0.9).unwrap_or(f32::NAN),
                lock(&stats.latencies).len(),
            );
            eprint!("{text}");
            let _ = std::fs::write(&out, text);
        }
        if let Some(p) = drop_path {
            tokio::time::sleep(Duration::from_secs(1)).await;
            crate::app::fire(drop_token, std::path::PathBuf::from(p));
        }
    });
}
