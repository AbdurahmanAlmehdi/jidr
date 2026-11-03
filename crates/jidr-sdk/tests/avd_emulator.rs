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

    // Per-process names, a guard that kills the clone's emulator however
    // the test ends, and cleanup of orphans from earlier killed runs.
    jidr_backend::testing::kill_orphans("jidr_e2e_clone-");
    let src = jidr_backend::testing::unique_avd("jidr_e2e_src");
    let clone_name = jidr_backend::testing::unique_avd("jidr_e2e_clone");
    let _guard = jidr_backend::testing::AvdGuard::new(&clone_name);
    let avds = AvdManager::new(paths.avd_home());
    let spec = AvdSpec {
        name: src.clone(),
        display_name: "E2E source".into(),
        profile: profiles::find("small-phone-5").unwrap(),
        image: image.clone(),
        overrides: Default::default(),
    };
    avds.create(&spec).unwrap();
    avds.clone_avd(&src, &clone_name, "E2E clone").unwrap();

    let listed = Command::new(tools.emulator())
        .arg("-list-avds")
        .env("ANDROID_AVD_HOME", avds.home())
        .env("ANDROID_SDK_ROOT", &sdk.root)
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listed.contains(src.as_str()) && listed.contains(clone_name.as_str()),
        "{listed}"
    );

    // Boot the clone to prove a cloned AVD is valid.
    let port = free_console_port();
    let serial = format!("emulator-{port}");
    let mut child = Command::new(tools.emulator())
        .args([
            "-avd",
            &clone_name,
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

    assert!(avds.disk_usage(&clone_name).unwrap() > 0);
    avds.delete(&src).unwrap();
    avds.delete(&clone_name).unwrap();
    assert!(avds.list().unwrap().is_empty());
}

/// A running emulator of a test AVD, killed (with its QEMU child) on drop.
struct Emu {
    child: std::process::Child,
    serial: String,
    adb: PathBuf,
}

impl Emu {
    fn launch(avds: &AvdManager, sdk: &Sdk, tools: &Sdk, avd: &str, extra: &[&str]) -> Self {
        let port = free_console_port();
        let child = Command::new(tools.emulator())
            .args(["-avd", avd, "-no-window", "-no-audio", "-no-boot-anim"])
            .args(["-gpu", gpu(), "-port", &port.to_string()])
            .args(extra)
            .env("ANDROID_AVD_HOME", avds.home())
            .env("ANDROID_SDK_ROOT", &sdk.root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            child,
            serial: format!("emulator-{port}"),
            adb: tools.adb(),
        }
    }

    fn adb(&self, args: &[&str]) -> String {
        Command::new(&self.adb)
            .arg("-s")
            .arg(&self.serial)
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    /// Seconds until the guest reports boot completed.
    fn wait_booted(&mut self, within: Duration) -> f64 {
        let start = Instant::now();
        while start.elapsed() < within {
            if self.adb(&["shell", "getprop", "sys.boot_completed"]) == "1" {
                return start.elapsed().as_secs_f64();
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!("emulator exited early: {status}");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        panic!("{} did not boot within {within:?}", self.serial);
    }

    /// Pids of the launcher and its QEMU child.
    fn pids(&self) -> Vec<u32> {
        let me = self.child.id();
        let out = Command::new("pgrep")
            .args(["-P", &me.to_string()])
            .output()
            .unwrap();
        let mut v = vec![me];
        v.extend(
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .filter_map(|p| p.parse::<u32>().ok()),
        );
        v
    }

    /// Every file the emulator processes hold open.
    fn open_files(&self) -> String {
        self.pids()
            .iter()
            .map(|p| {
                Command::new("lsof")
                    .args(["-p", &p.to_string(), "-Fn"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default()
            })
            .collect()
    }
}

impl Drop for Emu {
    fn drop(&mut self) {
        for p in self.pids().into_iter().rev() {
            let _ = Command::new("kill").args(["-9", &p.to_string()]).status();
        }
        let _ = self.child.wait();
    }
}

/// Phase 13b acceptance (D42): snapshot a device, clone it keeping the
/// snapshot, resume the clone in ≤ 15 s, check it holds only its own disks
/// and that a file written in the clone is absent from the source.
#[test]
fn snapshot_clone_resumes_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = JidrPaths::under(tmp.path());
    let sdks = SdkSet {
        private: Sdk::private(&paths),
        studio: Some(Sdk::read_only(test_sdk())),
    };
    let image = test_image();
    let sdk = sdks
        .find_image(&image)
        .expect("test image installed")
        .clone();
    let tools = sdks.tools().unwrap().clone();
    jidr_backend::testing::kill_orphans("jidr_e2e_snap");
    let src = jidr_backend::testing::unique_avd("jidr_e2e_snapsrc");
    let dst = jidr_backend::testing::unique_avd("jidr_e2e_snapdst");
    let _g1 = jidr_backend::testing::AvdGuard::new(&src);
    let _g2 = jidr_backend::testing::AvdGuard::new(&dst);
    let avds = AvdManager::new(paths.avd_home());
    avds.create(&AvdSpec {
        name: src.clone(),
        display_name: "Snapshot source".into(),
        profile: profiles::find("agent-lean").unwrap(),
        image: image.clone(),
        overrides: Default::default(),
    })
    .unwrap();

    // 1. Boot the source and save a named snapshot.
    {
        let mut e = Emu::launch(&avds, &sdk, &tools, &src, &["-no-snapshot-save"]);
        let t = e.wait_booted(Duration::from_secs(420));
        println!("source cold boot: {t:.0} s");
        let t = Instant::now();
        let out = e.adb(&["emu", "avd", "snapshot", "save", "probe"]);
        println!("snapshot save: {:.1} s ({out})", t.elapsed().as_secs_f64());
        e.adb(&["emu", "kill"]);
        let deadline = Instant::now() + Duration::from_secs(30);
        while e.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    let ram = avds.dir(&src).join("snapshots/probe/ram.bin");
    let ram_len = std::fs::metadata(&ram).map(|m| m.len()).unwrap_or(0);
    println!("snapshot ram.bin: {} MB", ram_len / 1_000_000);
    assert!(ram_len > 0, "the named snapshot holds memory");

    // 2. Clone it (copy-on-write) and resume the clone.
    let t = Instant::now();
    let method = avds
        .clone_with_snapshot(&src, &dst, "Snapshot clone", "probe")
        .unwrap();
    println!("clone: {method:?} in {:.2} s", t.elapsed().as_secs_f64());
    let mut clone = Emu::launch(
        &avds,
        &sdk,
        &tools,
        &dst,
        &["-snapshot", "probe", "-no-snapshot-save"],
    );
    let resumed = clone.wait_booted(Duration::from_secs(120));
    println!("clone resumed in {resumed:.1} s");
    let files = clone.open_files();
    let (src_dir, dst_dir) = (avds.dir(&src), avds.dir(&dst));
    assert!(
        !files.contains(&format!("{}/", src_dir.display())),
        "the clone opened source files:\n{}",
        files
            .lines()
            .filter(|l| l.contains(&src))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        files.contains(&format!("{}/", dst_dir.display())),
        "it uses its own disks"
    );
    clone.adb(&["shell", "echo clone > /data/local/tmp/clone_marker"]);
    assert_eq!(
        clone.adb(&["shell", "cat /data/local/tmp/clone_marker"]),
        "clone"
    );

    // 3. Resume the source alongside: the marker isn't there.
    let mut source = Emu::launch(
        &avds,
        &sdk,
        &tools,
        &src,
        &["-snapshot", "probe", "-no-snapshot-save"],
    );
    let t = source.wait_booted(Duration::from_secs(120));
    println!("source resumed alongside the clone in {t:.1} s");
    let marker = source.adb(&["shell", "ls /data/local/tmp/clone_marker 2>&1"]);
    assert!(
        marker.contains("No such file"),
        "source sees the clone's file: {marker}"
    );
    drop(source);
    drop(clone);
    assert!(resumed <= 15.0, "clone resume took {resumed:.1} s (> 15 s)");
    avds.delete(&src).unwrap();
    avds.delete(&dst).unwrap();
}
