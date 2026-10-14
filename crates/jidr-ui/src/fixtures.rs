//! Sample data from docs/design/PRODUCT.md §7, run through the same model
//! code as live data. Used by the snapshot goldens and tests.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use jidr_core::{
    Device, DeviceRuntime, DeviceStatus, ImageRef, Job, JobState, RootHealth, RootMode, RootState,
    Snapshot,
};
use uuid::Uuid;

use crate::model::{self, Clock, DeviceInfo};
use crate::{DeviceVm, JobVm, SnapshotVm};

/// The moment every golden is rendered at: Oct 13, 14:32 UTC.
pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 13, 14, 32, 0)
        .single()
        .unwrap_or_default()
}

/// Fixture clock: `now()` in UTC.
pub fn clock() -> Clock {
    Clock {
        now: now(),
        tz: FixedOffset::east_opt(0).unwrap_or_else(|| *Utc::now().fixed_offset().offset()),
    }
}

fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 13, h, m, 0)
        .single()
        .unwrap_or_default()
}

fn image(api: u32, tag: &str) -> ImageRef {
    ImageRef {
        api,
        tag: tag.into(),
        abi: "arm64-v8a".into(),
    }
}

fn device(
    n: u128,
    name: &str,
    profile: &str,
    image: ImageRef,
    root: RootState,
    status: DeviceStatus,
) -> Device {
    Device {
        id: Uuid::from_u128(n),
        name: name.into(),
        avd: name.to_lowercase().replace(' ', "_"),
        profile: profile.into(),
        image,
        sdk_root: "/sdk".into(),
        root,
        agent_owned: false,
        created_at: at(9, 0),
        launch: Default::default(),
        status,
        runtime: None,
    }
}

/// The four sample devices, as daemon data.
pub fn device_infos() -> Vec<DeviceInfo> {
    let mut pixel = device(
        1,
        "Pixel-ish 6.1",
        "phone-6.1-fhd",
        image(35, "google_apis"),
        RootState {
            mode: RootMode::Magisk,
            magisk_version: Some("30.7".into()),
            health: RootHealth::Ok,
            verified_at: Some(at(14, 30)),
            ..Default::default()
        },
        DeviceStatus::Running,
    );
    pixel.runtime = Some(DeviceRuntime {
        pid: 4242,
        console_port: 5556,
        adb_serial: "emulator-5556".into(),
        grpc_port: 8556,
        started_at: at(12, 18),
    });
    let tablet = device(
        2,
        "Tablet 11",
        "tablet-11",
        image(34, "google_apis"),
        RootState {
            mode: RootMode::Adb,
            ..Default::default()
        },
        DeviceStatus::Stopped,
    );
    let play = device(
        3,
        "Play Store test",
        "phone-6.1-fhd",
        image(33, "google_apis_playstore"),
        RootState {
            mode: RootMode::Magisk,
            ..Default::default()
        },
        DeviceStatus::Booting,
    );
    let mut ci = device(
        4,
        "CI-runner-03",
        "small-phone-5",
        image(30, "default"),
        RootState::default(),
        DeviceStatus::Running,
    );
    ci.runtime = Some(DeviceRuntime {
        pid: 4343,
        console_port: 5558,
        adb_serial: "emulator-5558".into(),
        grpc_port: 8558,
        started_at: at(13, 50),
    });

    let info = |d: Device, profile: &str, w, h, dpi, ram_mb, cpus, disk: u64| DeviceInfo {
        profile_name: Some(profile.into()),
        width: w,
        height: h,
        density: dpi,
        ram_mb,
        cpus,
        disk_bytes: Some(disk),
        ..DeviceInfo::new(d)
    };
    let mut play = info(
        play,
        "Phone 6.1″ FHD+",
        1080,
        2400,
        420,
        4096,
        4,
        5_300_000_000,
    );
    play.boot_progress = Some(0.62);
    let mut ci = info(ci, "Small phone 5″", 720, 1280, 320, 2048, 2, 3_000_000_000);
    ci.busy_holder = Some("CLI".into());
    vec![
        info(
            pixel,
            "Phone 6.1″ FHD+",
            1080,
            2400,
            420,
            4096,
            4,
            6_800_000_000,
        ),
        info(
            tablet,
            "Tablet 11″",
            1600,
            2560,
            320,
            4096,
            4,
            4_100_000_000,
        ),
        play,
        ci,
    ]
}

pub fn devices() -> Vec<DeviceVm> {
    device_infos()
        .iter()
        .map(|i| model::device_vm(i, &clock()))
        .collect()
}

/// "Downloading Android 16 (API 36) image · 1.4 / 2.1 GB · 2 min left".
pub fn jobs() -> Vec<JobVm> {
    let job = Job {
        id: Uuid::from_u128(100),
        kind: "image.install".into(),
        device: None,
        progress: Some(0.67),
        message: "Downloading".into(),
        state: JobState::Running,
        error: None,
        result: None,
        created_at: at(14, 20),
    };
    vec![model::job_vm(
        &job,
        "Android 16 (API 36) image",
        Some((1_400_000_000, 2_100_000_000)),
        Some(110),
    )]
}

/// Snapshots on Pixel-ish 6.1, seen from a device in `mode`.
pub fn snapshots(mode: RootMode) -> Vec<SnapshotVm> {
    let snap = |name: &str, root_mode, created_at| Snapshot {
        name: name.into(),
        root_mode,
        magisk_version: Some("30.7".into()),
        created_at,
    };
    vec![
        model::snapshot_vm(
            &snap(
                "Clean install",
                RootMode::Magisk,
                Utc.with_ymd_and_hms(2026, 10, 9, 18, 40, 0)
                    .single()
                    .unwrap_or_default(),
            ),
            mode,
            Some(2_100_000_000),
            &clock(),
        ),
        model::snapshot_vm(
            &snap("Logged in test account", RootMode::Magisk, at(11, 2)),
            mode,
            Some(2_400_000_000),
            &clock(),
        ),
        model::snapshot_vm(
            &snap("default_boot", RootMode::Magisk, at(12, 17)),
            mode,
            None,
            &clock(),
        ),
    ]
}
