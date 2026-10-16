//! Root UX (Phase 9e, Stream C): the Root setup sheet that follows a
//! `root.set` job step by step, its failure and revert UI, the Play Store
//! guided tap (ADR-0002) and the dev-tools root section with the opt-in
//! "auto-grant all" toggle (D22). The Slint side is `root.slint`; B's code
//! calls [`wire`], [`sync`] and [`start`] (gap B8).

pub mod goldens;
pub mod props;
pub mod steps;

pub use goldens::cases as golden_cases;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use jidr_core::{DeviceStatus, ErrorCode, JidrError, JidrResult, Job, RootHealth, RootMode};
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use uuid::Uuid;

use crate::app::{Ctx, report, toast};
use crate::model::{DeviceInfo, ago, root_labels};
use crate::{RootLogLine, RootSheetState, RootStep, RootStepState, RootStepVm, RootUi};
use steps::{Failure, LogEntry, STEPS};

/// The sheet's job, from the first progress message to the result.
#[derive(Debug, Default)]
struct Sheet {
    device: Option<Uuid>,
    job: Option<String>,
    step: usize,
    started: Option<Instant>,
    step_started: Option<Instant>,
    /// Play Store image still on its first boot: `root.set` runs once it's up.
    pending: bool,
    /// The guided tap started at.
    grant_since: Option<Instant>,
    /// "Stop and revert" cancelled the job: revert once it ends.
    revert_after_cancel: bool,
    log: Vec<LogEntry>,
    outcome: Option<Outcome>,
}

#[derive(Debug, Clone)]
enum Outcome {
    Rooted {
        secs: u64,
        magisk: Option<String>,
        uid: Option<i64>,
    },
    Failed {
        failure: Failure,
        secs: u64,
        detail: String,
    },
}

thread_local! {
    static SHEET: RefCell<Sheet> = RefCell::default();
    /// The last failed root job per device: the device only stores
    /// `health: failed`, the reason lives here (and in the job).
    static FAILED: RefCell<HashMap<Uuid, Failure>> = RefCell::default();
    /// Ticks the running step's clock while the sheet is open.
    static TICK: Timer = Timer::default();
    static AUTO_GRANT: Cell<bool> = const { Cell::new(false) };
    /// `start` calls for devices the UI hasn't seen yet (just created).
    static QUEUED: RefCell<Vec<(Uuid, RootMode)>> = RefCell::default();
}

/// Hooks the sheet and dev-tools callbacks up. Called once from `app::run`.
pub fn wire(ctx: &Rc<Ctx>) {
    let ui = ctx.app.global::<RootUi>();
    let c = ctx.clone();
    ui.on_action(move |a| sheet_action(&c, a.as_str()));
    let c = ctx.clone();
    ui.on_tools_action(move |a, v| tools_action(&c, a.as_str(), v.as_str()));

    props::wire(ctx);

    // G15 and 9f: `root.auto_grant` and `device.props.set` arrive with a
    // newer daemon; find out whether this one has them.
    let w = ctx.app.as_weak();
    ctx.bridge.call("commands.list", json!({}), move |r| {
        let list = r.ok();
        let has = list
            .as_ref()
            .is_some_and(|v| lists_command(v, "root.auto_grant"));
        let props = list
            .as_ref()
            .is_some_and(|v| lists_command(v, "device.props.set"));
        let _ = slint::invoke_from_event_loop(move || {
            AUTO_GRANT.with(|g| g.set(has));
            if let Some(app) = w.upgrade() {
                let ui = app.global::<RootUi>();
                ui.set_tools_auto_grant_supported(has);
                ui.set_props_supported(props);
            }
            if let Some(c) = crate::app::ctx() {
                sync(&c);
            }
        });
    });
}

/// Whether a `commands.list` result has `id`.
fn lists_command(v: &Value, id: &str) -> bool {
    let list = v.get("commands").unwrap_or(v);
    list.as_array().is_some_and(|a| {
        a.iter()
            .any(|c| c.get("id").and_then(Value::as_str) == Some(id))
    })
}

/// Sets `device` to `mode`. Full root opens the Root sheet and follows the
/// job; None and Shell run in the background with a toast.
pub fn start(ctx: &Rc<Ctx>, device: Uuid, mode: RootMode) {
    if ctx.info(device).is_none() {
        // Just created: `sync` starts it once `device.updated` brings it in.
        QUEUED.with(|q| q.borrow_mut().push((device, mode)));
        return;
    }
    match mode {
        RootMode::Magisk => full_root(ctx, device),
        RootMode::None | RootMode::Adb => {
            let name = name_of(ctx, device);
            let label = if mode == RootMode::Adb {
                "Shell root (adb)"
            } else {
                "unrooted"
            };
            ctx.bridge.run(
                "root.set",
                json!({ "device": device, "mode": mode_name(mode) }),
                |_| {},
                report(&ctx.app.as_weak(), format!("{name} is now {label}")),
            );
        }
    }
}

/// [`start`] from an IO-thread callback (a `device.create` / `device.edit`
/// result): hops to the UI thread first.
pub fn start_later(device: Uuid, mode: RootMode) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ctx) = crate::app::ctx() {
            start(&ctx, device, mode);
        }
    });
}

/// The form's root choice as a mode.
pub fn mode_of(choice: crate::RootChoice) -> RootMode {
    match choice {
        crate::RootChoice::None => RootMode::None,
        crate::RootChoice::Shell => RootMode::Adb,
        crate::RootChoice::Full => RootMode::Magisk,
    }
}

fn mode_name(mode: RootMode) -> &'static str {
    match mode {
        RootMode::None => "none",
        RootMode::Adb => "adb",
        RootMode::Magisk => "magisk",
    }
}

fn name_of(ctx: &Ctx, id: Uuid) -> String {
    ctx.info(id).map(|d| d.device.name).unwrap_or_default()
}

fn play_store(info: &DeviceInfo) -> bool {
    !info.device.image.allows_adb_root()
}

/// Full root: open the sheet and run `root.set magisk`. A Play Store image
/// on its first boot waits for Android first (Root.dc.html, beta frame).
fn full_root(ctx: &Rc<Ctx>, device: Uuid) {
    let Some(info) = ctx.info(device) else { return };
    FAILED.with(|f| f.borrow_mut().remove(&device));
    let now = Instant::now();
    let first_boot = play_store(&info) && !matches!(info.device.status, DeviceStatus::Running);
    SHEET.with(|s| {
        *s.borrow_mut() = Sheet {
            device: Some(device),
            started: Some(now),
            step_started: Some(now),
            pending: first_boot,
            ..Sheet::default()
        };
    });
    ctx.app.global::<RootUi>().set_show_steps(false);
    ctx.app.global::<RootUi>().set_open(true);
    start_tick();
    if first_boot {
        if matches!(info.device.status, DeviceStatus::Stopped) {
            ctx.bridge.call(
                "device.start",
                json!({ "device": device }),
                report(&ctx.app.as_weak(), ""),
            );
        }
        render(ctx);
        return;
    }
    run_set(ctx, device);
}

fn run_set(ctx: &Rc<Ctx>, device: Uuid) {
    render(ctx);
    ctx.bridge.run(
        "root.set",
        json!({ "device": device, "mode": "magisk" }),
        |job: &Job| {
            let job = job.clone();
            let _ = slint::invoke_from_event_loop(move || progress(&job));
        },
        move |r: JidrResult<Value>| {
            let _ = slint::invoke_from_event_loop(move || finished(device, r));
        },
    );
}

/// A `job.progress` of the sheet's job.
fn progress(job: &Job) {
    let Some(ctx) = crate::app::ctx() else { return };
    SHEET.with(|s| {
        let mut s = s.borrow_mut();
        if s.device != job.device || s.outcome.is_some() {
            return;
        }
        s.job = Some(job.id.to_string());
        let msg = job.message.trim();
        if msg.is_empty() || s.log.last().is_some_and(|l| l.text == msg) {
            return;
        }
        s.log.push(entry("", msg));
        let next = steps::step_for(msg, s.step);
        if next != s.step {
            s.step = next;
            s.step_started = Some(Instant::now());
        }
        if steps::is_grant_prompt(msg) && s.grant_since.is_none() {
            s.grant_since = Some(Instant::now());
        }
    });
    render(&ctx);
}

fn entry(level: &'static str, text: &str) -> LogEntry {
    LogEntry {
        time: chrono::Local::now().format("%H:%M:%S").to_string(),
        source: "jidr",
        level,
        text: text.to_string(),
    }
}

/// The sheet's job ended.
fn finished(device: Uuid, r: JidrResult<Value>) {
    let Some(ctx) = crate::app::ctx() else { return };
    let name = name_of(&ctx, device);
    let mut revert = false;
    let outcome = SHEET.with(|s| {
        let mut s = s.borrow_mut();
        let secs = s.started.map_or(0, |t| t.elapsed().as_secs());
        let outcome = match r {
            Ok(v) => Outcome::Rooted {
                secs,
                magisk: v
                    .get("magisk_version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                uid: v.pointer("/verification/uid").and_then(Value::as_i64),
            },
            Err(e) => {
                let mut failure = steps::failure(&e, s.step);
                if failure.cancelled && s.grant_since.is_some() {
                    // Cancelling the guided tap leaves root unverified.
                    failure.unverified = true;
                }
                revert = failure.cancelled && s.revert_after_cancel;
                s.log.push(entry("e", &short_error(&e)));
                s.log.push(entry(
                    "",
                    &format!("root setup stopped at step {} of {STEPS}", failure.step + 1),
                ));
                let detail = detail(&name, &e);
                Outcome::Failed {
                    failure,
                    secs,
                    detail,
                }
            }
        };
        if s.device == Some(device) {
            s.outcome = Some(outcome.clone());
        }
        outcome
    });
    let open = ctx.app.global::<RootUi>().get_open();
    match &outcome {
        Outcome::Rooted { magisk, .. } => {
            FAILED.with(|f| f.borrow_mut().remove(&device));
            if !open {
                let text = match magisk {
                    Some(v) => format!("{name} is rooted · Magisk {v}"),
                    None => format!("{name} is rooted"),
                };
                toast(&ctx.app.as_weak(), text);
            }
        }
        Outcome::Failed { failure, .. } => {
            FAILED.with(|f| f.borrow_mut().insert(device, failure.clone()));
            if revert {
                ctx.app.global::<RootUi>().set_open(false);
                start(&ctx, device, RootMode::None);
            } else if !open {
                toast(
                    &ctx.app.as_weak(),
                    format!(
                        "Root failed: {}. Open {name} to revert or try again.",
                        failure.reason
                    ),
                );
            }
        }
    }
    render(&ctx);
    sync(&ctx);
}

/// The daemon's error, one line, for the log tail.
fn short_error(e: &JidrError) -> String {
    let code = serde_json::to_value(e.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let first = e.message.lines().next().unwrap_or_default();
    if code.is_empty() {
        first.to_string()
    } else {
        format!("{code}: {first}")
    }
}

/// The failure explanation when the error says more than the step does:
/// a boot timeout names how long Jidr waited.
fn detail(name: &str, e: &JidrError) -> String {
    if e.code == ErrorCode::BootTimeout {
        let waited = e
            .message
            .split("within ")
            .nth(1)
            .and_then(|r| r.split(" s").next())
            .and_then(|n| n.trim().parse::<u64>().ok());
        return match waited {
            Some(n) => format!(
                "{name} didn't finish booting after the reboot (waited {n} s). Your data is intact and the original boot image is kept, so you can revert."
            ),
            None => String::new(),
        };
    }
    String::new()
}

fn start_tick() {
    TICK.with(|t| {
        if !t.running() {
            t.start(TimerMode::Repeated, Duration::from_secs(1), || {
                let Some(ctx) = crate::app::ctx() else { return };
                if ctx.app.global::<RootUi>().get_open() {
                    render(&ctx);
                } else {
                    TICK.with(Timer::stop);
                }
            });
        }
    });
}

/// Fills `RootUi` from the sheet's job.
fn render(ctx: &Rc<Ctx>) {
    let ui = ctx.app.global::<RootUi>();
    SHEET.with(|s| {
        let s = s.borrow();
        let Some(device) = s.device else { return };
        let info = ctx.info(device);
        let vm = sheet_vm(&s, info.as_ref(), Instant::now());
        apply(&ui, &vm);
    });
}

/// Everything the sheet shows, computed from the job's state (pure, so
/// goldens and tests can build it from fixtures).
#[derive(Debug, Clone, PartialEq)]
pub struct SheetVm {
    pub state: RootSheetState,
    pub device_name: String,
    pub serial: String,
    pub play_store: bool,
    pub magisk: String,
    pub steps: Vec<RootStepVm>,
    pub step_index: i32,
    pub reason: String,
    pub detail: String,
    pub log: Vec<RootLogLine>,
    pub su_output: String,
    pub took: String,
    pub grant_left: String,
}

fn sheet_vm(s: &Sheet, info: Option<&DeviceInfo>, now: Instant) -> SheetVm {
    let since = |t: Option<Instant>| t.map_or(0, |t| now.saturating_duration_since(t).as_secs());
    let play = info.is_some_and(play_store);
    let magisk = info
        .and_then(|i| i.device.root.magisk_version.clone())
        .unwrap_or_else(|| "30.7".into());
    let mut vm = SheetVm {
        state: RootSheetState::Progress,
        device_name: info.map(|i| i.device.name.clone()).unwrap_or_default(),
        serial: info
            .and_then(|i| i.device.runtime.as_ref())
            .map(|r| r.adb_serial.clone())
            .unwrap_or_default(),
        play_store: play,
        magisk,
        steps: Vec::new(),
        step_index: i32::try_from(s.step + 1).unwrap_or(1),
        reason: String::new(),
        detail: String::new(),
        log: Vec::new(),
        su_output: "uid=0(root) gid=0(root)".into(),
        took: String::new(),
        grant_left: String::new(),
    };
    match &s.outcome {
        None if s.pending => {
            vm.state = RootSheetState::Waiting;
            let pct = info
                .and_then(|i| i.boot_progress)
                .map_or(0.0, |p| p * 100.0);
            vm.steps = vec![
                step(
                    RootStep::FirstBoot,
                    RootStepState::Now,
                    String::new(),
                    format!("{pct:.0}%"),
                ),
                step(
                    RootStep::All,
                    RootStepState::Waiting,
                    String::new(),
                    String::new(),
                ),
            ];
        }
        None => {
            vm.state = if s.grant_since.is_some() {
                RootSheetState::Grant
            } else {
                RootSheetState::Progress
            };
            vm.grant_left = format!("{} s", 60u64.saturating_sub(since(s.grant_since)));
            vm.steps = rows(s.step, None, &steps::clock(since(s.step_started)));
        }
        Some(Outcome::Rooted { secs, magisk, uid }) => {
            vm.state = RootSheetState::Success;
            if let Some(m) = magisk {
                vm.magisk = m.clone();
            }
            if let Some(u) = uid.filter(|u| *u != 0) {
                vm.su_output = format!("uid={u}");
            }
            vm.took = format!("{secs} s");
            vm.steps = rows(STEPS, None, "");
        }
        Some(Outcome::Failed {
            failure,
            secs,
            detail,
        }) => {
            vm.state = if failure.unverified {
                RootSheetState::Unverified
            } else {
                RootSheetState::Failed
            };
            vm.reason = failure.reason.clone();
            vm.detail = detail.clone();
            vm.step_index = i32::try_from(failure.step + 1).unwrap_or(1);
            let step_secs = since(s.step_started).min(*secs);
            vm.steps = rows(failure.step, Some(failure), &steps::clock(step_secs));
            vm.log = steps::log_tail(&s.log, 4)
                .into_iter()
                .map(log_line)
                .collect();
        }
    }
    vm
}

fn step(step: RootStep, state: RootStepState, time: String, percent: String) -> RootStepVm {
    RootStepVm {
        step,
        state,
        time: time.into(),
        percent: percent.into(),
    }
}

const ORDER: [RootStep; STEPS] = [
    RootStep::Download,
    RootStep::Patch,
    RootStep::Reboot,
    RootStep::Install,
    RootStep::Verify,
];

/// The step rows: done before `at`, running (or failed) at `at`, queued (or
/// not run) after. A failed reboot folds Install and Verify into one row.
fn rows(at: usize, failure: Option<&Failure>, time: &str) -> Vec<RootStepVm> {
    let mut out = Vec::new();
    for (i, s) in ORDER.iter().enumerate() {
        let state = if i < at {
            RootStepState::Done
        } else if i == at {
            if failure.is_some() {
                RootStepState::Failed
            } else {
                RootStepState::Now
            }
        } else if failure.is_some() {
            RootStepState::Skipped
        } else {
            RootStepState::Waiting
        };
        if failure.is_some() && at == steps::REBOOT && i == steps::VERIFY {
            continue;
        }
        let t = if i == at {
            time.to_string()
        } else {
            String::new()
        };
        out.push(step(*s, state, t, String::new()));
    }
    out
}

fn log_line(e: LogEntry) -> RootLogLine {
    RootLogLine {
        time: e.time.into(),
        source: e.source.into(),
        level: e.level.into(),
        text: e.text.into(),
    }
}

fn apply(ui: &RootUi<'_>, vm: &SheetVm) {
    ui.set_state(vm.state);
    ui.set_device_name(vm.device_name.as_str().into());
    ui.set_serial(vm.serial.as_str().into());
    ui.set_play_store(vm.play_store);
    ui.set_magisk_version(vm.magisk.as_str().into());
    ui.set_steps(ModelRc::new(VecModel::from(vm.steps.clone())));
    ui.set_step_index(vm.step_index);
    ui.set_reason(vm.reason.as_str().into());
    ui.set_detail(vm.detail.as_str().into());
    ui.set_log(ModelRc::new(VecModel::from(vm.log.clone())));
    ui.set_su_output(vm.su_output.as_str().into());
    ui.set_took(vm.took.as_str().into());
    ui.set_grant_left(vm.grant_left.as_str().into());
}

fn sheet_device(ctx: &Ctx) -> Option<(Uuid, Option<String>)> {
    let _ = ctx;
    SHEET.with(|s| {
        let s = s.borrow();
        s.device.map(|d| (d, s.job.clone()))
    })
}

fn device_index(ctx: &Ctx, id: Uuid) -> Option<i32> {
    let st = ctx.state.borrow();
    st.data
        .devices
        .iter()
        .position(|d| d.device.id == id)
        .and_then(|i| i32::try_from(i).ok())
}

fn sheet_action(ctx: &Rc<Ctx>, action: &str) {
    let ui = ctx.app.global::<RootUi>();
    let Some((device, job)) = sheet_device(ctx) else {
        ui.set_open(false);
        return;
    };
    match action {
        "background" | "done" => ui.set_open(false),
        "stop" | "cancel-grant" => {
            if action == "stop" {
                SHEET.with(|s| s.borrow_mut().revert_after_cancel = true);
            }
            if let Some(job) = job {
                ctx.bridge.call(
                    "jobs.cancel",
                    json!({ "job": job }),
                    report(&ctx.app.as_weak(), ""),
                );
            }
        }
        "try-again" => full_root(ctx, device),
        "revert" => {
            ui.set_open(false);
            start(ctx, device, RootMode::None);
        }
        "dont-root" => {
            SHEET.with(|s| s.borrow_mut().pending = false);
            ui.set_open(false);
            start(ctx, device, RootMode::None);
        }
        "copy-log" => {
            let text = SHEET.with(|s| {
                s.borrow()
                    .log
                    .iter()
                    .map(|l| {
                        format!(
                            "{}  {} {} {}",
                            l.time,
                            l.source,
                            if l.level.is_empty() { " " } else { l.level },
                            l.text
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            crate::settings::copy_text(&text);
            toast(&ctx.app.as_weak(), "Copied the root setup log".into());
        }
        "save-snapshot" => {
            ui.set_open(false);
            if let Some(i) = device_index(ctx, device) {
                ctx.app.set_selected(i);
            }
            ctx.app.invoke_snapshot_save_request();
        }
        _ => {}
    }
}

fn tools_action(ctx: &Rc<Ctx>, action: &str, value: &str) {
    let Some(device) = crate::player::active_id(ctx) else {
        return;
    };
    match action {
        "verify" => {
            if let Some(i) = device_index(ctx, device) {
                ctx.app.invoke_verify_root(i);
            }
        }
        "revert" => {
            if let Some(i) = device_index(ctx, device) {
                ctx.app.invoke_revert_root(i);
            }
        }
        "set-up" => full_root(ctx, device),
        "auto-grant" => {
            let enabled = value == "true";
            let w = ctx.app.as_weak();
            ctx.bridge.call(
                "root.auto_grant",
                json!({ "device": device, "enabled": enabled }),
                move |r| {
                    toast(
                        &w,
                        match r {
                            Ok(_) if enabled => "Apps now get root without asking".into(),
                            Ok(_) => "Apps ask before they get root".into(),
                            Err(e) => e.message,
                        },
                    );
                    // Show the stored flag again (it stays put on an error).
                    let _ = slint::invoke_from_event_loop(|| {
                        if let Some(c) = crate::app::ctx() {
                            sync(&c);
                        }
                    });
                },
            );
        }
        _ => {}
    }
}

/// Re-reads the device list: starts a pending Play Store setup once Android
/// is up, and fills the dev-tools section for the active tab. Called from
/// `Ctx::rerender`.
pub fn sync(ctx: &Rc<Ctx>) {
    let ready: Vec<(Uuid, RootMode)> = QUEUED.with(|q| {
        let mut q = q.borrow_mut();
        let (ready, wait) = q.drain(..).partition(|(d, _)| ctx.info(*d).is_some());
        *q = wait;
        ready
    });
    for (device, mode) in ready {
        start(ctx, device, mode);
    }
    let pending = SHEET.with(|s| {
        let s = s.borrow();
        s.device.filter(|_| s.pending && s.outcome.is_none())
    });
    if let Some(device) = pending {
        let running = ctx
            .info(device)
            .is_some_and(|i| matches!(i.device.status, DeviceStatus::Running));
        if running {
            SHEET.with(|s| s.borrow_mut().pending = false);
            run_set(ctx, device);
        } else if ctx.app.global::<RootUi>().get_open() {
            render(ctx);
        }
    }
    let ui = ctx.app.global::<RootUi>();
    let info = crate::player::active_id(ctx).and_then(|id| ctx.info(id));
    let failure = info
        .as_ref()
        .and_then(|i| FAILED.with(|f| f.borrow().get(&i.device.id).cloned()));
    let vm = tools_vm(info.as_ref(), failure.as_ref(), chrono::Utc::now());
    ui.set_tools_visible(vm.visible);
    ui.set_tools_kind(vm.kind);
    ui.set_tools_badge(vm.badge);
    ui.set_tools_beta(vm.beta);
    ui.set_tools_full(vm.full);
    ui.set_tools_running(vm.running);
    ui.set_tools_busy(vm.busy);
    ui.set_tools_verified(vm.verified);
    ui.set_tools_failed(vm.failed);
    ui.set_tools_auto_grant(vm.auto_grant);
    ui.set_tools_auto_grant_supported(AUTO_GRANT.with(Cell::get));
    props::sync(ctx, info.as_ref());
}

/// The dev-tools root section for one device.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolsVm {
    pub visible: bool,
    pub kind: crate::RootKind,
    pub badge: SharedString,
    pub beta: bool,
    pub full: bool,
    pub running: bool,
    pub busy: bool,
    pub verified: SharedString,
    pub failed: bool,
    pub auto_grant: bool,
}

pub fn tools_vm(
    info: Option<&DeviceInfo>,
    failure: Option<&Failure>,
    now: chrono::DateTime<chrono::Utc>,
) -> ToolsVm {
    let Some(info) = info else {
        return ToolsVm {
            visible: false,
            kind: crate::RootKind::None,
            badge: SharedString::new(),
            beta: false,
            full: false,
            running: false,
            busy: false,
            verified: SharedString::new(),
            failed: false,
            auto_grant: false,
        };
    };
    let root = &info.device.root;
    let (kind, mut badge, _) = root_labels(info);
    if let Some(f) = failure.filter(|_| root.health == RootHealth::Failed && !info.root_job) {
        badge = format!("Root failed: {}", f.reason);
    }
    let full = root.mode == RootMode::Magisk;
    let verified = match root.verified_at {
        Some(t) if root.health == RootHealth::Ok => format!("{} · su -c id → uid=0", ago(t, now)),
        _ => String::new(),
    };
    ToolsVm {
        visible: true,
        kind,
        badge: badge.into(),
        beta: full && play_store(info),
        full,
        running: matches!(info.device.status, DeviceStatus::Running),
        busy: info.root_job || info.busy_holder.is_some(),
        verified: verified.into(),
        failed: full && root.health == RootHealth::Failed,
        auto_grant: root.auto_grant_all,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn pixel() -> DeviceInfo {
        fixtures::device_infos().remove(0)
    }

    #[test]
    fn progress_rows_follow_the_step() {
        let r = rows(steps::REBOOT, None, "0:18");
        let states: Vec<_> = r.iter().map(|s| s.state).collect();
        assert_eq!(
            states,
            [
                RootStepState::Done,
                RootStepState::Done,
                RootStepState::Now,
                RootStepState::Waiting,
                RootStepState::Waiting
            ]
        );
        assert_eq!(r[2].time, "0:18");
    }

    #[test]
    fn a_failed_reboot_folds_the_rest_into_one_row() {
        let f = steps::failure(
            &JidrError::new(ErrorCode::BootTimeout, "x did not boot within 120 s"),
            steps::REBOOT,
        );
        let r = rows(steps::REBOOT, Some(&f), "2:00");
        assert_eq!(r.len(), 4);
        assert_eq!(r[2].state, RootStepState::Failed);
        assert_eq!(
            (r[3].step, r[3].state),
            (RootStep::Install, RootStepState::Skipped)
        );
    }

    #[test]
    fn sheet_states() {
        let info = pixel();
        let t0 = Instant::now();
        let mut s = Sheet {
            device: Some(info.device.id),
            started: Some(t0),
            step_started: Some(t0),
            step: steps::REBOOT,
            ..Sheet::default()
        };
        let now = t0 + Duration::from_secs(18);
        let vm = sheet_vm(&s, Some(&info), now);
        assert_eq!(vm.state, RootSheetState::Progress);
        assert_eq!(vm.step_index, 3);
        assert_eq!(vm.steps[2].time, "0:18");
        assert_eq!(vm.serial, "emulator-5556");

        s.grant_since = Some(t0);
        let vm = sheet_vm(&s, Some(&info), now);
        assert_eq!(
            (vm.state, vm.grant_left.as_str()),
            (RootSheetState::Grant, "42 s")
        );

        s.outcome = Some(Outcome::Rooted {
            secs: 58,
            magisk: Some("30.7".into()),
            uid: Some(0),
        });
        let vm = sheet_vm(&s, Some(&info), now);
        assert_eq!(
            (vm.state, vm.took.as_str()),
            (RootSheetState::Success, "58 s")
        );
        assert!(vm.steps.iter().all(|r| r.state == RootStepState::Done));

        let e = JidrError::new(
            ErrorCode::BootTimeout,
            "Pixel did not boot within 120 s (boot); see log",
        );
        s.outcome = Some(Outcome::Failed {
            failure: steps::failure(&e, steps::REBOOT),
            secs: 140,
            detail: detail("Pixel-ish 6.1", &e),
        });
        let vm = sheet_vm(&s, Some(&info), now);
        assert_eq!(vm.state, RootSheetState::Failed);
        assert_eq!(vm.reason, "boot timed out");
        assert!(vm.detail.contains("waited 120 s"), "{}", vm.detail);

        let grant = JidrError::new(ErrorCode::RootFailed, "denied")
            .with_data(json!({ "state": "awaiting_user_grant" }));
        s.outcome = Some(Outcome::Failed {
            failure: steps::failure(&grant, steps::VERIFY),
            secs: 90,
            detail: String::new(),
        });
        assert_eq!(
            sheet_vm(&s, Some(&info), now).state,
            RootSheetState::Unverified
        );
    }

    #[test]
    fn a_play_store_first_boot_waits() {
        let info = fixtures::device_infos().remove(2);
        assert!(play_store(&info));
        let s = Sheet {
            device: Some(info.device.id),
            pending: true,
            ..Sheet::default()
        };
        let vm = sheet_vm(&s, Some(&info), Instant::now());
        assert_eq!(vm.state, RootSheetState::Waiting);
        assert_eq!(vm.steps[0].percent, "62%");
        assert_eq!(vm.steps[1].step, RootStep::All);
    }

    #[test]
    fn dev_tools_show_the_failure_reason() {
        let mut info = pixel();
        let vm = tools_vm(Some(&info), None, fixtures::now());
        assert!(vm.full && vm.visible && vm.running);
        assert_eq!(vm.verified, "2 min ago · su -c id → uid=0");
        info.device.root.health = RootHealth::Failed;
        let f = steps::failure(&JidrError::new(ErrorCode::BootTimeout, "x"), steps::REBOOT);
        let vm = tools_vm(Some(&info), Some(&f), fixtures::now());
        assert_eq!(vm.badge, "Root failed: boot timed out");
        assert!(!tools_vm(None, None, fixtures::now()).visible);
    }

    #[test]
    fn commands_list_shapes() {
        let v = json!([{ "id": "root.set" }, { "id": "root.auto_grant" }]);
        assert!(lists_command(&v, "root.auto_grant"));
        assert!(!lists_command(
            &json!({ "commands": [{ "id": "root.set" }] }),
            "root.auto_grant"
        ));
    }
}
