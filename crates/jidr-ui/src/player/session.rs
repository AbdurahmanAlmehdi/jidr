//! A live player session on one emulator, talking to it directly (plan,
//! "Display pipeline"; ADR-0001): `streamScreenshot` RGBA8888 at the
//! window's physical size into a newest-frame slot, input over
//! `streamInputEvent`, unary calls for keys, rotation and clipboard.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use jidr_core::{ConnectInfo, JidrResult};
use jidr_proto::Controller;
use jidr_proto::control as pb;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer, Weak};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

use super::input::Screen;
use crate::AppWindow;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Live numbers for the debug overlay and the bench.
#[derive(Debug, Default)]
pub struct Stats {
    pub received: AtomicU64,
    pub shown: AtomicU64,
    pub bytes: AtomicU64,
    /// Arrival time of each frame (last few seconds).
    pub arrivals: Mutex<VecDeque<Instant>>,
    /// Emulator timestamp → shown on the UI thread, ms.
    pub ages: Mutex<VecDeque<f32>>,
    /// Touch sent → next frame received, ms.
    pub latencies: Mutex<VecDeque<f32>>,
    pub touch_sent: Mutex<Option<Instant>>,
    /// Last frame size in pixels as received.
    pub frame: Mutex<(u32, u32)>,
}

impl Stats {
    fn push<T>(q: &Mutex<VecDeque<T>>, v: T, keep: usize) {
        let mut q = lock(q);
        q.push_back(v);
        while q.len() > keep {
            q.pop_front();
        }
    }

    /// Frames received per second over the last `window`.
    pub fn fps(&self, window: Duration) -> f32 {
        let now = Instant::now();
        let q = lock(&self.arrivals);
        let n = q
            .iter()
            .filter(|t| now.duration_since(**t) <= window)
            .count();
        n as f32 / window.as_secs_f32()
    }

    /// 1 / median frame interval, ignoring gaps over 100 ms (idle screen).
    pub fn active_fps(&self) -> f32 {
        let q = lock(&self.arrivals);
        let mut iv: Vec<f32> = q
            .iter()
            .zip(q.iter().skip(1))
            .map(|(a, b)| b.duration_since(*a).as_secs_f32() * 1000.0)
            .filter(|ms| *ms < 100.0)
            .collect();
        median(&mut iv).map(|m| 1000.0 / m).unwrap_or(0.0)
    }

    pub fn median_latency(&self) -> Option<f32> {
        median(&mut lock(&self.latencies).iter().copied().collect::<Vec<_>>())
    }

    pub fn median_age(&self) -> Option<f32> {
        median(&mut lock(&self.ages).iter().copied().collect::<Vec<_>>())
    }
}

pub fn median(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    Some(v[v.len() / 2])
}

pub fn percentile(v: &mut [f32], p: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let i = ((v.len() - 1) as f32 * p).round() as usize;
    v.get(i).copied()
}

struct Frame {
    width: u32,
    height: u32,
    data: Vec<u8>,
    ts_us: u64,
}

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Device pixels in the orientation a frame shows.
pub fn oriented(native: Screen, frame: (u32, u32)) -> Screen {
    let landscape_frame = frame.0 > frame.1;
    let landscape_native = native.width > native.height;
    if landscape_frame != landscape_native {
        Screen {
            width: native.height,
            height: native.width,
        }
    } else {
        native
    }
}

/// The size to request: the screen's physical pixels, capped at the
/// device's own size, both sides set (ADR-0001: width-only is ignored).
pub fn request_size(logical: (f32, f32), scale: f32, device: Screen) -> (u32, u32) {
    let (w, h) = (logical.0 * scale, logical.1 * scale);
    if w <= 0.0 || h <= 0.0 {
        return (device.width.min(540), device.height.min(1200));
    }
    // Never more pixels than the device has.
    let k = (device.width as f32 / w)
        .min(device.height as f32 / h)
        .min(1.0);
    let (w, h) = (w * k, h * k);
    ((w.round() as u32).max(1), (h.round() as u32).max(1))
}

/// An authenticated `EmulatorController` client (`-grpc-use-token`).
pub async fn connect(info: &ConnectInfo) -> JidrResult<Controller> {
    use jidr_core::JidrError;
    let token = jidr_proto::BearerToken::new(&info.grpc_token)
        .map_err(|_| JidrError::unexpected("invalid gRPC token"))?;
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", info.grpc_endpoint))
        .map_err(|e| JidrError::unexpected(format!("gRPC endpoint: {e}")))?
        .connect_timeout(Duration::from_secs(5))
        .tcp_nodelay(true)
        .connect()
        .await
        .map_err(|e| {
            JidrError::sdk(format!(
                "can't reach the emulator at {}: {e}",
                info.grpc_endpoint
            ))
        })?;
    Ok(jidr_proto::clients(channel, token).0)
}

pub struct Session {
    pub device: Uuid,
    pub native: Screen,
    pub ctl: Controller,
    pub input: mpsc::UnboundedSender<pb::InputEvent>,
    pub want: watch::Sender<(u32, u32)>,
    pub stats: Arc<Stats>,
    tasks: Vec<JoinHandle<()>>,
    clipboard: Option<Vec<JoinHandle<()>>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
        for t in self.clipboard.take().into_iter().flatten() {
            t.abort();
        }
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Connects and starts streaming at `want` pixels.
    pub async fn open(
        device: Uuid,
        info: ConnectInfo,
        native: Screen,
        want: (u32, u32),
        weak: Weak<AppWindow>,
    ) -> JidrResult<Self> {
        let ctl = connect(&info).await?;
        let stats = Arc::new(Stats::default());
        let (tx, rx) = mpsc::unbounded_channel::<pb::InputEvent>();
        let (want_tx, want_rx) = watch::channel(want);
        let mut tasks = Vec::new();
        {
            let mut c = ctl.clone();
            tasks.push(tokio::spawn(async move {
                if let Err(e) = c.stream_input_event(UnboundedReceiverStream::new(rx)).await {
                    tracing::warn!("input stream ended: {e}");
                }
            }));
        }
        tasks.push(tokio::spawn(stream_frames(
            ctl.clone(),
            want_rx,
            stats.clone(),
            native,
            weak,
        )));
        Ok(Self {
            device,
            native,
            ctl,
            input: tx,
            want: want_tx,
            stats,
            tasks,
            clipboard: None,
        })
    }

    /// Sends a touch (and marks it for the latency probe on press/move).
    pub fn send(&self, ev: pb::InputEvent, probe: bool) {
        if probe {
            let mut t = lock(&self.stats.touch_sent);
            if t.is_none() {
                *t = Some(Instant::now());
            }
        }
        let _ = self.input.send(ev);
    }

    /// Re-requests frames at a new size (resize, rotation, Retina change).
    pub fn resize(&self, want: (u32, u32)) {
        let cur = *self.want.borrow();
        let changed = |a: u32, b: u32| (a as f32 - b as f32).abs() / (b.max(1) as f32) > 0.04;
        if changed(want.0, cur.0) || changed(want.1, cur.1) {
            let _ = self.want.send(want);
        }
    }

    /// Shares the clipboard both ways while on.
    pub fn set_clipboard_sharing(&mut self, on: bool) {
        for t in self.clipboard.take().into_iter().flatten() {
            t.abort();
        }
        if !on {
            return;
        }
        let last = Arc::new(Mutex::new(String::new()));
        let mut down = self.ctl.clone();
        let l1 = last.clone();
        let device_to_host = tokio::spawn(async move {
            let Ok(resp) = down.stream_clipboard(()).await else {
                return;
            };
            let mut s = resp.into_inner();
            while let Ok(Some(clip)) = s.message().await {
                let changed = {
                    let mut l = lock(&l1);
                    if *l != clip.text {
                        l.clone_from(&clip.text);
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    crate::settings::copy_text(&clip.text);
                }
            }
        });
        let mut up = self.ctl.clone();
        let host_to_device = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let Ok(out) = tokio::process::Command::new("pbpaste").output().await else {
                    continue;
                };
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                let send = {
                    let mut l = lock(&last);
                    if !text.is_empty() && *l != text {
                        l.clone_from(&text);
                        true
                    } else {
                        false
                    }
                };
                if send {
                    let _ = up.set_clipboard(pb::ClipData { text }).await;
                }
            }
        });
        self.clipboard = Some(vec![device_to_host, host_to_device]);
    }

    pub fn clipboard_sharing(&self) -> bool {
        self.clipboard.is_some()
    }
}

async fn stream_frames(
    mut ctl: Controller,
    mut want: watch::Receiver<(u32, u32)>,
    stats: Arc<Stats>,
    native: Screen,
    weak: Weak<AppWindow>,
) {
    let latest: Arc<Mutex<Option<Frame>>> = Arc::default();
    let pending = Arc::new(AtomicBool::new(false));
    loop {
        let (w, h) = *want.borrow_and_update();
        let fmt = pb::ImageFormat {
            format: pb::image_format::ImgFormat::Rgba8888 as i32,
            width: w,
            height: h,
            ..Default::default()
        };
        let mut stream = match ctl.stream_screenshot(fmt).await {
            Ok(s) => s.into_inner(),
            Err(e) => {
                tracing::warn!("streamScreenshot: {e}");
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        loop {
            tokio::select! {
                changed = want.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
                msg = stream.message() => {
                    let img = match msg {
                        Ok(Some(img)) => img,
                        Ok(None) | Err(_) => {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            break;
                        }
                    };
                    let Some(f) = img.format else { continue };
                    if img.image.is_empty() {
                        continue;
                    }
                    let now = Instant::now();
                    stats.received.fetch_add(1, Ordering::Relaxed);
                    stats.bytes.fetch_add(img.image.len() as u64, Ordering::Relaxed);
                    Stats::push(&stats.arrivals, now, 600);
                    if let Some(t) = lock(&stats.touch_sent).take() {
                        Stats::push(&stats.latencies, now.duration_since(t).as_secs_f32() * 1000.0, 200);
                    }
                    *lock(&stats.frame) = (f.width, f.height);
                    *lock(&latest) = Some(Frame {
                        width: f.width,
                        height: f.height,
                        data: img.image,
                        ts_us: img.timestamp_us,
                    });
                    // One hop to the UI thread in flight; newer frames replace older ones.
                    if !pending.swap(true, Ordering::AcqRel) {
                        let (latest, pending, stats, weak) =
                            (latest.clone(), pending.clone(), stats.clone(), weak.clone());
                        let _ = slint::invoke_from_event_loop(move || {
                            pending.store(false, Ordering::Release);
                            let Some(frame) = lock(&latest).take() else { return };
                            let Some(app) = weak.upgrade() else { return };
                            if frame.data.len() < (frame.width * frame.height * 4) as usize {
                                return;
                            }
                            let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                                &frame.data,
                                frame.width,
                                frame.height,
                            );
                            let dev = oriented(native, (frame.width, frame.height));
                            app.set_player_frame(Image::from_rgba8(buf));
                            app.set_player_has_frame(true);
                            app.set_player_frame_w(dev.width as i32);
                            app.set_player_frame_h(dev.height as i32);
                            stats.shown.fetch_add(1, Ordering::Relaxed);
                            Stats::push(&stats.ages, now_us().saturating_sub(frame.ts_us) as f32 / 1000.0, 200);
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: Screen = Screen {
        width: 1080,
        height: 2400,
    };

    #[test]
    fn requests_physical_size_capped_at_device() {
        assert_eq!(request_size((400.0, 889.0), 2.0, PHONE), (800, 1778));
        assert_eq!(request_size((800.0, 1778.0), 2.0, PHONE), (1080, 2400));
        assert_eq!(request_size((0.0, 0.0), 2.0, PHONE), (540, 1200));
    }

    #[test]
    fn orientation_follows_the_frame() {
        assert_eq!(oriented(PHONE, (399, 888)), PHONE);
        assert_eq!(
            oriented(PHONE, (888, 399)),
            Screen {
                width: 2400,
                height: 1080
            }
        );
    }

    #[test]
    fn stats_medians() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(percentile(&mut [1.0, 2.0, 3.0, 4.0, 5.0], 0.9), Some(5.0));
        assert_eq!(median(&mut []), None);
    }
}
