//! Emulator integration: create → clone → boot the clone → delete, using an
//! image from `JIDR_TEST_SDK` (read-only) and a temp AVD home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use jidr_core::{ImageRef, JidrPaths};
use jidr_sdk::{AvdManager, AvdSpec, Sdk, SdkSet, profiles};

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

fn gpu() -> &'static str {
    if cfg!(target_os = "macos") {
        "host"
    } else {
        "swiftshader_indirect"
    }
}

fn free_console_port() -> u16 {
    (5600..5680)
        .step_by(2)
        .find(|p| {
            std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()
                && std::net::TcpListener::bind(("127.0.0.1", *p + 1)).is_ok()
        })
        .unwrap()
}

#[test]
fn create_clone_boot_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = JidrPaths::under(tmp.path());
    let sdks = SdkSet {
        private: Sdk::private(&paths),
        studio: Some(Sdk::read_only(test_sdk())),
    };
    let image = test_image();
    let sdk = sdks
        .find_image(&image)
        .expect("test image installed in JIDR_TEST_SDK")
        .clone();
    let tools = sdks.tools().unwrap().clone();

    let avds = AvdManager::new(paths.avd_home());
    let spec = AvdSpec {
        name: "jidr_e2e_src".into(),
        display_name: "E2E source".into(),
        profile: profiles::find("small-phone-5").unwrap(),
        image: image.clone(),
        overrides: Default::default(),
    };
    avds.create(&spec).unwrap();
    avds.clone_avd("jidr_e2e_src", "jidr_e2e_clone", "E2E clone")
        .unwrap();

    let listed = Command::new(tools.emulator())
        .arg("-list-avds")
        .env("ANDROID_AVD_HOME", avds.home())
        .env("ANDROID_SDK_ROOT", &sdk.root)
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listed.contains("jidr_e2e_src") && listed.contains("jidr_e2e_clone"),
        "{listed}"
    );

    // Boot the clone to prove a cloned AVD is valid.
    let port = free_console_port();
    let serial = format!("emulator-{port}");
    let mut child = Command::new(tools.emulator())
        .args([
            "-avd",
            "jidr_e2e_clone",
            "-no-window",
            "-no-audio",
            "-no-boot-anim",
            "-no-snapshot",
        ])
        .args(["-gpu", gpu(), "-port", &port.to_string()])
        .env("ANDROID_AVD_HOME", avds.home())
        .env("ANDROID_SDK_ROOT", &sdk.root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let adb = |args: &[&str]| {
        Command::new(tools.adb())
            .arg("-s")
            .arg(&serial)
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let start = Instant::now();
    let mut booted = false;
    while start.elapsed() < Duration::from_secs(300) {
        if adb(&["shell", "getprop", "sys.boot_completed"]) == "1" {
            booted = true;
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("emulator exited early: {status}");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let model = adb(&["shell", "getprop", "ro.product.cpu.abi"]);
    adb(&["emu", "kill"]);
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(booted, "clone did not boot within 300 s");
    assert_eq!(model, image.abi);
    println!("clone booted in {:.1} s", start.elapsed().as_secs_f64());

    assert!(avds.disk_usage("jidr_e2e_clone").unwrap() > 0);
    avds.delete("jidr_e2e_src").unwrap();
    avds.delete("jidr_e2e_clone").unwrap();
    assert!(avds.list().unwrap().is_empty());
}
