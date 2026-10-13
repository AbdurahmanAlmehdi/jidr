//! Phase 1 spike (throwaway): stream emulator frames over gRPC into a Slint
//! window, send touches back, and measure fps / CPU / input latency.
//!
//! ```text
//! cargo run -p spike-display -- --create                  # spike-only AVD, interactive
//! cargo run -p spike-display -- --create --bench fps      # 10 s of scripted scrolling
//! cargo run -p spike-display -- --create --bench latency  # show_touches pixel probe
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::Request;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

#[allow(clippy::all, clippy::pedantic)]
mod pb {
    tonic::include_proto!("android.emulation.control");
}

use pb::emulator_controller_client::EmulatorControllerClient;

slint::slint! {
    export component Player inherits Window {
        in property <image> frame;
        in property <string> stats;
        in property <length> view-width: 400px;
        in property <length> view-height: 889px;
        title: "Jidr display spike";
        width: view-width;
        height: view-height;
        background: black;

        // kind: 0 = down, 1 = move, 2 = up
        callback pointer(int, length, length);

        Image {
            width: parent.width;
            height: parent.height;
            source: frame;
            image-fit: contain;
        }

        TouchArea {
            pointer-event(ev) => {
                if (ev.kind == PointerEventKind.down) {
                    root.pointer(0, self.mouse-x, self.mouse-y);
                } else if (ev.kind == PointerEventKind.up || ev.kind == PointerEventKind.cancel) {
                    root.pointer(2, self.mouse-x, self.mouse-y);
                }
            }
            moved => {
                if (self.pressed) {
                    root.pointer(1, self.mouse-x, self.mouse-y);
                }
            }
        }

        Rectangle {
            x: 4px;
            y: 4px;
            width: label.preferred-width + 8px;
            height: label.preferred-height + 4px;
            background: #000000a0;
            label := Text {
                text: stats;
                color: #7CFC00;
                font-size: 11px;
            }
        }
    }
}

const DEVICE_W: u32 = 1080;
const DEVICE_H: u32 = 2400;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Bench {
    None,
    Fps,
    Latency,
}

#[derive(Debug)]
struct Args {
    sdk: PathBuf,
    avd: String,
    create: bool,
    image: String,
    avd_home: Option<PathBuf>,
    window_width: u32,
    frame_width: u32,
    bench: Bench,
    seconds: u64,
    samples: usize,
    keep: bool,
    dump: Option<PathBuf>,
    gpu: Option<String>,
}

fn parse_args() -> Result<Args> {
    let home = dirs::home_dir().context("no home dir")?;
    let mut a = Args {
        sdk: std::env::var_os("ANDROID_SDK_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("Library/Android/sdk")),
        avd: "jidr_spike".into(),
        create: false,
        image: "system-images/android-31/google_apis/arm64-v8a/".into(),
        avd_home: None,
        window_width: 400,
        frame_width: 0,
        bench: Bench::None,
        seconds: 10,
        samples: 20,
        keep: false,
        dump: None,
        gpu: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--sdk" => a.sdk = val()?.into(),
            "--avd" => a.avd = val()?,
            "--create" => a.create = true,
            "--image" => a.image = val()?,
            "--avd-home" => a.avd_home = Some(val()?.into()),
            "--window-width" => a.window_width = val()?.parse()?,
            "--frame-width" => a.frame_width = val()?.parse()?,
            "--seconds" => a.seconds = val()?.parse()?,
            "--samples" => a.samples = val()?.parse()?,
            "--keep" => a.keep = true,
            "--dump" => a.dump = Some(val()?.into()),
            "--gpu" => a.gpu = Some(val()?),
            "--bench" => {
                a.bench = match val()?.as_str() {
                    "fps" => Bench::Fps,
                    "latency" => Bench::Latency,
                    other => bail!("unknown bench {other}"),
                }
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if a.frame_width == 0 {
        a.frame_width = a.window_width;
    }
    if a.create && a.avd_home.is_none() {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"));
        a.avd_home = Some(std::path::absolute(target.join("spike-avd"))?);
    }
    Ok(a)
}

/// Writes a minimal AVD into `home` so the spike never touches ~/.android/avd.
fn create_avd(home: &Path, name: &str, image: &str) -> Result<()> {
    let dir = home.join(format!("{name}.avd"));
    std::fs::create_dir_all(&dir)?;
    let api = image
        .split('/')
        .find_map(|s| s.strip_prefix("android-"))
        .unwrap_or("31");
    let config = format!(
        "AvdId={name}\navd.ini.encoding=UTF-8\nabi.type=arm64-v8a\nhw.cpu.arch=arm64\n\
         hw.cpu.ncore=4\nhw.ramSize=2048\nhw.lcd.width={DEVICE_W}\nhw.lcd.height={DEVICE_H}\n\
         hw.lcd.density=420\nhw.gpu.enabled=yes\nhw.gpu.mode=auto\nhw.keyboard=yes\n\
         image.sysdir.1={image}\ntag.id=google_apis\ntarget=android-{api}\n\
         disk.dataPartition.size=4G\nPlayStore.enabled=false\nfastboot.forceColdBoot=yes\n"
    );
    std::fs::write(dir.join("config.ini"), config)?;
    std::fs::write(
        home.join(format!("{name}.ini")),
        format!(
            "avd.ini.encoding=UTF-8\npath={}\ntarget=android-{api}\n",
            dir.display()
        ),
    )?;
    Ok(())
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

/// An even console port whose +1 adb port is also free.
fn free_console_port() -> Result<u16> {
    (5554..5680)
        .step_by(2)
        .find(|p| {
            std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()
                && std::net::TcpListener::bind(("127.0.0.1", *p + 1)).is_ok()
        })
        .context("no free console port")
}

fn parse_ini(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

#[derive(Clone)]
struct Bearer(MetadataValue<Ascii>);

impl Interceptor for Bearer {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, tonic::Status> {
        req.metadata_mut().insert("authorization", self.0.clone());
        Ok(req)
    }
}

type Client = EmulatorControllerClient<InterceptedService<Channel, Bearer>>;

struct Frame {
    width: u32,
    height: u32,
    data: Vec<u8>,
    emu_ts_us: u64,
}

#[derive(Default)]
struct Stats {
    received: AtomicU64,
    shown: AtomicU64,
    bytes: AtomicU64,
    stop: AtomicBool,
    /// Emulator frame timestamp → shown on the UI thread, in µs.
    frame_age_us: Mutex<Vec<u64>>,
    /// Arrival time of every received frame.
    arrivals: Mutex<Vec<Instant>>,
    probe: Mutex<Probe>,
}

/// Latency probe state, shared between the bench task and the UI thread.
#[derive(Default)]
struct Probe {
    /// Device-pixel position watched for a change.
    xy: Option<(u32, u32)>,
    /// Probe pixel of the most recently shown frame.
    last: Option<[u8; 3]>,
    baseline: Option<[u8; 3]>,
    sent: Option<Instant>,
    hit: Option<Instant>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

fn touch(x: u32, y: u32, down: bool) -> pb::InputEvent {
    pb::InputEvent {
        r#type: Some(pb::input_event::Type::TouchEvent(pb::TouchEvent {
            touches: vec![pb::Touch {
                x: x as i32,
                y: y as i32,
                identifier: 1,
                pressure: if down { 100 } else { 0 },
                touch_major: if down { 5 } else { 0 },
                expiration: pb::touch::EventExpiration::NeverExpire as i32,
                ..Default::default()
            }],
            display: 0,
        })),
    }
}

/// adb + pid of the spawned emulator; cheap to clone into tasks.
#[derive(Clone)]
struct Device {
    adb: PathBuf,
    serial: String,
    pid: u32,
}

impl Device {
    fn adb(&self, args: &[&str]) -> Result<String> {
        let out = Command::new(&self.adb)
            .arg("-s")
            .arg(&self.serial)
            .args(args)
            .output()?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Cumulative CPU time of a process via `ps -o time=` ("M:SS.ss" or "H:MM:SS").
fn cpu_seconds(pid: u32) -> f64 {
    let Ok(out) = Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
    else {
        return 0.0;
    };
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .split(':')
        .fold(0.0, |acc, part| {
            acc * 60.0 + part.parse::<f64>().unwrap_or(0.0)
        })
}

fn launch(args: &Args) -> Result<(Child, Device, u16, String)> {
    if let (true, Some(home)) = (args.create, &args.avd_home) {
        create_avd(home, &args.avd, &args.image)?;
    }
    let grpc = free_port()?;
    let console = free_console_port()?;
    let mut cmd = Command::new(args.sdk.join("emulator/emulator"));
    cmd.args([
        "-avd",
        &args.avd,
        "-no-window",
        "-no-boot-anim",
        "-no-snapshot",
        "-no-audio",
    ])
    .args(["-grpc", &grpc.to_string(), "-grpc-use-token"])
    .args(["-port", &console.to_string()])
    .env("ANDROID_SDK_ROOT", &args.sdk)
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    if let Some(gpu) = &args.gpu {
        cmd.args(["-gpu", gpu]);
    }
    if let Some(home) = &args.avd_home {
        cmd.env("ANDROID_AVD_HOME", home);
    }
    let child = cmd.spawn().context("spawning emulator")?;
    let pid = child.id();
    let device = Device {
        adb: args.sdk.join("platform-tools/adb"),
        serial: format!("emulator-{console}"),
        pid,
    };
    println!("spike: emulator pid {pid}, console {console}, grpc {grpc}");
    let disc = dirs::home_dir().context("no home dir")?.join(format!(
        "Library/Caches/TemporaryItems/avd/running/pid_{pid}.ini"
    ));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(text) = std::fs::read_to_string(&disc)
            && let Some(token) = parse_ini(&text).get("grpc.token")
        {
            return Ok((child, device, grpc, token.clone()));
        }
        if Instant::now() > deadline {
            bail!("no grpc.token in {} after 60 s", disc.display());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

async fn connect(port: u16, token: &str) -> Result<Client> {
    let bearer = Bearer(format!("Bearer {token}").parse()?);
    for _ in 0..50 {
        if let Ok(channel) = Channel::from_shared(format!("http://127.0.0.1:{port}"))?
            .connect()
            .await
        {
            return Ok(EmulatorControllerClient::with_interceptor(channel, bearer)
                .max_decoding_message_size(64 * 1024 * 1024));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(anyhow!("could not connect to gRPC on {port}"))
}

async fn wait_boot(client: &mut Client) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Ok(s) = client.get_status(()).await
            && s.get_ref().booted
        {
            println!("spike: booted in {:.1} s", start.elapsed().as_secs_f64());
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(300) {
            bail!("boot timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn pixel(f: &Frame, x: u32, y: u32) -> Option<[u8; 3]> {
    let i = ((y * f.width + x) * 4) as usize;
    f.data.get(i..i + 3).map(|p| [p[0], p[1], p[2]])
}

/// Called on the UI thread with the newest frame.
fn show(player: &Player, f: Frame, stats: &Stats) {
    {
        let mut probe = lock(&stats.probe);
        if let Some((px, py)) = probe.xy {
            let now = pixel(&f, px * f.width / DEVICE_W, py * f.height / DEVICE_H);
            probe.last = now;
            if let (Some(_), None, Some(base), Some(now)) =
                (probe.sent, probe.hit, probe.baseline, now)
            {
                let diff: u32 = base
                    .iter()
                    .zip(now)
                    .map(|(a, b)| u32::from(a.abs_diff(b)))
                    .sum();
                if diff > 60 {
                    probe.hit = Some(Instant::now());
                }
            }
        }
    }
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&f.data, f.width, f.height);
    player.set_frame(Image::from_rgba8(buf));
    stats.shown.fetch_add(1, Ordering::Relaxed);
    lock(&stats.frame_age_us).push(now_us().saturating_sub(f.emu_ts_us));
}

async fn stream_frames(
    mut client: Client,
    width: u32,
    stats: Arc<Stats>,
    weak: slint::Weak<Player>,
    dump: Option<PathBuf>,
) -> Result<()> {
    let latest: Arc<Mutex<Option<Frame>>> = Arc::default();
    let pending = Arc::new(AtomicBool::new(false));
    // Finding: the emulator ignores a width-only request (returns full size),
    // so pass both dimensions; it keeps the aspect ratio.
    let fmt = pb::ImageFormat {
        format: pb::image_format::ImgFormat::Rgba8888 as i32,
        width,
        height: width * DEVICE_H / DEVICE_W,
        ..Default::default()
    };
    let mut stream = client.stream_screenshot(fmt).await?.into_inner();
    let mut dump = dump;
    while let Some(img) = stream.message().await? {
        let Some(fmt) = img.format else { continue };
        if img.image.is_empty() || stats.stop.load(Ordering::Relaxed) {
            continue;
        }
        stats.received.fetch_add(1, Ordering::Relaxed);
        lock(&stats.arrivals).push(Instant::now());
        stats
            .bytes
            .fetch_add(img.image.len() as u64, Ordering::Relaxed);
        let frame = Frame {
            width: fmt.width,
            height: fmt.height,
            data: img.image,
            emu_ts_us: img.timestamp_us,
        };
        if let Some(path) = dump.take() {
            write_png(&path, &frame)?;
        }
        *lock(&latest) = Some(frame);
        // Only one hop to the UI thread in flight; newer frames replace older ones.
        if !pending.swap(true, Ordering::AcqRel) {
            let (latest, pending, stats, weak) =
                (latest.clone(), pending.clone(), stats.clone(), weak.clone());
            slint::invoke_from_event_loop(move || {
                pending.store(false, Ordering::Release);
                let frame = lock(&latest).take();
                if let (Some(f), Some(p)) = (frame, weak.upgrade()) {
                    show(&p, f, &stats);
                }
            })?;
        }
    }
    Ok(())
}

fn write_png(path: &Path, f: &Frame) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, f.width, f.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&f.data)?;
    println!(
        "spike: dumped first frame {}x{} to {}",
        f.width,
        f.height,
        path.display()
    );
    Ok(())
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v.get(i).copied().unwrap_or(f64::NAN)
}

async fn bench_fps(
    dev: &Device,
    input: &mpsc::UnboundedSender<pb::InputEvent>,
    stats: &Stats,
    seconds: u64,
) -> Result<String> {
    dev.adb(&["shell", "am", "start", "-a", "android.settings.SETTINGS"])?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (r0, s0, b0) = (
        stats.received.load(Ordering::Relaxed),
        stats.shown.load(Ordering::Relaxed),
        stats.bytes.load(Ordering::Relaxed),
    );
    lock(&stats.frame_age_us).clear();
    lock(&stats.arrivals).clear();
    let (emu_cpu0, self_cpu0) = (cpu_seconds(dev.pid), cpu_seconds(std::process::id()));
    let start = Instant::now();
    let mut up = true;
    while start.elapsed() < Duration::from_secs(seconds) {
        // One 300 ms drag, alternating direction.
        let (y0, y1): (i64, i64) = if up { (1900, 700) } else { (700, 1900) };
        up = !up;
        let steps = 30;
        for i in 0..=steps {
            let y = y0 + (y1 - y0) * i / steps;
            input.send(touch(540, y as u32, true))?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        input.send(touch(540, y1 as u32, false))?;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let secs = start.elapsed().as_secs_f64();
    let received = stats.received.load(Ordering::Relaxed) - r0;
    let shown = stats.shown.load(Ordering::Relaxed) - s0;
    let mb = (stats.bytes.load(Ordering::Relaxed) - b0) as f64 / 1e6;
    let emu_cpu = (cpu_seconds(dev.pid) - emu_cpu0) / secs * 100.0;
    let self_cpu = (cpu_seconds(std::process::id()) - self_cpu0) / secs * 100.0;
    let mut ages: Vec<f64> = lock(&stats.frame_age_us)
        .iter()
        .map(|a| *a as f64 / 1000.0)
        .collect();
    // Active-scroll rate: intervals under 100 ms (gaps between drags excluded).
    let arrivals = lock(&stats.arrivals).clone();
    let mut intervals: Vec<f64> = arrivals
        .windows(2)
        .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.0)
        .filter(|ms| *ms < 100.0)
        .collect();
    let active_fps = 1000.0 / percentile(&mut intervals, 0.5);
    let active_p10 = 1000.0 / percentile(&mut intervals, 0.9);
    Ok(format!(
        "active fps: median {active_fps:.1}, p10 {active_p10:.1} (from {} intervals)\nfps: received {:.1}/s, shown {:.1}/s, {:.0} MB/s; frame age (emulator ts -> shown) median {:.1} ms p90 {:.1} ms; CPU emulator {:.0}%, spike {:.0}%",
        intervals.len(),
        received as f64 / secs,
        shown as f64 / secs,
        mb / secs,
        percentile(&mut ages, 0.5),
        percentile(&mut ages, 0.9),
        emu_cpu,
        self_cpu,
    ))
}

async fn bench_latency(
    dev: &Device,
    input: &mpsc::UnboundedSender<pb::InputEvent>,
    stats: &Stats,
    samples: usize,
) -> Result<String> {
    dev.adb(&["shell", "settings", "put", "system", "show_touches", "1"])?;
    dev.adb(&["shell", "input", "keyevent", "KEYCODE_HOME"])?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (px, py) = (DEVICE_W / 2, DEVICE_H * 45 / 100);
    lock(&stats.probe).xy = Some((px, py));
    let mut latencies = Vec::new();
    for _ in 0..samples {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let sent = {
            let mut probe = lock(&stats.probe);
            probe.hit = None;
            probe.baseline = probe.last;
            let sent = Instant::now();
            probe.sent = Some(sent);
            sent
        };
        input.send(touch(px, py, true))?;
        let deadline = sent + Duration::from_secs(1);
        let hit = loop {
            let hit = lock(&stats.probe).hit;
            if hit.is_some() || Instant::now() > deadline {
                break hit;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        };
        if let Some(hit) = hit {
            latencies.push(hit.duration_since(sent).as_secs_f64() * 1000.0);
        }
        lock(&stats.probe).sent = None;
        input.send(touch(px, py, false))?;
    }
    dev.adb(&["shell", "settings", "put", "system", "show_touches", "0"])?;
    let detected = latencies.len();
    Ok(format!(
        "latency (touch sent -> changed frame on UI thread): {detected}/{samples} detected, median {:.1} ms, p90 {:.1} ms, min {:.1} ms, max {:.1} ms",
        percentile(&mut latencies, 0.5),
        percentile(&mut latencies, 0.9),
        percentile(&mut latencies, 0.0),
        percentile(&mut latencies, 1.0),
    ))
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let (mut child, dev, port, token) = launch(&args)?;
    let result = run(&args, &dev, port, &token);
    if !args.keep {
        let _ = dev.adb(&["emu", "kill"]);
        std::thread::sleep(Duration::from_secs(2));
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(report) = result? {
        println!("{report}");
    }
    Ok(())
}

fn run(args: &Args, dev: &Device, port: u16, token: &str) -> Result<Option<String>> {
    let rt = tokio::runtime::Runtime::new()?;
    let mut client = rt.block_on(connect(port, token))?;
    rt.block_on(wait_boot(&mut client))?;

    let player = Player::new()?;
    let ww = args.window_width as f32;
    player.set_view_width(ww);
    player.set_view_height(ww * DEVICE_H as f32 / DEVICE_W as f32);

    let stats = Arc::new(Stats::default());
    let (tx, rx) = mpsc::unbounded_channel::<pb::InputEvent>();

    // Input: UI pointer (logical px) → device px → streamInputEvent.
    {
        let tx = tx.clone();
        let scale = DEVICE_W as f32 / ww;
        player.on_pointer(move |kind, x, y| {
            let dx = (x * scale).clamp(0.0, DEVICE_W as f32 - 1.0) as u32;
            let dy = (y * scale).clamp(0.0, DEVICE_H as f32 - 1.0) as u32;
            let _ = tx.send(touch(dx, dy, kind != 2));
        });
    }
    let mut input_client = client.clone();
    rt.spawn(async move {
        let _ = input_client
            .stream_input_event(UnboundedReceiverStream::new(rx))
            .await;
    });

    rt.spawn(stream_frames(
        client.clone(),
        args.frame_width,
        stats.clone(),
        player.as_weak(),
        args.dump.clone(),
    ));

    // Overlay: fps once a second.
    let timer = slint::Timer::default();
    {
        let weak = player.as_weak();
        let stats = stats.clone();
        let frame_width = args.frame_width;
        let mut last = (Instant::now(), 0u64, 0u64);
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(1),
            move || {
                let r = stats.received.load(Ordering::Relaxed);
                let s = stats.shown.load(Ordering::Relaxed);
                let dt = last.0.elapsed().as_secs_f64();
                let text = format!(
                    "recv {:.0} fps · shown {:.0} fps · {frame_width}px frames",
                    (r - last.1) as f64 / dt,
                    (s - last.2) as f64 / dt,
                );
                last = (Instant::now(), r, s);
                if let Some(p) = weak.upgrade() {
                    p.set_stats(text.into());
                }
            },
        );
    }

    let report = Arc::new(Mutex::new(None::<String>));
    if args.bench != Bench::None {
        let (bench, seconds, samples) = (args.bench, args.seconds, args.samples);
        let (stats, report, dev) = (stats.clone(), report.clone(), dev.clone());
        rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let r = match bench {
                Bench::Fps => bench_fps(&dev, &tx, &stats, seconds).await,
                Bench::Latency => bench_latency(&dev, &tx, &stats, samples).await,
                Bench::None => Ok(String::new()),
            };
            *lock(&report) = Some(r.unwrap_or_else(|e| format!("bench failed: {e:#}")));
            stats.stop.store(true, Ordering::Relaxed);
            let _ = slint::invoke_from_event_loop(|| {
                let _ = slint::quit_event_loop();
            });
        });
    }

    player.run()?;
    let r = lock(&report).take();
    Ok(r)
}
