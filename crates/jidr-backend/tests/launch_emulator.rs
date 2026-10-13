//! Boots a real emulator: battery over gRPC, stop with a quick-boot snapshot,
//! cold boot with a custom `-ramdisk` through `LaunchOptions`, quick boot.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use jidr_backend::{AospBackend, Backend, BootStage, LaunchOptions, LaunchSpec, grpc};
use jidr_core::{Cancel, ImageRef, JidrPaths};
use jidr_sdk::{AvdManager, AvdSpec, ConfigIni, Sdk, SdkSet, profiles};

fn test_sdk() -> PathBuf {
    std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap().join("Library/Android/sdk"))
}

fn test_image() -> ImageRef {
    let pkg = std::env::var("JIDR_TEST_IMAGE").unwrap_or_else(|_| {
        format!(
            "system-images;android-31;google_apis;{}",
            jidr_sdk::host::host_abi()
        )
    });
    ImageRef::parse_package(&pkg).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn boot_battery_stop_ramdisk_quickboot() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = JidrPaths::under(tmp.path());
    let sdks = SdkSet {
        private: Sdk::private(&paths),
        studio: Some(Sdk::read_only(test_sdk())),
    };
    let image = test_image();
    let image_sdk = sdks
        .find_image(&image)
        .expect("test image installed")
        .clone();
    let tools = sdks.tools().unwrap().clone();

    let avds = AvdManager::new(paths.avd_home());
    avds.create(&AvdSpec {
        name: "jidr_backend_e2e".into(),
        display_name: "Backend E2E".into(),
        profile: profiles::find("small-phone-5").unwrap(),
        image: image.clone(),
        overrides: Default::default(),
    })
    .unwrap();
    let spec = LaunchSpec {
        avd: "jidr_backend_e2e".into(),
        avd_home: paths.avd_home(),
        sdk_root: image_sdk.root.clone(),
        emulator: tools.emulator(),
        adb: tools.adb(),
        log_dir: paths.device_logs("jidr_backend_e2e"),
    };
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
            &spec,
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
    let snap = avds.dir("jidr_backend_e2e").join("snapshots/default_boot");
    assert!(
        snap.is_dir(),
        "quick-boot snapshot missing at {}",
        snap.display()
    );

    // 4. Cold boot with a custom ramdisk via LaunchOptions (Stream C's hook).
    let custom = tmp.path().join("custom-ramdisk.img");
    std::fs::copy(image_sdk.image_dir(&image).join("ramdisk.img"), &custom).unwrap();
    let opts = LaunchOptions {
        ramdisk: Some(custom.clone()),
        cold_boot: true,
        ..quiet.clone()
    };
    let inst = backend
        .start(&spec, &opts, &|_| {}, &Cancel::new())
        .await
        .unwrap();
    let hw = ConfigIni::read(&avds.dir("jidr_backend_e2e").join("hardware-qemu.ini")).unwrap();
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
            &spec,
            &quiet,
            &|s| println!("  quick boot {s:?} at {:.1} s", t.elapsed().as_secs_f64()),
            &Cancel::new(),
        )
        .await
        .unwrap();
    println!("quick boot: {:.1} s", t.elapsed().as_secs_f64());
    backend.stop(&inst).await.unwrap();

    assert!(
        !avds.is_running("jidr_backend_e2e"),
        "stopped AVD must not look locked"
    );
    avds.delete("jidr_backend_e2e").unwrap();
}
