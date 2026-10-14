//! Screenshots (gRPC `getScreenshot` → PNG) and screen recording: the
//! daemon streams frames from the emulator and encodes MP4/H.264 at a
//! constant frame rate (D13; no 180 s cap; audio post-MVP).

#[cfg(target_os = "macos")]
pub mod mp4;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant;

use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{JidrError, JidrResult};
use jidr_proto::control as pb;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::device::{Target, grpc_err, target};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShotParams {
    pub device: String,
    /// Where to save the PNG (default `~/Pictures/Jidr/<device> <time>.png`).
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordParams {
    pub device: String,
    /// Where to save the MP4 (default `~/Movies/Jidr/<device> <time>.mp4`).
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Stop on its own after this many seconds (default 3600).
    #[serde(default)]
    pub max_seconds: Option<u64>,
    /// Frames per second (default 30).
    #[serde(default)]
    pub fps: Option<u32>,
}

#[derive(Debug, Default)]
pub struct Recordings(Mutex<HashMap<Uuid, Arc<AtomicBool>>>);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<ShotParams>("media.screenshot", "Save a PNG of the screen"),
        CommandSpec::new::<RecordParams>(
            "media.record",
            "Record the screen to MP4 (H.264) until media.record_stop or max_seconds",
        )
        .long_running(),
        CommandSpec::new::<DeviceParams>(
            "media.record_stop",
            "Stop the screen recording and save it",
        ),
    ]
}

/// `<dir>/<name> 2026-10-13 at 14.02.07.<ext>`, made unique.
pub fn default_path(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d at %H.%M.%S");
    let safe: String = name
        .chars()
        .map(|c| if c == '/' || c == ':' { '-' } else { c })
        .collect();
    let mut p = dir.join(format!("{safe} {stamp}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{safe} {stamp} {n}.{ext}"));
        n += 1;
    }
    p
}

fn home_dir(sub: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(sub)
        .join("Jidr")
}

/// Recording size: the device's, long side capped at 1280, even sides.
pub fn record_size(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h).max(1);
    let k = (1280.0 / f64::from(long)).min(1.0);
    let (w, h) = ((f64::from(w) * k) as u32, (f64::from(h) * k) as u32);
    (w & !1, h & !1)
}

#[cfg(target_os = "macos")]
async fn lcd_size(t: &Target) -> JidrResult<(u32, u32)> {
    let st = t
        .grpc()
        .await?
        .get_status(())
        .await
        .map_err(grpc_err("getStatus"))?
        .into_inner();
    // Newer SDK protos mark `hardwareConfig` deprecated; the build uses the
    // installed SDK's protos when there is one, so allow it either way.
    #[allow(deprecated)]
    let get = |k: &str| {
        st.hardware_config
            .as_ref()
            .and_then(|c| c.entry.iter().find(|e| e.key == k))
            .and_then(|e| e.value.parse::<u32>().ok())
    };
    match (get("hw.lcd.width"), get("hw.lcd.height")) {
        (Some(w), Some(h)) => Ok((w, h)),
        _ => Err(JidrError::sdk("the emulator didn't report its screen size")),
    }
}

pub async fn call(
    ctx: &CallCtx,
    recs: &Recordings,
    method: &str,
    params: Value,
) -> JidrResult<Value> {
    match method {
        "media.screenshot" => {
            let p: ShotParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let img = t
                .grpc()
                .await?
                .get_screenshot(pb::ImageFormat {
                    format: pb::image_format::ImgFormat::Png as i32,
                    ..Default::default()
                })
                .await
                .map_err(grpc_err("getScreenshot"))?
                .into_inner();
            let path = p
                .path
                .unwrap_or_else(|| default_path(&home_dir("Pictures"), &t.device.name, "png"));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    JidrError::usage(format!("cannot create {}: {e}", dir.display()))
                })?;
            }
            std::fs::write(&path, &img.image)
                .map_err(|e| JidrError::usage(format!("cannot write {}: {e}", path.display())))?;
            let (w, h) = img.format.map(|f| (f.width, f.height)).unwrap_or((0, 0));
            Ok(json!({ "path": path, "width": w, "height": h }))
        }
        "media.record" => {
            let p: RecordParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let fps = p.fps.unwrap_or(30).clamp(1, 60);
            let max = Duration::from_secs(p.max_seconds.unwrap_or(3600).max(1));
            let path = p
                .path
                .unwrap_or_else(|| default_path(&home_dir("Movies"), &t.device.name, "mp4"));
            if path.exists() {
                return Err(JidrError::usage(format!(
                    "{} already exists",
                    path.display()
                )));
            }
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    JidrError::usage(format!("cannot create {}: {e}", dir.display()))
                })?;
            }
            let stop = Arc::new(AtomicBool::new(false));
            if let Ok(mut m) = recs.0.lock() {
                if m.contains_key(&t.device.id) {
                    return Err(JidrError::busy(format!(
                        "{} is already recording",
                        t.device.name
                    )));
                }
                m.insert(t.device.id, stop.clone());
            }
            let r = record(ctx, &t, &path, fps, max, stop).await;
            if let Ok(mut m) = recs.0.lock() {
                m.remove(&t.device.id);
            }
            r
        }
        "media.record_stop" => {
            let p: DeviceParams = parse_params(params)?;
            let d = ctx.daemon.device(&p.device).await?;
            let flag = recs.0.lock().ok().and_then(|m| m.get(&d.id).cloned());
            match flag {
                Some(f) => {
                    f.store(true, Ordering::Relaxed);
                    Ok(json!({ "stopping": true }))
                }
                None => Err(JidrError::not_found(format!("{} isn't recording", d.name))),
            }
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

/// The latest frame from the stream.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct Latest {
    frame: Option<(u32, u32, Vec<u8>)>,
}

#[cfg(target_os = "macos")]
async fn record(
    ctx: &CallCtx,
    t: &Target,
    path: &Path,
    fps: u32,
    max: Duration,
    stop: Arc<AtomicBool>,
) -> JidrResult<Value> {
    let (lw, lh) = lcd_size(t).await?;
    let (w, h) = record_size(lw, lh);
    let latest: Arc<Mutex<Latest>> = Arc::default();
    // Frame stream (emulator → latest slot).
    let mut grpc = t.grpc().await?;
    let mut stream = grpc
        .stream_screenshot(pb::ImageFormat {
            format: pb::image_format::ImgFormat::Rgba8888 as i32,
            width: w,
            height: h,
            ..Default::default()
        })
        .await
        .map_err(grpc_err("streamScreenshot"))?
        .into_inner();
    let slot = latest.clone();
    let reader = tokio::spawn(async move {
        while let Ok(Some(img)) = stream.message().await {
            if let (Some(f), false) = (img.format, img.image.is_empty())
                && let Ok(mut l) = slot.lock()
            {
                l.frame = Some((f.width, f.height, img.image));
            }
        }
    });
    // Encoder on its own thread at a constant rate; duplicates the last
    // frame when the screen doesn't change.
    let cancel = ctx.cancel.clone();
    let progress = ctx.progress.clone();
    let path_buf = path.to_path_buf();
    let enc = tokio::task::spawn_blocking(move || -> JidrResult<(i64, bool)> {
        let mut writer: Option<mp4::Mp4Writer> = None;
        let start = Instant::now();
        let mut last_report = Instant::now();
        let mut cancelled = false;
        let frame_dt = Duration::from_secs_f64(1.0 / f64::from(fps));
        let mut current: Option<Vec<u8>> = None;
        loop {
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            let elapsed = start.elapsed();
            if stop.load(Ordering::Relaxed) || elapsed >= max {
                break;
            }
            if let Ok(mut l) = latest.lock()
                && let Some((fw, fh, px)) = l.frame.take()
            {
                if writer.is_none() {
                    writer = Some(mp4::Mp4Writer::create(&path_buf, fw, fh, fps as i32)?);
                }
                if let Some(wr) = writer.as_ref() {
                    // Frames of another size (rotation) are cropped/padded to the first.
                    current = Some(fit(&px, fw, fh, wr.size()));
                }
            }
            if let (Some(wr), Some(px)) = (writer.as_mut(), current.as_ref()) {
                // Catch up to the wall clock with the latest frame.
                let due = (start.elapsed().as_secs_f64() * f64::from(fps)) as i64;
                while wr.frames() <= due {
                    wr.push(px)?;
                }
            }
            if last_report.elapsed() >= Duration::from_secs(1) {
                last_report = Instant::now();
                let s = start.elapsed().as_secs();
                progress(
                    Some((elapsed.as_secs_f64() / max.as_secs_f64()) as f32),
                    &format!("Recording {}:{:02}", s / 60, s % 60),
                );
            }
            std::thread::sleep(frame_dt / 2);
        }
        let frames = writer.as_ref().map(mp4::Mp4Writer::frames).unwrap_or(0);
        match writer {
            Some(w) if cancelled => w.cancel(),
            Some(w) => w.finish()?,
            None => {}
        }
        Ok((frames, cancelled))
    });
    let r = enc
        .await
        .map_err(|e| JidrError::unexpected(format!("encoder: {e}")))?;
    reader.abort();
    let (frames, cancelled) = r?;
    if cancelled {
        let _ = std::fs::remove_file(path);
        return Err(jidr_core::cancel::cancelled());
    }
    if frames == 0 {
        return Err(JidrError::sdk("no frames arrived from the emulator"));
    }
    Ok(json!({
        "path": path,
        "frames": frames,
        "fps": fps,
        "duration_s": frames as f64 / f64::from(fps),
        "width": w,
        "height": h,
    }))
}

#[cfg(not(target_os = "macos"))]
async fn record(
    _ctx: &CallCtx,
    _t: &Target,
    _path: &Path,
    _fps: u32,
    _max: Duration,
    _stop: Arc<AtomicBool>,
) -> JidrResult<Value> {
    Err(JidrError::sdk(
        "screen recording uses VideoToolbox and is macOS-only for now",
    ))
}

/// Copies an RGBA frame into a `size` canvas (top-left aligned, black pad).
pub fn fit(px: &[u8], w: u32, h: u32, size: (u32, u32)) -> Vec<u8> {
    let (tw, th) = (size.0 as usize, size.1 as usize);
    let (w, h) = (w as usize, h as usize);
    if (w, h) == (tw, th) && px.len() >= tw * th * 4 {
        return px[..tw * th * 4].to_vec();
    }
    let mut out = vec![0u8; tw * th * 4];
    for y in 0..th.min(h) {
        let n = tw.min(w) * 4;
        let (s, d) = (y * w * 4, y * tw * 4);
        if let (Some(src), Some(dst)) = (px.get(s..s + n), out.get_mut(d..d + n)) {
            dst.copy_from_slice(src);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_size_caps_the_long_side_and_keeps_it_even() {
        assert_eq!(record_size(1080, 2400), (576, 1280));
        assert_eq!(record_size(720, 1280), (720, 1280));
        assert_eq!(record_size(1601, 2561), (800, 1280));
    }

    #[test]
    fn fit_pads_and_crops() {
        let px = vec![7u8; 2 * 2 * 4];
        let out = fit(&px, 2, 2, (3, 1));
        assert_eq!(out.len(), 12);
        assert_eq!(&out[..8], &[7; 8]);
        assert_eq!(&out[8..], &[0; 4]);
    }

    #[test]
    fn default_paths_are_unique() {
        let tmp = tempfile::tempdir().unwrap();
        let a = default_path(tmp.path(), "Pixel/ish", "png");
        std::fs::write(&a, b"x").unwrap();
        let b = default_path(tmp.path(), "Pixel/ish", "png");
        assert_ne!(a, b);
        assert!(
            a.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Pixel-ish ")
        );
    }

    /// Encodes 2 s of frames and checks the file is a valid MP4.
    #[cfg(target_os = "macos")]
    #[test]
    fn writes_an_mp4() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("t.mp4");
        let mut w = mp4::Mp4Writer::create(&path, 320, 240, 30).unwrap();
        let mut px = vec![0u8; 320 * 240 * 4];
        for i in 0..60u32 {
            px.iter_mut().for_each(|b| *b = (i * 4) as u8);
            w.push(&px).unwrap();
        }
        w.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 1000);
        assert_eq!(&bytes[4..8], b"ftyp");
    }
}
