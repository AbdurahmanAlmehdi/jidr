//! `jidr-adb` against a fake `adb` shell script: argument passing, output
//! parsing and error classification without a real device.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use jidr_adb::{Adb, InstallOptions};
use jidr_core::ErrorCode;

const FAKE: &str = r#"#!/bin/bash
echo "$*" >> "$(dirname "$0")/calls.log"
serial=""
if [ "$1" = "-s" ]; then serial="$2"; shift 2; fi
if [ "$serial" = "emulator-9999" ]; then
  echo "error: device 'emulator-9999' not found" >&2; exit 1
fi
case "$1" in
  version) echo "Android Debug Bridge version 1.0.41";;
  devices) printf 'List of devices attached\nemulator-5554\tdevice\nemulator-5556\toffline\n\n';;
  shell)
    case "$2" in
      "getprop sys.boot_completed") echo 1;;
      "exit 3") echo partial; exit 3;;
      *) echo "ran: $2";;
    esac;;
  install)
    apk="${@: -1}"
    if [[ "$apk" == *bad-sig* ]]; then
      echo "Performing Streamed Install"
      echo "adb: failed to install $apk: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package signatures do not match]" >&2
      exit 1
    fi
    echo "Performing Streamed Install"; echo "Success";;
  uninstall) echo "Success";;
  push) echo "$2: 1 file pushed";;
  pull) touch "$3"; echo "$2: 1 file pulled";;
  root)
    if [ "$serial" = "emulator-5556" ]; then echo "adbd cannot run as root in production builds"; exit 1; fi
    echo "restarting adbd as root";;
  unroot) echo "restarting adbd as non root";;
  connect) echo "connected to $2";;
  logcat)
    if [ "$2" = "-d" ]; then echo "D/tag: one"; echo "D/tag: two";
    elif [ "$2" = "-c" ]; then :;
    else echo "I/live: a"; echo "I/live: b"; sleep 30; fi;;
  slow) sleep 10;;
  *) echo "unknown command $1" >&2; exit 1;;
esac
"#;

fn fake() -> (tempfile::TempDir, Adb) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("adb");
    std::fs::write(&bin, FAKE).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (dir, Adb::new(bin))
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn devices_and_version() {
    let (_d, adb) = fake();
    assert!(adb.version().await.unwrap().contains("1.0.41"));
    let devs = adb.devices().await.unwrap();
    assert_eq!(devs.len(), 2);
    assert_eq!(devs[1].state, "offline");
}

#[tokio::test]
async fn shell_passes_serial_and_exit_codes() {
    let (dir, adb) = fake();
    let dev = adb.device("emulator-5554");
    assert_eq!(dev.getprop("sys.boot_completed").await.unwrap(), "1");
    assert_eq!(
        dev.shell_status("exit 3").await.unwrap(),
        (3, "partial\n".into())
    );
    assert_eq!(dev.shell("exit 3").await.unwrap_err().code, ErrorCode::Sdk);
    assert!(
        calls(dir.path())
            .iter()
            .any(|c| c == "-s emulator-5554 shell getprop sys.boot_completed")
    );
}

#[tokio::test]
async fn missing_device_is_not_found() {
    let (_d, adb) = fake();
    let err = adb.device("emulator-9999").shell("id").await.unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err}");
    let err = adb
        .device("emulator-9999")
        .shell_status("id")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err}");
}

#[tokio::test]
async fn install_success_and_signature_mismatch() {
    let (dir, adb) = fake();
    let dev = adb.device("emulator-5554");
    let opts = InstallOptions {
        replace: true,
        grant_permissions: true,
        downgrade: false,
    };
    dev.install(Path::new("/tmp/app.apk"), opts).await.unwrap();
    assert!(
        calls(dir.path())
            .iter()
            .any(|c| c.ends_with("install -r -g /tmp/app.apk"))
    );

    let err = dev
        .install(Path::new("/tmp/bad-sig.apk"), InstallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Sdk);
    let data = err.data.unwrap();
    assert_eq!(data["reason"], "INSTALL_FAILED_UPDATE_INCOMPATIBLE");
    assert_eq!(data["signature_mismatch"], true);
    dev.uninstall("com.example").await.unwrap();
}

#[tokio::test]
async fn push_pull() {
    let (dir, adb) = fake();
    let dev = adb.device("emulator-5554");
    dev.push(Path::new("/tmp/f.txt"), "/sdcard/Download/")
        .await
        .unwrap();
    let local: PathBuf = dir.path().join("pulled.txt");
    dev.pull("/sdcard/x.txt", &local).await.unwrap();
    assert!(local.exists());
}

#[tokio::test]
async fn root_unroot_and_production_builds() {
    let (_d, adb) = fake();
    adb.device("emulator-5554").root().await.unwrap();
    adb.device("emulator-5554").unroot().await.unwrap();
    let err = adb.device("emulator-5556").root().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::RootFailed, "{err}");
}

#[tokio::test]
async fn logcat_dump_clear_follow() {
    let (_d, adb) = fake();
    let dev = adb.device("emulator-5554");
    assert_eq!(dev.logcat_dump(&[]).await.unwrap().lines().count(), 2);
    dev.logcat_clear().await.unwrap();
    let mut s = dev.logcat_follow(&["-v", "brief"]).unwrap();
    assert_eq!(s.lines.recv().await.unwrap(), "I/live: a");
    assert_eq!(s.lines.recv().await.unwrap(), "I/live: b");
    s.stop().await;
}

#[tokio::test]
async fn connect_and_wait_for_device() {
    let (_d, adb) = fake();
    adb.connect("127.0.0.1:5585").await.unwrap();
    adb.device("emulator-5554")
        .wait_for_device(Duration::from_secs(2))
        .await
        .unwrap();
    let err = adb
        .device("emulator-5556")
        .wait_for_device(Duration::from_millis(600))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::BootTimeout);
    assert_eq!(
        adb.wait_for_device(Duration::from_secs(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Usage
    );
}

#[tokio::test]
async fn timeouts_and_missing_binary() {
    let (_d, adb) = fake();
    let err = adb
        .with_timeout(Duration::from_millis(300))
        .run(&["slow"])
        .await
        .unwrap_err();
    assert!(err.message.contains("timed out"), "{err}");
    let err = Adb::new("/nonexistent/adb").version().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Sdk);
}
