//! Daemon data → view models. Every user-facing label is built here, from
//! the status vocabulary in docs/design/PRODUCT.md §5 and the number rules
//! in docs/design/system/README.md, so the Slint files only lay things out.

use chrono::{DateTime, FixedOffset, Utc};
use jidr_core::{Device, DeviceStatus, ImageRef, Job, JobState, RootHealth, RootMode};
use slint::SharedString;

use crate::{DevState, DeviceVm, FormFactor, JobTone, JobVm, RootKind, SnapshotVm, TabVm};

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
        }
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
        DeviceStatus::Starting => DevState::Starting,
        DeviceStatus::Booting => DevState::Booting,
        DeviceStatus::Running => DevState::Running,
        DeviceStatus::Stopping => DevState::Stopping,
        DeviceStatus::Crashed { .. } => DevState::Crashed,
    }
}

/// The status line: "Running · 2 h 14 m", "Booting Android… 62%", …
pub fn status_label(info: &DeviceInfo, now: DateTime<Utc>) -> String {
    if let Some(h) = &info.busy_holder {
        return format!("Busy · in use by {h}");
    }
    match &info.device.status {
        DeviceStatus::Stopped => "Stopped".into(),
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
}
