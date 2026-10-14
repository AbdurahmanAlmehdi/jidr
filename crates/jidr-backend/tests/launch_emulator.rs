//! Boots a real emulator: battery over gRPC, stop with a quick-boot snapshot,
//! cold boot with a custom `-ramdisk` through `LaunchOptions`, quick boot.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Mutex;
use std::time::Instant;

use jidr_backend::{AospBackend, Backend, BootStage, LaunchOptions, grpc};
use jidr_core::Cancel;
use jidr_sdk::ConfigIni;

#[tokio::test(flavor = "multi_thread")]
async fn boot_battery_stop_ramdisk_quickboot() {
    let f = common::fixture("jidr_backend_e2e");
    let (spec, avds) = (&f.spec, &f.avds);
    let quiet = LaunchOptions {
        audio: false,
        ..Default::default()
    };
    let backend = AospBackend::new();

    // 1. First boot.
    let stages = Mutex::new(Vec::new());
    let t = Instant::now();
    let inst = backend
        .start(
            spec,
            &quiet,
            &|s| stages.lock().unwrap().push(s),
            &Cancel::new(),
        )
        .await
        .unwrap();
    println!("first boot: {:.1} s", t.elapsed().as_secs_f64());
    assert_eq!(
        *stages.lock().unwrap(),
        vec![
            BootStage::Spawned,
            BootStage::GrpcReady,
            BootStage::AdbOnline,
            BootStage::Booted
        ]
    );
    assert!(backend.is_alive(&inst).await);

    // 2. Battery over gRPC.
    let (mut ctl, _) = grpc::connect(&inst).await.unwrap();
    let battery = ctl.get_battery(()).await.unwrap().into_inner();
    assert!(battery.has_battery);
    assert!((0..=100).contains(&battery.charge_level), "{battery:?}");

    // 3. Graceful stop saves the quick-boot snapshot (D29).
    backend.stop(&inst).await.unwrap();
    assert!(!backend.is_alive(&inst).await);
    let snap = avds.dir(&spec.avd).join("snapshots/default_boot");
    assert!(
        snap.is_dir(),
        "quick-boot snapshot missing at {}",
        snap.display()
    );

    // 4. Cold boot with a custom ramdisk via LaunchOptions (Stream C's hook).
    let custom = f.tmp.path().join("custom-ramdisk.img");
    std::fs::copy(
        f.image_sdk
            .image_dir(&common::test_image())
            .join("ramdisk.img"),
        &custom,
    )
    .unwrap();
    let opts = LaunchOptions {
        ramdisk: Some(custom.clone()),
        cold_boot: true,
        camera_back: Some("emulated".into()),
        ..quiet.clone()
    };
    let inst = backend
        .start(spec, &opts, &|_| {}, &Cancel::new())
        .await
        .unwrap();
    let hw = ConfigIni::read(&avds.dir(&spec.avd).join("hardware-qemu.ini")).unwrap();
    assert_eq!(hw.get("hw.camera.back"), Some("emulated"), "camera option applied (R7)");
    assert_eq!(
        hw.get("disk.ramdisk.path"),
        Some(custom.to_str().unwrap()),
        "emulator used our ramdisk"
    );
    backend.stop(&inst).await.unwrap();

    // 5. Quick boot from the snapshot.
    let t = Instant::now();
    let inst = backend
        .start(
            spec,
            &quiet,
            &|s| println!("  quick boot {s:?} at {:.1} s", t.elapsed().as_secs_f64()),
            &Cancel::new(),
        )
        .await
        .unwrap();
    println!("quick boot: {:.1} s", t.elapsed().as_secs_f64());
    backend.stop(&inst).await.unwrap();

    assert!(
        !avds.is_running(&spec.avd),
        "stopped AVD must not look locked"
    );
    avds.delete(&spec.avd).unwrap();
}
