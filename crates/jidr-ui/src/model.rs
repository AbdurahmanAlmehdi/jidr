//! Daemon data → view models. Every user-facing label is built here, from
//! the status vocabulary in docs/design/PRODUCT.md §5 and the number rules
//! in docs/design/system/README.md, so the Slint files only lay things out.

use chrono::{DateTime, FixedOffset, Utc};
use jidr_core::{Device, DeviceStatus, ImageRef, Job, JobState, RootHealth, RootMode};
use serde::Deserialize;
use slint::SharedString;
use uuid::Uuid;

use crate::{
    BudgetLevel, BudgetVm, DevKind, DevState, DeviceVm, ExhaustedRow, FormFactor, JobTone, JobVm,
    RootKind, SnapshotVm, TabVm,
};

/// "Now" and the local time zone, injected so goldens don't depend on the
/// machine's clock or zone.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub now: DateTime<Utc>,
    pub tz: FixedOffset,
}

impl Clock {
    pub fn system() -> Self {
        let now = Utc::now();
        Self {
            now,
            tz: *now.with_timezone(&chrono::Local).offset(),
        }
    }

    pub fn local(&self, t: DateTime<Utc>) -> DateTime<FixedOffset> {
        t.with_timezone(&self.tz)
    }
}

/// Everything the UI knows about one device, gathered from `device.list`,
/// `device.details`, `profile.list`, events and jobs.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub device: Device,
    pub profile_name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub density: u32,
    pub ram_mb: u32,
    pub cpus: u32,
    pub disk_bytes: Option<u64>,
    /// Latest `device.boot` progress while booting.
    pub boot_progress: Option<f32>,
    /// Who holds the device lock when it isn't this window ("CLI", "agent").
    pub busy_holder: Option<String>,
    /// A root job (`root.set`) is running on it.
    pub root_job: bool,
    /// Pool clone: its lease (`pool.list`).
    pub lease: Option<Lease>,
    /// Pool clone: the name of its base.
    pub base_name: Option<String>,
    /// Base: how many of its clones are leased (`base.list`).
    pub base_leases: usize,
}

/// A pool clone's lease, from `pool.list` (D42).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// A fixed lifetime (`ttl_s`); otherwise tied to the agent's connection.
    pub ttl: bool,
    /// When the daemon releases it; `None` while the agent is connected.
    pub expires_at: Option<DateTime<Utc>>,
}

impl DeviceInfo {
    pub fn new(device: Device) -> Self {
        Self {
            device,
            profile_name: None,
            width: 0,
            height: 0,
            density: 0,
            ram_mb: 0,
            cpus: 0,
            disk_bytes: None,
            boot_progress: None,
            busy_holder: None,
            root_job: false,
            lease: None,
            base_name: None,
            base_leases: 0,
        }
    }

    pub fn kind(&self) -> DevKind {
        if self.device.base {
            DevKind::Base
        } else if self.device.pool_base.is_some() {
            DevKind::Pool
        } else {
            DevKind::Device
        }
    }

    /// What the budget expects it to cost: its last measured peak, or its
    /// guest RAM plus the emulator's own ~2 GB (the daemon's cost model).
    pub fn expected_mb(&self) -> u64 {
        match self.device.footprint_mb {
            Some(mb) if mb > 0 => u64::from(mb),
            _ => u64::from(self.ram_mb) + EMULATOR_OVERHEAD_MB,
        }
    }
}

/// The emulator's cost on top of guest RAM (jidr-daemon `budget.rs`).
pub const EMULATOR_OVERHEAD_MB: u64 = 2048;

/// Sidebar order: people's devices, then the agent pool, then bases.
pub fn group_rank(info: &DeviceInfo) -> u8 {
    match info.kind() {
        DevKind::Device => 0,
        DevKind::Pool => 1,
        DevKind::Base => 2,
    }
}

/// "Android 15" for API 35 (marketing names, D23 range plus neighbours).
pub fn android_version(api: u32) -> String {
    let v = match api {
        25 => "7.1",
        26 => "8.0",
        27 => "8.1",
        28 => "9",
        29 => "10",
        30 => "11",
        31 => "12",
        32 => "12L",
        33 => "13",
        34 => "14",
        35 => "15",
        36 => "16",
        _ => return format!("API {api}"),
    };
    format!("Android {v}")
}

/// "Android 15 · API 35".
pub fn android_label(api: u32) -> String {
    format!("{} · API {api}", android_version(api))
}

/// Image tag as users read it: Google APIs / Play Store / AOSP.
pub fn image_tag(image: &ImageRef) -> String {
    match image.tag.as_str() {
        "google_apis" => "Google APIs".into(),
        "google_apis_playstore" | "playstore" => "Play Store".into(),
        "default" => "AOSP".into(),
        "aosp_atd" => "AOSP ATD".into(),
        "google_atd" => "Google ATD".into(),
        other => other.replace('_', " "),
    }
}

/// Decimal GB with one decimal, like Finder ("6.8 GB").
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// RAM in whole binary GB ("4 GB"); under 1 GB in MB.
pub fn ram(mb: u32) -> String {
    if mb >= 1024 && mb.is_multiple_of(1024) {
        format!("{} GB", mb / 1024)
    } else if mb >= 1024 {
        format!("{:.1} GB", f64::from(mb) / 1024.0)
    } else {
        format!("{mb} MB")
    }
}

/// "40 s", "12 m", "2 h 14 m" (number, no-break space, unit).
pub fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}\u{a0}s")
    } else if secs < 3600 {
        format!("{}\u{a0}m", secs / 60)
    } else {
        format!("{}\u{a0}h {}\u{a0}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// "2 min ago", "Today 14:02"-style relative time for verification stamps.
pub fn ago(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let s = (now - then).num_seconds().max(0);
    if s < 60 {
        "just now".into()
    } else if s < 3600 {
        format!("{} min ago", s / 60)
    } else if s < 86_400 {
        format!("{} h ago", s / 3600)
    } else {
        format!("{} days ago", s / 86_400)
    }
}

fn state_of(info: &DeviceInfo) -> DevState {
    if info.busy_holder.is_some() {
        return DevState::Busy;
    }
    match &info.device.status {
        DeviceStatus::Stopped => DevState::Stopped,
        DeviceStatus::Hibernated => DevState::Hibernated,
        DeviceStatus::Starting => DevState::Starting,
        DeviceStatus::Booting => DevState::Booting,
        DeviceStatus::Running => DevState::Running,
        DeviceStatus::Stopping => DevState::Stopping,
        DeviceStatus::Crashed { .. } => DevState::Crashed,
    }
}

/// A pool clone's lease: "1 h 12 m left", "While its agent is connected".
pub fn lease_label(lease: &Lease, now: DateTime<Utc>) -> String {
    match (lease.ttl, lease.expires_at) {
        (_, Some(t)) if t <= now => "Releasing…".into(),
        (true, Some(t)) => format!("{} left", duration((t - now).num_seconds())),
        (false, Some(t)) => format!(
            "Agent gone · released in {}",
            duration((t - now).num_seconds())
        ),
        (_, None) => "While its agent is connected".into(),
    }
}

/// "2 clones leased", "No clones leased".
fn base_label(leases: usize) -> String {
    match leases {
        0 => "No clones leased".into(),
        1 => "1 clone leased".into(),
        n => format!("{n} clones leased"),
    }
}

/// The status line: "Running · 2 h 14 m", "Booting Android… 62%", …
pub fn status_label(info: &DeviceInfo, now: DateTime<Utc>) -> String {
    if let Some(h) = &info.busy_holder {
        return format!("Busy · in use by {h}");
    }
    if info.device.base {
        return format!("Base · {}", base_label(info.base_leases));
    }
    if let (Some(l), DeviceStatus::Running) = (&info.lease, &info.device.status) {
        return format!("Leased · {}", lease_label(l, now));
    }
    match &info.device.status {
        DeviceStatus::Stopped => "Stopped".into(),
        DeviceStatus::Hibernated => "Hibernated".into(),
        DeviceStatus::Starting => "Starting…".into(),
        DeviceStatus::Booting => match info.boot_progress {
            Some(p) => format!("Booting Android… {}%", (p * 100.0).round() as i64),
            None => "Booting Android…".into(),
        },
        DeviceStatus::Running => match &info.device.runtime {
            Some(rt) => format!(
                "Running · {}",
                duration((now - rt.started_at).num_seconds())
            ),
            None => "Running".into(),
        },
        DeviceStatus::Stopping => "Stopping…".into(),
        DeviceStatus::Crashed { .. } => "Crashed".into(),
    }
}

/// Root badge kind and its long and short labels (RootBadge spec).
pub fn root_labels(info: &DeviceInfo) -> (RootKind, String, String) {
    let root = &info.device.root;
    if info.root_job {
        return (
            RootKind::SettingUp,
            "Setting up root…".into(),
            "Setup".into(),
        );
    }
    match root.mode {
        RootMode::None => (RootKind::None, "Not rooted".into(), "No root".into()),
        RootMode::Adb => (RootKind::Shell, "Shell root (adb)".into(), "Shell".into()),
        RootMode::Magisk => match root.health {
            RootHealth::Ok => {
                let long = match &root.magisk_version {
                    Some(v) => format!("Rooted · Magisk {v}"),
                    None => "Rooted".into(),
                };
                (RootKind::Verified, long, "Rooted".into())
            }
            RootHealth::Failed => (RootKind::Failed, "Root failed".into(), "Failed".into()),
            RootHealth::Unverified if root.verified_at.is_some() => (
                RootKind::Unverified,
                "Root unverified".into(),
                "Unverified".into(),
            ),
            RootHealth::Unverified => (RootKind::Full, "Full root".into(), "Full".into()),
        },
    }
}

fn root_mode_name(mode: RootMode) -> &'static str {
    match mode {
        RootMode::None => "Not rooted",
        RootMode::Adb => "Shell root",
        RootMode::Magisk => "Full root",
    }
}

fn form_factor(info: &DeviceInfo) -> FormFactor {
    let short = info.width.min(info.height);
    let long = info.width.max(info.height);
    if info.device.profile.starts_with("tablet") || (short >= 1200 && long < short * 2) {
        FormFactor::Tablet
    } else if short > 0 && short <= 720 {
        FormFactor::SmallPhone
    } else {
        FormFactor::Phone
    }
}

pub fn device_vm(info: &DeviceInfo, clock: &Clock) -> DeviceVm {
    let now = clock.now;
    let d = &info.device;
    let (root, root_label, root_short) = root_labels(info);
    let verified = match d.root.verified_at {
        Some(t) if d.root.mode == RootMode::Magisk => format!("{} · su -c id → uid=0", ago(t, now)),
        Some(t) => format!("{} · adb shell id → uid=0", ago(t, now)),
        None if d.root.mode == RootMode::None => "Not needed".into(),
        None => "Not yet".into(),
    };
    let screen = if info.width > 0 {
        format!("{}×{} · {} dpi", info.width, info.height, info.density)
    } else {
        String::new()
    };
    let play_store = !d.image.allows_adb_root();
    DeviceVm {
        id: d.id.to_string().into(),
        name: d.name.clone().into(),
        profile: info
            .profile_name
            .clone()
            .unwrap_or_else(|| d.profile.clone())
            .into(),
        screen: screen.into(),
        android: android_label(d.image.api).into(),
        image: image_tag(&d.image).into(),
        state: state_of(info),
        status: status_label(info, now).into(),
        progress: info.boot_progress.unwrap_or(0.0),
        root,
        root_label: root_label.into(),
        root_short: root_short.into(),
        beta: play_store && d.root.mode == RootMode::Magisk,
        root_mode: root_mode_name(d.root.mode).into(),
        magisk: d.root.magisk_version.clone().unwrap_or_default().into(),
        verified: verified.into(),
        serial: d
            .runtime
            .as_ref()
            .map(|r| r.adb_serial.clone())
            .unwrap_or_else(|| "—".into())
            .into(),
        disk: info.disk_bytes.map(gb).unwrap_or_else(|| "—".into()).into(),
        ram: if info.ram_mb > 0 {
            ram(info.ram_mb)
        } else {
            "—".into()
        }
        .into(),
        cpus: if info.cpus > 0 {
            format!("{} cores", info.cpus)
        } else {
            "—".into()
        }
        .into(),
        uptime: d
            .runtime
            .as_ref()
            .filter(|_| matches!(d.status, DeviceStatus::Running))
            .map(|r| duration((now - r.started_at).num_seconds()))
            .unwrap_or_else(|| "—".into())
            .into(),
        last_started: d
            .runtime
            .as_ref()
            .map(|r| clock.local(r.started_at).format("Today, %H:%M").to_string())
            .unwrap_or_else(|| "—".into())
            .into(),
        form: form_factor(info),
        width_px: info.width as i32,
        height_px: info.height as i32,
        play_store,
        kind: info.kind(),
        lease: match info.kind() {
            DevKind::Pool => info
                .lease
                .as_ref()
                .map(|l| lease_label(l, now))
                .unwrap_or_default(),
            DevKind::Base => base_label(info.base_leases),
            DevKind::Device => String::new(),
        }
        .into(),
        base_name: info.base_name.clone().unwrap_or_default().into(),
        can_base: info.kind() == DevKind::Device
            && matches!(d.status, DeviceStatus::Running)
            && d.root.mode != RootMode::Magisk
            && !d.root.writable_system
            && info.busy_holder.is_none(),
    }
}

/// One device counted against the budget (`host.budget` `devices`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BudgetUse {
    pub id: Uuid,
    pub name: String,
    pub expected_mb: u64,
    #[serde(default)]
    pub measured_mb: Option<u64>,
}

impl BudgetUse {
    /// The larger of the expectation and the latest measurement (the
    /// daemon's rule).
    pub fn counted_mb(&self) -> u64 {
        self.expected_mb.max(self.measured_mb.unwrap_or(0))
    }
}

/// `host.budget` (D42).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct HostBudget {
    pub budget_mb: u64,
    pub used_mb: u64,
    #[serde(default)]
    pub devices: Vec<BudgetUse>,
}

/// What one more device is expected to cost: the mean over the person's
/// devices (bases and pool clones aside), else a 4 GB phone.
pub fn typical_mb(devices: &[DeviceInfo]) -> u64 {
    let costs: Vec<u64> = devices
        .iter()
        .filter(|d| {
            d.kind() == DevKind::Device && (d.ram_mb > 0 || d.device.footprint_mb.is_some())
        })
        .map(DeviceInfo::expected_mb)
        .collect();
    if costs.is_empty() {
        4096 + EMULATOR_OVERHEAD_MB
    } else {
        costs.iter().sum::<u64>() / costs.len() as u64
    }
}

/// RAM in binary GB with one decimal ("9.0 GB"), for budget numbers.
pub fn ram_gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

/// The launcher meter: how many devices fit, and how full the budget is.
/// Fits while two or more could still start, near with room for one,
/// full when the next start would be refused.
pub fn budget_vm(b: &HostBudget, typical: u64) -> BudgetVm {
    let running = b.devices.len();
    let free = b.budget_mb.saturating_sub(b.used_mb);
    let more = (free / typical.max(1)) as usize;
    BudgetVm {
        known: true,
        running: running as i32,
        fit: (running + more) as i32,
        level: match more {
            0 => BudgetLevel::Full,
            1 => BudgetLevel::Near,
            _ => BudgetLevel::Fits,
        },
        fraction: if b.budget_mb > 0 {
            (b.used_mb as f32 / b.budget_mb as f32).min(1.0)
        } else {
            1.0
        },
        used: ram_gb(b.used_mb).into(),
        budget: ram_gb(b.budget_mb).into(),
    }
}

/// A running device the daemon suggests hibernating (`hibernate_candidates`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct IdleDevice {
    pub id: Uuid,
    pub name: String,
    pub idle_s: u64,
}

/// `resource_exhausted` details from `device.start` (D42).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Exhausted {
    pub needed_mb: u64,
    pub used_mb: u64,
    pub budget_mb: u64,
    #[serde(default)]
    pub devices: Vec<BudgetUse>,
    #[serde(default)]
    pub hibernate_candidates: Vec<IdleDevice>,
}

impl Exhausted {
    pub fn free_mb(&self) -> u64 {
        self.budget_mb.saturating_sub(self.used_mb)
    }

    fn cost_of(&self, id: Uuid) -> u64 {
        self.devices
            .iter()
            .find(|d| d.id == id)
            .map(BudgetUse::counted_mb)
            .unwrap_or(0)
    }

    /// The idle devices to hibernate, longest-idle first, until the start
    /// fits. Empty when hibernating all of them still wouldn't make room.
    pub fn picks(&self) -> Vec<Uuid> {
        let mut free = self.free_mb();
        let mut picked = Vec::new();
        for c in &self.hibernate_candidates {
            if free >= self.needed_mb {
                break;
            }
            free += self.cost_of(c.id);
            picked.push(c.id);
        }
        if free >= self.needed_mb {
            picked
        } else {
            Vec::new()
        }
    }

    /// The dialog's rows: every candidate with its idle time and cost.
    pub fn rows(&self) -> Vec<ExhaustedRow> {
        let picks = self.picks();
        self.hibernate_candidates
            .iter()
            .map(|c| ExhaustedRow {
                name: c.name.clone().into(),
                detail: format!(
                    "Idle {} · {}",
                    duration(i64::try_from(c.idle_s).unwrap_or(i64::MAX)),
                    ram_gb(self.cost_of(c.id))
                )
                .into(),
                picked: picks.contains(&c.id),
            })
            .collect()
    }
}

pub fn tab_vm(vm: &DeviceVm) -> TabVm {
    TabVm {
        id: vm.id.clone(),
        name: vm.name.clone(),
        state: vm.state,
    }
}

/// A job row. `title` comes from the job kind and message.
pub fn job_vm(job: &Job, title: &str, bytes: Option<(u64, u64)>, eta: Option<i64>) -> JobVm {
    let tone = match job.state {
        JobState::Pending => JobTone::Queued,
        JobState::Running => JobTone::Running,
        JobState::Succeeded => JobTone::Done,
        JobState::Failed => JobTone::Failed,
        JobState::Cancelled => JobTone::Cancelled,
    };
    let detail = match (bytes, job.state) {
        (Some((done, total)), _) if total > 0 => {
            format!("{:.1} / {:.1} GB", done as f64 / 1e9, total as f64 / 1e9)
        }
        (_, JobState::Failed) => job
            .error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "Failed".into()),
        (_, JobState::Cancelled) => "Cancelled".into(),
        (_, JobState::Succeeded) => "Done".into(),
        (_, JobState::Pending) => "Queued".into(),
        _ => job.message.clone(),
    };
    let eta = match eta {
        Some(s) if s >= 60 => format!("{} min left", (s + 59) / 60),
        Some(s) => format!("{s} s left"),
        None => String::new(),
    };
    JobVm {
        id: job.id.to_string().into(),
        title: SharedString::from(title),
        tone,
        progress: job.progress.unwrap_or(0.0),
        indeterminate: job.progress.is_none(),
        detail: detail.into(),
        eta: eta.into(),
        cancellable: !job.state.is_terminal(),
    }
}

/// Snapshot row; `refused` when its root mode differs from the device's (D29).
pub fn snapshot_vm(
    s: &jidr_core::Snapshot,
    device_mode: RootMode,
    size: Option<u64>,
    clock: &Clock,
) -> SnapshotVm {
    let auto = s.name == "default_boot";
    let root = match s.root_mode {
        RootMode::None => RootKind::None,
        RootMode::Adb => RootKind::Shell,
        RootMode::Magisk => RootKind::Full,
    };
    SnapshotVm {
        name: if auto {
            "Quick boot".into()
        } else {
            s.name.clone().into()
        },
        tag: format!("Taken with: {}", root_mode_name(s.root_mode)).into(),
        root,
        auto,
        size: size.map(gb).unwrap_or_default().into(),
        when: clock
            .local(s.created_at)
            .format("%b %-d, %H:%M")
            .to_string()
            .into(),
        refused: !auto && s.root_mode != device_mode,
        thumb: Default::default(),
        has_thumb: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use jidr_core::{DeviceRuntime, RootState};

    fn utc(now: DateTime<Utc>) -> Clock {
        Clock {
            now,
            tz: FixedOffset::east_opt(0).unwrap(),
        }
    }

    fn device(status: DeviceStatus) -> Device {
        Device {
            id: uuid::Uuid::nil(),
            name: "Pixel-ish 6.1".into(),
            avd: "pixel".into(),
            profile: "phone-6.1-fhd".into(),
            image: ImageRef {
                api: 35,
                tag: "google_apis".into(),
                abi: "arm64-v8a".into(),
            },
            sdk_root: "/sdk".into(),
            root: RootState::default(),
            agent_owned: false,
            created_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
            launch: Default::default(),
            status,
            runtime: None,
            footprint_mb: None,
            auto_hibernate_minutes: None,
            base: false,
            pool_base: None,
        }
    }

    #[test]
    fn numbers_follow_the_system_rules() {
        assert_eq!(gb(6_800_000_000), "6.8 GB");
        assert_eq!(ram(4096), "4 GB");
        assert_eq!(ram(1536), "1.5 GB");
        assert_eq!(duration(40), "40\u{a0}s");
        assert_eq!(duration(12 * 60), "12\u{a0}m");
        assert_eq!(duration(2 * 3600 + 14 * 60), "2\u{a0}h 14\u{a0}m");
        assert_eq!(android_label(35), "Android 15 · API 35");
        assert_eq!(android_label(32), "Android 12L · API 32");
    }

    #[test]
    fn status_labels_match_product_vocabulary() {
        let now = Utc.with_ymd_and_hms(2026, 10, 13, 14, 32, 0).unwrap();
        let mut d = device(DeviceStatus::Running);
        d.runtime = Some(DeviceRuntime {
            pid: 1,
            console_port: 5556,
            adb_serial: "emulator-5556".into(),
            grpc_port: 8554,
            started_at: Utc.with_ymd_and_hms(2026, 10, 13, 12, 18, 0).unwrap(),
        });
        let mut info = DeviceInfo::new(d);
        assert_eq!(status_label(&info, now), "Running · 2\u{a0}h 14\u{a0}m");
        info.device.status = DeviceStatus::Booting;
        info.boot_progress = Some(0.62);
        assert_eq!(status_label(&info, now), "Booting Android… 62%");
        info.busy_holder = Some("CLI".into());
        assert_eq!(status_label(&info, now), "Busy · in use by CLI");
        assert_eq!(device_vm(&info, &utc(now)).state, DevState::Busy);
    }

    #[test]
    fn root_states_are_distinct() {
        let mut info = DeviceInfo::new(device(DeviceStatus::Stopped));
        assert_eq!(root_labels(&info).0, RootKind::None);
        info.device.root.mode = RootMode::Adb;
        assert_eq!(root_labels(&info).1, "Shell root (adb)");
        info.device.root.mode = RootMode::Magisk;
        assert_eq!(root_labels(&info).0, RootKind::Full);
        info.device.root.health = RootHealth::Ok;
        info.device.root.magisk_version = Some("30.7".into());
        assert_eq!(root_labels(&info).1, "Rooted · Magisk 30.7");
        info.device.root.health = RootHealth::Failed;
        assert_eq!(root_labels(&info).0, RootKind::Failed);
        info.root_job = true;
        assert_eq!(root_labels(&info).0, RootKind::SettingUp);
    }

    #[test]
    fn play_store_full_root_is_beta() {
        let mut info = DeviceInfo::new(device(DeviceStatus::Stopped));
        info.device.image.tag = "google_apis_playstore".into();
        info.device.root.mode = RootMode::Magisk;
        let vm = device_vm(&info, &Clock::system());
        assert!(vm.beta);
        assert_eq!(vm.image, "Play Store");
    }

    #[test]
    fn cross_mode_snapshots_are_refused() {
        let s = jidr_core::Snapshot {
            name: "Clean install".into(),
            root_mode: RootMode::Magisk,
            magisk_version: Some("30.7".into()),
            created_at: Utc::now(),
        };
        assert!(snapshot_vm(&s, RootMode::None, None, &Clock::system()).refused);
        assert!(!snapshot_vm(&s, RootMode::Magisk, None, &Clock::system()).refused);
        let quick = jidr_core::Snapshot {
            name: "default_boot".into(),
            ..s
        };
        let vm = snapshot_vm(&quick, RootMode::None, None, &Clock::system());
        assert!(vm.auto && !vm.refused);
    }

    #[test]
    fn hibernated_is_its_own_state() {
        let info = DeviceInfo::new(device(DeviceStatus::Hibernated));
        let vm = device_vm(&info, &Clock::system());
        assert_eq!(vm.state, DevState::Hibernated);
        assert_eq!(vm.status, "Hibernated");
        assert!(!vm.can_base);
    }

    #[test]
    fn budget_meter_levels() {
        let infos = crate::fixtures::device_infos();
        let typical = typical_mb(&infos);
        assert_eq!(typical, 5632);
        let fits = budget_vm(&crate::fixtures::budget(64), typical);
        assert_eq!(
            (fits.running, fits.fit, fits.level),
            (3, 8, BudgetLevel::Fits)
        );
        let near = budget_vm(&crate::fixtures::budget(32), typical);
        assert_eq!(
            (near.running, near.fit, near.level),
            (3, 4, BudgetLevel::Near)
        );
        let full = budget_vm(&crate::fixtures::budget(24), typical);
        assert_eq!((full.fit, full.level), (3, BudgetLevel::Full));
        assert_eq!(
            (full.used.as_str(), full.budget.as_str()),
            ("16.0 GB", "18.0 GB")
        );
        assert_eq!(typical_mb(&[]), 6144);
    }

    #[test]
    fn exhausted_offers_the_longest_idle_devices_that_make_room() {
        let mut x = crate::fixtures::exhausted();
        assert_eq!(x.picks(), vec![Uuid::from_u128(1)]);
        let rows = x.rows();
        assert!(rows[0].picked && !rows[1].picked);
        assert_eq!(rows[0].detail, "Idle 1\u{a0}h 12\u{a0}m · 6.0 GB");
        x.needed_mb = 12_000;
        assert_eq!(x.picks().len(), 2);
        x.needed_mb = 20_000;
        assert!(
            x.picks().is_empty(),
            "hibernating everything still won't fit"
        );

        // The daemon's refusal data (budget.rs `refusal` + state.rs).
        let id = Uuid::from_u128(1);
        let parsed: Exhausted = serde_json::from_value(serde_json::json!({
            "needed_mb": 6144, "used_mb": 16384, "budget_mb": 18432, "total_mb": 24576,
            "devices": [{"id": id, "name": "Pixel-ish 6.1", "expected_mb": 6144, "measured_mb": 6500}],
            "suggestions": ["hibernate", "stop", "lean_profile", "force"],
            "hibernate_candidates": [{"id": id, "name": "Pixel-ish 6.1", "idle_s": 600}]
        }))
        .unwrap();
        assert_eq!(parsed.free_mb(), 2048);
        assert_eq!(parsed.picks(), vec![id]);
        assert_eq!(parsed.rows()[0].detail, "Idle 10\u{a0}m · 6.3 GB");
    }

    #[test]
    fn pool_clones_and_bases_are_grouped_and_labelled() {
        let infos = crate::fixtures::pool_infos();
        let names: Vec<_> = infos.iter().map(|i| i.device.name.as_str()).collect();
        assert_eq!(names[4..], ["Shop base 1", "Shop base 2", "Shop base"]);
        let vms = crate::fixtures::vms(&infos);
        assert_eq!(vms[4].kind, DevKind::Pool);
        assert_eq!(vms[4].status, "Leased · 1\u{a0}h 12\u{a0}m left");
        assert_eq!(vms[4].base_name, "Shop base");
        assert_eq!(vms[5].lease, "While its agent is connected");
        assert_eq!(vms[6].kind, DevKind::Base);
        assert_eq!(vms[6].status, "Base · 2 clones leased");
        assert!(!vms[4].can_base, "a pool clone can't become a base");
        let now = crate::fixtures::now();
        let gone = Lease {
            ttl: false,
            expires_at: Some(now + chrono::Duration::seconds(90)),
        };
        assert_eq!(lease_label(&gone, now), "Agent gone · released in 1\u{a0}m");
    }

    #[test]
    fn only_running_unrooted_or_shell_devices_can_become_bases() {
        let mut info = DeviceInfo::new(device(DeviceStatus::Running));
        assert!(device_vm(&info, &Clock::system()).can_base);
        info.device.root.mode = RootMode::Magisk;
        assert!(!device_vm(&info, &Clock::system()).can_base);
        info.device.root.mode = RootMode::Adb;
        info.device.root.writable_system = true;
        assert!(!device_vm(&info, &Clock::system()).can_base);
        info.device.root.writable_system = false;
        info.device.status = DeviceStatus::Stopped;
        assert!(!device_vm(&info, &Clock::system()).can_base);
    }
}
