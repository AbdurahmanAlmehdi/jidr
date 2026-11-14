//! Sample data from docs/design/PRODUCT.md §7, run through the same model
//! code as live data. Used by the snapshot goldens and tests.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use jidr_core::{
    Device, DeviceRuntime, DeviceStatus, ImageRef, Job, JobState, RootHealth, RootMode, RootState,
    Snapshot,
};
use uuid::Uuid;

use crate::model::{self, BudgetUse, Clock, DeviceInfo, Exhausted, HostBudget, IdleDevice, Lease};
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
        footprint_mb: None,
        auto_hibernate_minutes: None,
        base: false,
        pool_base: None,
        network_shaping: Default::default(),
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
        bytes: Some(jidr_core::JobBytes {
            done: 1_400_000_000,
            total: 2_100_000_000,
        }),
        state: JobState::Running,
        error: None,
        result: None,
        created_at: at(14, 20),
    };
    let bytes = job.bytes.map(|b| (b.done, b.total));
    vec![model::job_vm(
        &job,
        "Android 16 (API 36) image",
        bytes,
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

/// The built-in profiles (assets/profiles).
pub fn profiles() -> Vec<jidr_core::HardwareProfile> {
    let p = |id: &str, name: &str, w, h, d, ram, cpus| jidr_core::HardwareProfile {
        id: id.into(),
        name: name.into(),
        form_factor: if id.starts_with("tablet") {
            jidr_core::FormFactor::Tablet
        } else {
            jidr_core::FormFactor::Phone
        },
        width: w,
        height: h,
        density: d,
        ram_mb: ram,
        cpus,
        data_mb: 6144,
        config: Default::default(),
        build_props: Default::default(),
        audio: true,
        camera_back: None,
        camera_front: None,
    };
    vec![
        p("phone-6.1-fhd", "Phone 6.1″ FHD+", 1080, 2400, 420, 4096, 4),
        p("phone-6.7-qhd", "Phone 6.7″ QHD+", 1440, 3120, 560, 4096, 4),
        p("small-phone-5", "Small phone 5″", 720, 1280, 320, 2048, 2),
        p("tablet-11", "Tablet 11″", 1600, 2560, 320, 4096, 4),
        p("agent-lean", "Agent lean", 720, 1280, 320, 2048, 2),
    ]
}

/// `host.budget` on a Mac with `total_gb` of RAM at the default 75%, with
/// the sample devices that run (Pixel-ish 6.1, Play Store test, CI-runner-03)
/// counted: 3 of ~8 fit at 64 GB, ~4 at 32 GB (near), ~3 at 24 GB (full).
pub fn budget(total_gb: u64) -> HostBudget {
    HostBudget {
        budget_mb: total_gb * 1024 * 75 / 100,
        used_mb: 6144 + 6144 + 4096,
        devices: vec![
            budget_use(1, "Pixel-ish 6.1", 6144),
            budget_use(3, "Play Store test", 6144),
            budget_use(4, "CI-runner-03", 4096),
        ],
    }
}

fn budget_use(n: u128, name: &str, mb: u64) -> BudgetUse {
    BudgetUse {
        id: Uuid::from_u128(n),
        name: name.into(),
        expected_mb: mb,
        measured_mb: None,
    }
}

/// Starting Tablet 11 on a 24 GB Mac: `resource_exhausted` with Pixel-ish
/// 6.1 idle the longest (hibernating it makes room).
pub fn exhausted() -> Exhausted {
    let b = budget(24);
    Exhausted {
        needed_mb: 6144,
        used_mb: b.used_mb,
        budget_mb: b.budget_mb,
        devices: b.devices,
        hibernate_candidates: vec![
            IdleDevice {
                id: Uuid::from_u128(1),
                name: "Pixel-ish 6.1".into(),
                idle_s: 72 * 60,
            },
            IdleDevice {
                id: Uuid::from_u128(4),
                name: "CI-runner-03".into(),
                idle_s: 42 * 60,
            },
        ],
    }
}

/// The sample devices with Tablet 11 hibernated.
pub fn hibernated_infos() -> Vec<DeviceInfo> {
    let mut infos = device_infos();
    infos[1].device.status = DeviceStatus::Hibernated;
    infos
}

/// The sample devices plus an agent pool (D42): a base "Shop base" and two
/// leased clones of it, one with 1 h 12 m left and one held by its agent's
/// connection. Grouped the way the bridge sorts them.
pub fn pool_infos() -> Vec<DeviceInfo> {
    let base_id = Uuid::from_u128(20);
    let mut base = device(
        20,
        "Shop base",
        "agent-lean",
        image(34, "google_apis"),
        RootState {
            mode: RootMode::Adb,
            ..Default::default()
        },
        DeviceStatus::Stopped,
    );
    base.base = true;
    let clone = |n: u128, name: &str, port: u16, started: DateTime<Utc>| {
        let mut d = device(
            n,
            name,
            "agent-lean",
            image(34, "google_apis"),
            RootState {
                mode: RootMode::Adb,
                ..Default::default()
            },
            DeviceStatus::Running,
        );
        d.pool_base = Some(base_id);
        d.runtime = Some(DeviceRuntime {
            pid: u32::from(port),
            console_port: port,
            adb_serial: format!("emulator-{port}"),
            grpc_port: port + 3000,
            started_at: started,
        });
        d
    };
    let lean = |d: Device| DeviceInfo {
        profile_name: Some("Agent lean".into()),
        width: 720,
        height: 1280,
        density: 320,
        ram_mb: 2048,
        cpus: 2,
        disk_bytes: Some(400_000_000),
        ..DeviceInfo::new(d)
    };
    let mut c1 = lean(clone(21, "Shop base 1", 5560, at(14, 2)));
    c1.lease = Some(Lease {
        ttl: true,
        expires_at: Some(at(15, 44)),
    });
    c1.base_name = Some("Shop base".into());
    let mut c2 = lean(clone(22, "Shop base 2", 5562, at(14, 20)));
    c2.lease = Some(Lease {
        ttl: false,
        expires_at: None,
    });
    c2.base_name = Some("Shop base".into());
    let mut b = lean(base);
    b.base_leases = 2;
    let mut infos = device_infos();
    infos.extend([b, c1, c2]);
    infos.sort_by_key(model::group_rank);
    infos
}

/// View models for `infos` at the fixture clock.
pub fn vms(infos: &[DeviceInfo]) -> Vec<DeviceVm> {
    infos
        .iter()
        .map(|i| model::device_vm(i, &clock()))
        .collect()
}

/// The image list from the create design, newest first.
pub fn system_images() -> Vec<jidr_core::SystemImage> {
    let img = |api, tag: &str, size: u64, installed: bool| jidr_core::SystemImage {
        image: image(api, tag),
        display: String::new(),
        revision: "1".into(),
        support: jidr_core::Support::for_api(api),
        size,
        license: Some("android-sdk-license".into()),
        installed_in: installed.then(|| "/sdk".into()),
        pristine_ramdisk_sha256: None,
    };
    crate::device_form::sorted_images(&[
        img(36, "google_apis", 2_100_000_000, false),
        img(35, "google_apis", 2_000_000_000, true),
        img(35, "google_apis_playstore", 2_200_000_000, false),
        img(34, "google_apis", 1_900_000_000, true),
        img(33, "google_apis_playstore", 1_800_000_000, true),
        img(30, "default", 1_200_000_000, true),
    ])
}

/// The API 36 download in progress (67%).
pub fn download_job() -> Job {
    Job {
        id: Uuid::from_u128(100),
        kind: "image.install".into(),
        device: None,
        progress: Some(0.67),
        message: "Downloading system-images;android-36;google_apis;arm64-v8a".into(),
        bytes: Some(jidr_core::JobBytes {
            done: 1_400_000_000,
            total: 2_100_000_000,
        }),
        state: JobState::Running,
        error: None,
        result: None,
        created_at: at(14, 20),
    }
}

/// Google's SDK license opening, as shown in step 2 (excerpt; the app shows
/// the full text from `sdk.licenses`).
pub const LICENSE_EXCERPT: &str = "1. Introduction\n\n1.1 The Android Software Development Kit (referred to in the License Agreement as the \"SDK\" and specifically including the Android system files, packaged APIs, and Google APIs add-ons) is licensed to you subject to the terms of the License Agreement. The License Agreement forms a legally binding contract between you and Google in relation to your use of the SDK.\n\n1.2 \"Android\" means the Android software stack for devices, as made available under the Android Open Source Project.\n\n2. Accepting this License Agreement\n\n2.1 In order to use the SDK, you must first agree to the License Agreement. You may not use the SDK if you do not accept the License Agreement.\n\n2.2 By clicking to accept, you hereby agree to the terms of the License Agreement.";

/// Logcat lines from PRODUCT.md §7 and the Device design.
pub fn log_lines() -> Vec<crate::LogLine> {
    [
        "10-13 14:02:07.318  4821  4821 I ActivityManager: Start proc 4821:com.example.shop/u0a190",
        "10-13 14:02:07.402  4821  4821 D ShopApp: onCreate: restoring cart (3 items)",
        "10-13 14:02:08.115  1288  1288 W Magisk: su: request from uid 10190 (com.example.shop)",
        "10-13 14:02:08.640  4821  4821 E AndroidRuntime: FATAL EXCEPTION: main",
        "10-13 14:02:09.010   612   612 I ActivityManager: Process com.example.shop (pid 4821) has died",
    ]
    .iter()
    .filter_map(|l| crate::player::logcat::parse(l))
    .collect()
}
