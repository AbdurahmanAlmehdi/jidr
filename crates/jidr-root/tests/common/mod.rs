//! An in-process daemon with the root provider: the fake backend, a fake
//! Studio SDK and a fake `adb` whose adbd remembers `root` / `unroot`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jidr_backend::fake::FakeBackend;
use jidr_core::provider::CallCtx;
use jidr_core::{Device, JidrPaths, JidrResult};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

/// adbd's uid lives in a file next to the script, so it survives "reboots"
/// unless a test resets it (a real reboot starts adbd as shell again).
const FAKE_ADB: &str = r#"#!/bin/sh
state="$(dirname "$0")/adbd-uid"
[ -f "$state" ] || echo 2000 > "$state"
while [ "$1" = -s ]; do shift 2; done
case "$1" in
  root) echo 0 > "$state"; echo "restarting adbd as root";;
  unroot) echo 2000 > "$state"; echo "restarting adbd as non root";;
  devices) printf 'List of devices attached\nemulator-5556\tdevice\nemulator-5558\tdevice\nemulator-5560\tdevice\n';;
  shell)
    uid=$(cat "$state")
    if [ "$uid" = 0 ]; then name=root; else name=shell; fi
    case "$2" in
      id) echo "uid=$uid($name) gid=$uid($name) groups=$uid($name)";;
      *) echo ok;;
    esac;;
  *) echo ok;;
esac
"#;

pub struct Env {
    pub tmp: tempfile::TempDir,
    pub studio: PathBuf,
    pub backend: Arc<FakeBackend>,
    pub daemon: Arc<Daemon>,
    pub ctx: CallCtx,
}

impl Env {
    pub async fn start() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let studio = jidr_daemon::testing::fake_studio(&tmp.path().join("studio")).unwrap();
        write_exec(&studio.join("platform-tools/adb"), FAKE_ADB);
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
        use jidr_core::provider::DaemonContext;
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
}

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
