//! Physical phones over plain `adb` (#92, D46 proposed): `physical.*`.
//!
//! A phone isn't a Jidr device: it has no AVD, no id and no root modes,
//! snapshots or sensor widgets, so it gets its own command family instead
//! of a new `Device` kind (the protocol stays additive). Every phone command
//! takes the adb serial as `device`. Emulators that adb also lists (Jidr's,
//! Android Studio's) are left out.
//!
//! A watcher on `adb track-devices -l`, started by the first `physical.*`
//! call, publishes `physical.connected`, `physical.changed` and
//! `physical.disconnected` as phones come and go.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use jidr_adb::physical::{self as adbp, LongEntry};
use jidr_adb::{Adb, InstallOptions};
use jidr_core::provider::{
    CallCtx, CommandProvider, CommandSpec, DaemonContext, NoParams, parse_params,
};
use jidr_core::{
    JidrError, JidrResult, PhoneBattery, PhoneConnectInfo, PhoneConnection, PhoneScreen,
    PhoneState, PhysicalDevice,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::core_provider::{logcat_args, tail_lines};

/// Quick adb calls (props, battery, input).
const QUICK: Duration = Duration::from_secs(10);
/// Installs and file copies.
const SLOW: Duration = Duration::from_secs(600);
/// Android's own cap on one `screenrecord` run.
pub const RECORD_MAX_S: u32 = 180;
/// Wait before re-running `track-devices` after it ends.
const RETRACK: Duration = Duration::from_secs(2);

// ---- params -------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhoneParams {
    /// The phone's adb serial (`physical.list`), or its model when only one
    /// connected phone has it.
    pub device: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotParams {
    pub device: String,
    /// Where to save the PNG (default `~/Pictures/Jidr/<model> <time>.png`).
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordParams {
    pub device: String,
    /// Where to save the MP4 (default `~/Movies/Jidr/<model> <time>.mp4`).
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Stop on its own after this many seconds (1–180, Android's limit; default 180).
    #[serde(default)]
    pub max_seconds: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallParams {
    pub device: String,
    /// Path of the APK on this machine.
    pub apk: PathBuf,
    /// Replace an installed app (`-r`).
    #[serde(default)]
    pub replace: bool,
    /// Grant runtime permissions (`-g`).
    #[serde(default)]
    pub grant_permissions: bool,
    /// Allow a version downgrade (`-d`).
    #[serde(default)]
    pub downgrade: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushParams {
    pub device: String,
    /// Local file.
    pub local: PathBuf,
    /// Where on the phone (default `/sdcard/Download/`).
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullParams {
    pub device: String,
    /// Path on the phone.
    pub remote: String,
    /// Where to save it on this machine.
    pub local: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogcatParams {
    pub device: String,
    /// Only the last N lines.
    #[serde(default)]
    pub lines: Option<u32>,
    /// logcat filter specs, e.g. `["ActivityManager:I", "*:S"]`.
    #[serde(default)]
    pub filter: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellParams {
    pub device: String,
    /// One shell command line, run as the shell user (no root).
    pub command: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TapParams {
    pub device: String,
    /// Screen pixels from the left.
    pub x: u32,
    /// Screen pixels from the top.
    pub y: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SwipeParams {
    pub device: String,
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
    /// How long the swipe takes (default 300 ms).
    #[serde(default)]
    pub duration_ms: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyParams {
    pub device: String,
    /// `back`, `home`, `recents`, `power`, `volume_up`, `volume_down`,
    /// `enter`, `delete`, a W3C key name (`GoBack`, `ArrowUp`…), a
    /// `KEYCODE_*` name or an Android key code.
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextParams {
    pub device: String,
    /// Printable ASCII to type into the focused field.
    pub text: String,
}

// ---- the provider -------------------------------------------------------

/// A recording in progress, by serial.
type Recordings = Arc<Mutex<HashMap<String, Arc<Notify>>>>;

#[derive(Default)]
pub struct PhysicalProvider {
    /// `getprop` by serial, kept until the phone goes away or changes state.
    props: Arc<Mutex<HashMap<String, BTreeMap<String, String>>>>,
    recordings: Recordings,
    watching: Mutex<bool>,
}

impl std::fmt::Debug for PhysicalProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicalProvider").finish_non_exhaustive()
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<NoParams>(
            "physical.list",
            "Physical Android phones adb sees (USB debugging on): state, model, Android version, battery",
        ),
        CommandSpec::new::<PhoneParams>(
            "physical.details",
            "One phone's details, with its screen size",
        ),
        CommandSpec::new::<PhoneParams>(
            "physical.connect_info",
            "The adb serial and adb binary to reach a ready phone directly",
        ),
        CommandSpec::new::<ScreenshotParams>(
            "physical.screenshot",
            "Save a PNG of a phone's screen (adb screencap)",
        ),
        CommandSpec::new::<RecordParams>(
            "physical.record",
            "Record a phone's screen to MP4 (adb screenrecord) until physical.record_stop or max_seconds",
        )
        .long_running(),
        CommandSpec::new::<PhoneParams>(
            "physical.record_stop",
            "Stop a phone's screen recording and save it",
        ),
        CommandSpec::new::<InstallParams>("physical.install", "Install an APK on a phone"),
        CommandSpec::new::<PushParams>(
            "physical.push",
            "Copy a file to a phone (default /sdcard/Download/)",
        ),
        CommandSpec::new::<PullParams>("physical.pull", "Copy a file from a phone"),
        CommandSpec::new::<LogcatParams>("physical.logcat", "Dump a phone's log"),
        CommandSpec::new::<ShellParams>(
            "physical.shell",
            "Run a shell command on a phone (as the shell user)",
        )
        .destructive(),
        CommandSpec::new::<PhoneParams>("physical.reboot", "Reboot a phone"),
        CommandSpec::new::<TapParams>("physical.tap", "Tap a phone's screen (screen pixels)"),
        CommandSpec::new::<SwipeParams>("physical.swipe", "Swipe on a phone's screen (screen pixels)"),
        CommandSpec::new::<KeyParams>("physical.key", "Press a key on a phone (back, home, power…)"),
        CommandSpec::new::<TextParams>("physical.text", "Type text into a phone's focused field"),
    ]
}

#[async_trait]
impl CommandProvider for PhysicalProvider {
    fn name(&self) -> &str {
        "physical"
    }

    fn commands(&self) -> Vec<CommandSpec> {
        specs()
    }

    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
        self.watch(ctx);
        match method {
            "physical.list" => {
                parse_params::<NoParams>(params)?;
                let phones = self.scan(ctx, false).await?;
                Ok(serde_json::to_value(phones).unwrap_or(Value::Null))
            }
            "physical.details" => {
                let p: PhoneParams = parse_params(params)?;
                let phones = self.scan(ctx, true).await?;
                let phone = pick(&phones, &p.device)?;
                Ok(serde_json::to_value(phone).unwrap_or(Value::Null))
            }
            "physical.connect_info" => {
                let p: PhoneParams = parse_params(params)?;
                let (adb, _) = self.ready(ctx, &p.device).await?;
                let info = PhoneConnectInfo {
                    serial: adb.serial().unwrap_or_default().to_string(),
                    adb_path: adb.bin().to_path_buf(),
                };
                Ok(serde_json::to_value(info).unwrap_or(Value::Null))
            }
            "physical.screenshot" => {
                let p: ScreenshotParams = parse_params(params)?;
                let (adb, phone) = self.ready(ctx, &p.device).await?;
                let png = adb
                    .with_timeout(Duration::from_secs(30))
                    .screencap_png()
                    .await?;
                let path = p.path.unwrap_or_else(|| {
                    jidr_core::paths::capture_path(
                        &jidr_core::paths::capture_dir("Pictures"),
                        phone.display_name(),
                        "png",
                    )
                });
                write_file(&path, &png)?;
                let (w, h) = png_size(&png).unwrap_or((0, 0));
                Ok(json!({ "path": path, "width": w, "height": h }))
            }
            "physical.record" => {
                let p: RecordParams = parse_params(params)?;
                self.record(ctx, p).await
            }
            "physical.record_stop" => {
                let p: PhoneParams = parse_params(params)?;
                let phones = self.scan(ctx, false).await?;
                let serial = pick(&phones, &p.device)
                    .map(|ph| ph.serial.clone())
                    .unwrap_or(p.device);
                let stop = lock(&self.recordings).get(&serial).cloned();
                match stop {
                    Some(n) => {
                        n.notify_one();
                        Ok(json!({ "stopping": true }))
                    }
                    None => Err(JidrError::not_found(format!("{serial} isn't recording"))),
                }
            }
            "physical.install" => {
                let p: InstallParams = parse_params(params)?;
                if !p.apk.is_file() {
                    return Err(JidrError::usage(format!(
                        "no such APK: {}",
                        p.apk.display()
                    )));
                }
                let (adb, _) = self.ready(ctx, &p.device).await?;
                let opts = InstallOptions {
                    replace: p.replace,
                    grant_permissions: p.grant_permissions,
                    downgrade: p.downgrade,
                };
                adb.with_timeout(SLOW).install(&p.apk, opts).await?;
                Ok(json!({ "installed": p.apk }))
            }
            "physical.push" => {
                let p: PushParams = parse_params(params)?;
                if !p.local.exists() {
                    return Err(JidrError::usage(format!(
                        "no such file: {}",
                        p.local.display()
                    )));
                }
                let (adb, _) = self.ready(ctx, &p.device).await?;
                let remote = p.remote.unwrap_or_else(|| "/sdcard/Download/".into());
                adb.with_timeout(SLOW).push(&p.local, &remote).await?;
                Ok(json!({ "pushed": p.local, "remote": remote }))
            }
            "physical.pull" => {
                let p: PullParams = parse_params(params)?;
                let (adb, _) = self.ready(ctx, &p.device).await?;
                adb.with_timeout(SLOW).pull(&p.remote, &p.local).await?;
                Ok(json!({ "pulled": p.remote, "local": p.local }))
            }
            "physical.logcat" => {
                let p: LogcatParams = parse_params(params)?;
                let (adb, _) = self.ready(ctx, &p.device).await?;
                let args = logcat_args(p.lines, &p.filter);
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let text = adb
                    .with_timeout(Duration::from_secs(60))
                    .logcat_dump(&args)
                    .await?;
                let text = match (p.lines, p.filter.is_empty()) {
                    (Some(n), false) => tail_lines(&text, n as usize),
                    _ => text,
                };
                Ok(json!({ "text": text }))
            }
            "physical.shell" => {
                let p: ShellParams = parse_params(params)?;
                let (adb, _) = self.ready(ctx, &p.device).await?;
                let out = adb
                    .with_timeout(Duration::from_secs(120))
                    .raw(&["shell", &p.command])
                    .await?;
                Ok(json!({
                    "exit_code": out.status,
                    "stdout": out.stdout,
                    "stderr": out.stderr,
                }))
            }
            "physical.reboot" => {
                let p: PhoneParams = parse_params(params)?;
                let (adb, phone) = self.ready(ctx, &p.device).await?;
                adb.with_timeout(QUICK).run(&["reboot"]).await?;
                lock(&self.props).remove(&phone.serial);
                Ok(json!({ "rebooting": phone.serial }))
            }
            "physical.tap" => {
                let p: TapParams = parse_params(params)?;
                let (adb, _) = self.ready(ctx, &p.device).await?;
                adb.with_timeout(QUICK)
                    .shell(&format!("input tap {} {}", p.x, p.y))
                    .await?;
                Ok(json!({ "tapped": [p.x, p.y] }))
            }
            "physical.swipe" => {
                let p: SwipeParams = parse_params(params)?;
                let (adb, _) = self.ready(ctx, &p.device).await?;
                let ms = p.duration_ms.unwrap_or(300).clamp(1, 10_000);
                adb.with_timeout(QUICK)
                    .shell(&format!(
                        "input swipe {} {} {} {} {ms}",
                        p.x1, p.y1, p.x2, p.y2
                    ))
                    .await?;
                Ok(json!({ "swiped": [p.x1, p.y1, p.x2, p.y2], "duration_ms": ms }))
            }
            "physical.key" => {
                let p: KeyParams = parse_params(params)?;
                let code = adbp::android_keycode(&p.key)
                    .ok_or_else(|| JidrError::usage(format!("unknown key {:?}", p.key)))?;
                let (adb, _) = self.ready(ctx, &p.device).await?;
                adb.with_timeout(QUICK)
                    .shell(&format!("input keyevent {code}"))
                    .await?;
                Ok(json!({ "key": p.key, "keycode": code }))
            }
            "physical.text" => {
                let p: TextParams = parse_params(params)?;
                if !p
                    .text
                    .chars()
                    .all(|c| c.is_ascii() && !c.is_ascii_control())
                {
                    return Err(JidrError::usage(
                        "physical.text types printable ASCII only (adb input text)",
                    ));
                }
                let (adb, _) = self.ready(ctx, &p.device).await?;
                if !p.text.is_empty() {
                    adb.with_timeout(QUICK)
                        .shell(&jidr_adb::input_text_command(&p.text))
                        .await?;
                }
                Ok(json!({ "typed": p.text.len() }))
            }
            _ => Err(JidrError::usage(format!("unknown method {method}"))),
        }
    }
}

impl PhysicalProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// The phones adb sees now, emulators left out. `details` adds the
    /// screen size.
    async fn scan(&self, ctx: &CallCtx, details: bool) -> JidrResult<Vec<PhysicalDevice>> {
        let adb = Adb::new(ctx.daemon.adb_path()?).with_timeout(QUICK);
        let entries = adb.devices_long().await?;
        let ours = emulator_serials(ctx.daemon.as_ref()).await;
        let mut phones = Vec::new();
        for e in entries {
            if adbp::is_emulator_serial(&e.serial) || ours.contains(&e.serial) {
                continue;
            }
            let mut phone = from_entry(&e);
            if phone.state == PhoneState::Ready {
                let one = adb.device(e.serial.clone());
                let props = self.props_of(&one).await;
                if adbp::props_say_emulator(&props) {
                    continue;
                }
                apply_props(&mut phone, &props);
                phone.battery = one
                    .shell("dumpsys battery")
                    .await
                    .ok()
                    .and_then(|t| adbp::parse_battery(&t))
                    .map(|(level, charging)| PhoneBattery { level, charging });
                if details {
                    phone.screen = screen_of(&one).await;
                }
            }
            phones.push(phone);
        }
        Ok(phones)
    }

    async fn props_of(&self, adb: &Adb) -> BTreeMap<String, String> {
        let serial = adb.serial().unwrap_or_default().to_string();
        if let Some(p) = lock(&self.props).get(&serial) {
            return p.clone();
        }
        let props = match adb.shell("getprop").await {
            Ok(text) => adbp::parse_getprop(&text),
            Err(e) => {
                tracing::debug!(%serial, "getprop failed: {e}");
                return BTreeMap::new();
            }
        };
        if !props.is_empty() {
            lock(&self.props).insert(serial, props.clone());
        }
        props
    }

    /// A ready phone and its adb; `not_found` with the reason (and what to
    /// do) otherwise.
    async fn ready(&self, ctx: &CallCtx, selector: &str) -> JidrResult<(Adb, PhysicalDevice)> {
        let phones = self.scan(ctx, false).await?;
        let phone = pick(&phones, selector)?.clone();
        if phone.state != PhoneState::Ready {
            let state = serde_json::to_value(phone.state).unwrap_or(Value::Null);
            return Err(JidrError::not_found(format!(
                "{} isn't ready ({}): {}",
                phone.display_name(),
                state.as_str().unwrap_or("not ready"),
                phone.hint.as_deref().unwrap_or_default()
            ))
            .with_data(json!({ "serial": phone.serial, "state": state })));
        }
        let adb = Adb::new(ctx.daemon.adb_path()?).device(phone.serial.clone());
        Ok((adb, phone))
    }

    /// `physical.record`: `screenrecord` on the phone into
    /// `/data/local/tmp`, then pulled and removed.
    async fn record(&self, ctx: &CallCtx, p: RecordParams) -> JidrResult<Value> {
        let (adb, phone) = self.ready(ctx, &p.device).await?;
        let secs = p.max_seconds.unwrap_or(RECORD_MAX_S).clamp(1, RECORD_MAX_S);
        let path = p.path.unwrap_or_else(|| {
            jidr_core::paths::capture_path(
                &jidr_core::paths::capture_dir("Movies"),
                phone.display_name(),
                "mp4",
            )
        });
        let stop = Arc::new(Notify::new());
        {
            let mut recs = lock(&self.recordings);
            if recs.contains_key(&phone.serial) {
                return Err(JidrError::busy(format!(
                    "{} is already recording",
                    phone.display_name()
                )));
            }
            recs.insert(phone.serial.clone(), stop.clone());
        }
        let _done = Forget(self.recordings.clone(), phone.serial.clone());
        let remote = format!("/data/local/tmp/jidr-record-{}.mp4", std::process::id());
        let limit = secs.to_string();
        let mut child = adb.spawn(&["shell", "screenrecord", "--time-limit", &limit, &remote])?;
        ctx.report(None, "Recording");
        let started = tokio::time::Instant::now();
        let quick = adb.clone().with_timeout(QUICK);
        let cancel = ctx.cancel.clone();
        let cancelled = tokio::select! {
            _ = child.wait() => false,
            _ = stop.notified() => false,
            _ = async move {
                while !cancel.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => true,
        };
        // SIGINT lets screenrecord finish the MP4's index.
        let _ = quick.raw(&["shell", "pkill -2 screenrecord"]).await;
        let _ = tokio::time::timeout(QUICK, child.wait()).await;
        let seconds = started.elapsed().as_secs_f64();
        if cancelled {
            let _ = quick.raw(&["shell", &format!("rm -f {remote}")]).await;
            return Err(jidr_core::cancel::cancelled());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        ctx.report(None, "Saving the recording");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| JidrError::unexpected(format!("{}: {e}", dir.display())))?;
        }
        let pulled = adb.clone().with_timeout(SLOW).pull(&remote, &path).await;
        let _ = quick.raw(&["shell", &format!("rm -f {remote}")]).await;
        pulled?;
        Ok(json!({ "path": path, "seconds": (seconds * 10.0).round() / 10.0 }))
    }

    /// Starts the `track-devices` watcher once, on the first call.
    fn watch(&self, ctx: &CallCtx) {
        {
            let mut w = self.watching.lock().unwrap_or_else(|e| e.into_inner());
            if *w {
                return;
            }
            *w = true;
        }
        let daemon = ctx.daemon.clone();
        let props = self.props.clone();
        tokio::spawn(async move {
            let mut known: BTreeMap<String, PhoneState> = BTreeMap::new();
            loop {
                let Ok(path) = daemon.adb_path() else {
                    tokio::time::sleep(RETRACK * 5).await;
                    continue;
                };
                let adb = Adb::new(path).with_timeout(QUICK);
                match adb.track_devices() {
                    Ok(mut stream) => {
                        while let Some(frame) = stream.frames.recv().await {
                            publish(daemon.as_ref(), &props, &mut known, &frame);
                        }
                    }
                    Err(e) => tracing::debug!("adb track-devices: {e}"),
                }
                // track-devices ended (adb server restarted, or an adb that
                // lacks it): catch up with one listing, then try again.
                if let Ok(entries) = adb.devices_long().await {
                    publish(daemon.as_ref(), &props, &mut known, &entries);
                }
                tokio::time::sleep(RETRACK).await;
            }
        });
    }
}

/// Removes a finished recording from the map, however `record` ends.
struct Forget(Recordings, String);

impl Drop for Forget {
    fn drop(&mut self) {
        lock(&self.0).remove(&self.1);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Emits events for what changed between `known` and a new listing.
pub(crate) fn publish(
    daemon: &dyn DaemonContext,
    props: &Mutex<HashMap<String, BTreeMap<String, String>>>,
    known: &mut BTreeMap<String, PhoneState>,
    entries: &[LongEntry],
) {
    let now: BTreeMap<String, PhoneState> = entries
        .iter()
        .filter(|e| !adbp::is_emulator_serial(&e.serial))
        .map(|e| (e.serial.clone(), PhoneState::from_adb(&e.state)))
        .collect();
    for (serial, change) in diff(known, &now) {
        match change {
            Change::Connected(state) => daemon.emit(
                "physical.connected",
                json!({ "serial": serial, "state": state }),
            ),
            Change::Changed(state) => {
                lock(props).remove(&serial);
                daemon.emit(
                    "physical.changed",
                    json!({ "serial": serial, "state": state }),
                );
            }
            Change::Disconnected => {
                lock(props).remove(&serial);
                daemon.emit("physical.disconnected", json!({ "serial": serial }));
            }
        }
    }
    *known = now;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    Connected(PhoneState),
    Changed(PhoneState),
    Disconnected,
}

pub(crate) fn diff(
    before: &BTreeMap<String, PhoneState>,
    after: &BTreeMap<String, PhoneState>,
) -> Vec<(String, Change)> {
    let mut out = Vec::new();
    for (serial, state) in after {
        match before.get(serial) {
            None => out.push((serial.clone(), Change::Connected(*state))),
            Some(old) if old != state => out.push((serial.clone(), Change::Changed(*state))),
            Some(_) => {}
        }
    }
    for serial in before.keys() {
        if !after.contains_key(serial) {
            out.push((serial.clone(), Change::Disconnected));
        }
    }
    out
}

/// `selector` when it is the serial of a ready phone (not an emulator):
/// `logcat.follow` takes phones too.
pub async fn ready_serial(adb: &Adb, selector: &str) -> Option<String> {
    let entries = adb.clone().with_timeout(QUICK).devices_long().await.ok()?;
    entries
        .into_iter()
        .find(|e| {
            e.serial == selector
                && !adbp::is_emulator_serial(&e.serial)
                && PhoneState::from_adb(&e.state) == PhoneState::Ready
        })
        .map(|e| e.serial)
}

/// adb serials of Jidr's running devices (their `adb connect` TCP ones too).
async fn emulator_serials(daemon: &dyn DaemonContext) -> HashSet<String> {
    let mut out = HashSet::new();
    for d in daemon.devices().await {
        if d.status.is_active()
            && let Ok(info) = daemon.connect_info(d.id).await
        {
            out.insert(info.adb_serial);
        }
    }
    out
}

/// What `devices -l` alone tells about a phone.
pub(crate) fn from_entry(e: &LongEntry) -> PhysicalDevice {
    let state = PhoneState::from_adb(&e.state);
    PhysicalDevice {
        serial: e.serial.clone(),
        state,
        connection: if adbp::is_wifi_serial(&e.serial) {
            PhoneConnection::Wifi
        } else {
            PhoneConnection::Usb
        },
        model: e.field("model").map(|m| m.replace('_', " ")),
        manufacturer: None,
        product: e.field("product").map(str::to_string),
        android: None,
        api: None,
        abi: None,
        battery: None,
        screen: None,
        usb: e.field("usb").map(str::to_string),
        hint: state.hint().map(str::to_string),
    }
}

pub(crate) fn apply_props(phone: &mut PhysicalDevice, props: &BTreeMap<String, String>) {
    let get = |k: &str| props.get(k).filter(|v| !v.is_empty()).cloned();
    if let Some(m) = get("ro.product.model") {
        phone.model = Some(m);
    }
    phone.manufacturer = get("ro.product.manufacturer").or_else(|| get("ro.product.brand"));
    phone.product = get("ro.product.name").or(phone.product.take());
    phone.android = get("ro.build.version.release");
    phone.api = get("ro.build.version.sdk").and_then(|v| v.parse().ok());
    phone.abi = get("ro.product.cpu.abi");
}

async fn screen_of(adb: &Adb) -> Option<PhoneScreen> {
    let size = adb.shell("wm size").await.ok()?;
    let (width, height) = parse_wm_size(&size)?;
    let density = adb
        .shell("wm density")
        .await
        .ok()
        .and_then(|t| parse_wm_density(&t));
    Some(PhoneScreen {
        width,
        height,
        density,
    })
}

/// `Physical size: 1080x2400` / `Override size: 720x1600` (override wins).
pub fn parse_wm_size(text: &str) -> Option<(u32, u32)> {
    let pick = |label: &str| {
        text.lines().find_map(|l| {
            let rest = l.trim().strip_prefix(label)?.trim();
            let (w, h) = rest.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
    };
    pick("Override size:").or_else(|| pick("Physical size:"))
}

/// `Physical density: 420` / `Override density: 480`.
pub fn parse_wm_density(text: &str) -> Option<u32> {
    let pick = |label: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(label)?.trim().parse().ok())
    };
    pick("Override density:").or_else(|| pick("Physical density:"))
}

/// A phone by serial, or by model when exactly one phone has it.
pub(crate) fn pick<'a>(
    phones: &'a [PhysicalDevice],
    selector: &str,
) -> JidrResult<&'a PhysicalDevice> {
    if let Some(p) = phones.iter().find(|p| p.serial == selector) {
        return Ok(p);
    }
    let norm = |s: &str| s.trim().replace('_', " ").to_lowercase();
    let want = norm(selector);
    let by_model: Vec<_> = phones
        .iter()
        .filter(|p| p.model.as_deref().is_some_and(|m| norm(m) == want))
        .collect();
    match by_model.as_slice() {
        [one] => Ok(one),
        [] => Err(JidrError::not_found(format!(
            "no connected phone {selector:?}; `jidr phone list` shows them (plug the phone in with USB debugging on)"
        ))),
        many => Err(JidrError::usage(format!(
            "{} phones are {selector:?}; name one by serial: {}",
            many.len(),
            many.iter()
                .map(|p| p.serial.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn write_file(path: &std::path::Path, bytes: &[u8]) -> JidrResult<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| JidrError::unexpected(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(path, bytes)
        .map_err(|e| JidrError::unexpected(format!("{}: {e}", path.display())))
}

/// Width and height from a PNG's IHDR chunk.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.get(..8)? != b"\x89PNG\r\n\x1a\n" || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(serial: &str, model: Option<&str>) -> PhysicalDevice {
        let mut p = from_entry(&LongEntry {
            serial: serial.into(),
            state: "device".into(),
            fields: BTreeMap::new(),
        });
        p.model = model.map(str::to_string);
        p
    }

    #[test]
    fn phones_are_picked_by_serial_or_a_unique_model() {
        let phones = vec![
            phone("A1", Some("Pixel 8")),
            phone("B2", Some("Galaxy S23")),
            phone("C3", Some("Galaxy S23")),
        ];
        assert_eq!(pick(&phones, "A1").unwrap().serial, "A1");
        assert_eq!(pick(&phones, "pixel_8").unwrap().serial, "A1");
        let e = pick(&phones, "galaxy s23").unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::Usage);
        assert!(e.message.contains("B2, C3"), "{e}");
        let e = pick(&phones, "nope").unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::NotFound);
    }

    #[test]
    fn listings_diff_into_events() {
        let mut before = BTreeMap::new();
        before.insert("A".to_string(), PhoneState::Unauthorized);
        before.insert("B".to_string(), PhoneState::Ready);
        let mut after = BTreeMap::new();
        after.insert("A".to_string(), PhoneState::Ready);
        after.insert("C".to_string(), PhoneState::Offline);
        assert_eq!(
            diff(&before, &after),
            vec![
                ("A".into(), Change::Changed(PhoneState::Ready)),
                ("C".into(), Change::Connected(PhoneState::Offline)),
                ("B".into(), Change::Disconnected),
            ]
        );
    }

    #[test]
    fn props_fill_the_phone() {
        let mut p = phone("A1", Some("Pixel_8"));
        let props: BTreeMap<String, String> = [
            ("ro.product.model", "Pixel 8"),
            ("ro.product.manufacturer", "Google"),
            ("ro.build.version.release", "14"),
            ("ro.build.version.sdk", "34"),
            ("ro.product.cpu.abi", "arm64-v8a"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        apply_props(&mut p, &props);
        assert_eq!(p.model.as_deref(), Some("Pixel 8"));
        assert_eq!(p.manufacturer.as_deref(), Some("Google"));
        assert_eq!(p.api, Some(34));
        assert_eq!(p.android.as_deref(), Some("14"));
    }

    #[test]
    fn wm_output() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600"),
            Some((720, 1600))
        );
        assert_eq!(parse_wm_density("Physical density: 420\n"), Some(420));
        assert_eq!(
            parse_wm_density("Physical density: 420\nOverride density: 480"),
            Some(480)
        );
    }
}
