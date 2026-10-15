//! An in-process daemon with the root provider: the fake backend, a fake
//! Studio SDK and a fake `adb` whose adbd remembers `root` / `unroot` and which
//! simulates Magisk (`su`, the `magisk` binary and the su-policy database).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jidr_backend::fake::FakeBackend;
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{Device, ImageRef, JidrPaths, JidrResult};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

/// The Magisk version the fake APK (and fake `magisk -v`) reports.
pub const MAGISK_VERSION: &str = "30.7";

/// adbd's uid lives in a file next to the script, so it survives "reboots"
/// unless a test resets it (a real reboot starts adbd as shell again). A
/// second file records whether the shell's su request has been granted.
const FAKE_ADB: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
state="$dir/adbd-uid"
grant="$dir/su-granted"
[ -f "$state" ] || echo 2000 > "$state"
while [ "$1" = -s ]; do shift 2; done
case "$1" in
  root) echo 0 > "$state"; echo "restarting adbd as root";;
  unroot) echo 2000 > "$state"; echo "restarting adbd as non root";;
  devices) printf 'List of devices attached\nemulator-5556\tdevice\nemulator-5558\tdevice\nemulator-5560\tdevice\n';;
  push) echo "pushed";;
  install) echo "Success";;
  uninstall) echo "Success";;
  logcat) echo 'I/fake: hello';;
  shell)
    uid=$(cat "$state")
    if [ "$uid" = 0 ]; then name=root; else name=shell; fi
    cmd="$2"
    case "$cmd" in
      "id -u") echo "$uid";;
      id*) echo "uid=$uid($name) gid=$uid($name) groups=$uid($name)";;
      "su -c id")
        if [ -f "$grant" ]; then echo "uid=0(root) gid=0(root) groups=0(root)"; else echo "su: Permission denied" >&2; exit 1; fi;;
      "su -c 'magisk -v'"|"su -c magisk -v")
        if [ -f "$grant" ]; then echo "30.7:MAGISK:R"; else echo "su: Permission denied" >&2; exit 1; fi;;
      "[ -x /debug_ramdisk/magisk ] && echo yes"|"[ -x /sbin/magisk ] && echo yes")
        echo "yes";;
      "/debug_ramdisk/magisk --sqlite"*|"/sbin/magisk --sqlite"*|"/data/adb/magisk/magisk --sqlite"*)
        case "$cmd" in
          *"REPLACE INTO policies"*) echo 1 > "$grant"; echo "done";;
          *"SELECT policy FROM policies"*)
            if [ -f "$grant" ]; then echo "policy=2"; else echo "policy=0"; fi;;
          *) echo "done";;
        esac;;
      *) echo ok;;
    esac;;
  *) echo ok;;
esac
"#;

pub struct Env {
    pub tmp: tempfile::TempDir,
    pub studio: PathBuf,
    pub apk: PathBuf,
    pub backend: Arc<FakeBackend>,
    pub daemon: Arc<Daemon>,
    pub ctx: CallCtx,
}

/// A minimal, parseable pristine ramdisk (one uncompressed cpio segment).
fn valid_ramdisk() -> Vec<u8> {
    let mut a = jidr_cpio::Archive::new();
    a.insert(jidr_cpio::Entry::file(
        "init",
        0o750,
        b"#!stock init".to_vec(),
    ));
    let ramdisk = jidr_cpio::Ramdisk {
        segments: vec![jidr_cpio::Segment {
            compression: jidr_cpio::Compression::None,
            archive: a,
        }],
        trailing: Vec::new(),
    };
    ramdisk.to_bytes().unwrap()
}

/// Writes a valid pristine ramdisk over the fake SDK's `ramdisk.img` (the fake
/// SDK writes a single byte, which the cpio parser can't read).
fn write_valid_ramdisks(studio: &Path) {
    let base = jidr_daemon::testing::fake_image();
    for tag in ["google_apis", "google_apis_playstore"] {
        let image = ImageRef {
            tag: tag.to_string(),
            ..base.clone()
        };
        let path = studio.join(image.sysdir()).join("ramdisk.img");
        std::fs::write(path, valid_ramdisk()).unwrap();
    }
}

/// A synthetic Magisk release APK: a zip with the payload libraries, the
/// environment files and `assets/util_functions.sh` declaring the version.
fn write_fake_apk(dir: &Path) -> PathBuf {
    let path = dir.join("Magisk-v30.7.apk");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    let add = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, body: &[u8]| {
        zip.start_file(name, opts).unwrap();
        use std::io::Write;
        zip.write_all(body).unwrap();
    };
    for abi in ["arm64-v8a", "x86_64"] {
        add(
            &mut zip,
            &format!("lib/{abi}/libmagiskinit.so"),
            b"magiskinit",
        );
        add(&mut zip, &format!("lib/{abi}/libmagisk.so"), b"magisk");
        add(&mut zip, &format!("lib/{abi}/libinit-ld.so"), b"init-ld");
        add(
            &mut zip,
            &format!("lib/{abi}/libmagiskpolicy.so"),
            b"magiskpolicy",
        );
    }
    add(&mut zip, "assets/stub.apk", b"stub");
    add(
        &mut zip,
        "assets/util_functions.sh",
        b"MAGISK_VER='30.7'\nMAGISK_VER_CODE=30700\n",
    );
    add(&mut zip, "assets/boot_patch.sh", b"#!/system/bin/sh\n");
    add(&mut zip, "assets/addon.d.sh", b"#!/system/bin/sh\n");
    add(&mut zip, "assets/chromeos/futility", b"futility");
    add(&mut zip, "assets/chromeos/kernel_data_key.vbprivk", b"key");
    add(&mut zip, "assets/chromeos/kernel.keyblock", b"keyblock");
    zip.finish().unwrap();
    path
}

impl Env {
    pub async fn start() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let studio = jidr_daemon::testing::fake_studio(&tmp.path().join("studio")).unwrap();
        write_exec(&studio.join("platform-tools/adb"), FAKE_ADB);
        write_valid_ramdisks(&studio);
        let apk = write_fake_apk(tmp.path());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join("repository2-3.xml"),
            "<sdk:sdk-repository xmlns:sdk=\"x\"/>",
        )
        .unwrap();
        let backend = Arc::new(FakeBackend::new());
        let daemon = Daemon::builder(
            DaemonConfig {
                paths: JidrPaths::under(tmp.path().join("j")),
                studio_sdk: Some(studio.clone()),
                idle_timeout: None,
                sdk_repo: Some(format!("file://{}/", repo.display())),
            },
            backend.clone(),
        )
        .provider(jidr_root::provider())
        .build()
        .unwrap();
        let ctx = CallCtx::new(daemon.state().clone());
        Self {
            tmp,
            studio,
            apk,
            backend,
            daemon,
            ctx,
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> JidrResult<Value> {
        let (provider, _) = self.daemon.registry().get(method).unwrap();
        provider.call(&self.ctx, method, params).await
    }

    pub async fn create(&self, name: &str, image: &str, root_mode: Option<&str>) -> Device {
        let mut params = json!({ "name": name, "image": image });
        if let Some(mode) = root_mode {
            params["root_mode"] = json!(mode);
        }
        self.call("device.create", params).await.unwrap();
        self.device(name).await
    }

    pub async fn device(&self, name: &str) -> Device {
        self.daemon.state().device(name).await.unwrap()
    }

    pub fn adbd_uid(&self) -> String {
        std::fs::read_to_string(self.studio.join("platform-tools/adbd-uid"))
            .unwrap_or_else(|_| "2000".into())
            .trim()
            .to_string()
    }

    /// What a real reboot does to adbd.
    pub fn reset_adbd(&self, uid: &str) {
        std::fs::write(
            self.studio.join("platform-tools/adbd-uid"),
            format!("{uid}\n"),
        )
        .unwrap();
    }

    /// Whether the fake Magisk has granted the shell su.
    pub fn su_granted(&self) -> bool {
        self.studio.join("platform-tools/su-granted").exists()
    }

    /// Simulates the person tapping Grant on the Magisk prompt.
    pub fn tap_grant(&self) {
        std::fs::write(self.studio.join("platform-tools/su-granted"), b"1").unwrap();
    }

    pub fn revoke_su(&self) {
        let _ = std::fs::remove_file(self.studio.join("platform-tools/su-granted"));
    }

    /// SHA-256 of the pristine `ramdisk.img` the fake SDK provides.
    pub fn pristine_sha256(&self) -> String {
        let image = jidr_daemon::testing::fake_image();
        jidr_sdk::cache::sha256_file(&self.studio.join(image.sysdir()).join("ramdisk.img")).unwrap()
    }

    /// The per-device patched ramdisk path for the pinned version.
    pub fn patched_ramdisk(&self, device: &Device) -> PathBuf {
        self.daemon
            .state()
            .paths()
            .avd_home()
            .join(format!("{}.avd", device.avd))
            .join("jidr")
            .join(format!("ramdisk.magisk-{MAGISK_VERSION}.img"))
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
