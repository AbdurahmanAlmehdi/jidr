//! An in-process daemon on the real AOSP backend, for `--features emulator`
//! tests. AVDs live in a temp dir (D38); the SDKs are only read.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jidr_backend::testing::{AvdGuard, kill_orphans};
use jidr_core::{ImageRef, JidrPaths};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::json;

use super::Conn;

/// Device names start with this, so orphans of killed runs can be found.
pub const PREFIX: &str = "jd13";

pub fn test_sdk() -> PathBuf {
    std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Android/sdk")
        })
}

/// `JIDR_TEST_IMAGE`, default API 35 `google_apis` for the host ABI (the
/// image D42 was measured on).
pub fn test_image() -> ImageRef {
    let abi = jidr_sdk::host::host_abi();
    let pkg = std::env::var("JIDR_TEST_IMAGE")
        .unwrap_or_else(|_| format!("system-images;android-35;google_apis;{abi}"));
    ImageRef::parse_package(&pkg).unwrap()
}

/// Where the test image is installed: the test SDK, or else the user's Jidr
/// SDK (linked into the temp private SDK, read-only).
fn image_dir(image: &ImageRef) -> Option<PathBuf> {
    let jidr_sdk = dirs_data().map(|d| d.join("Jidr/sdk"));
    [Some(test_sdk()), jidr_sdk]
        .into_iter()
        .flatten()
        .map(|root| root.join(image.sysdir()))
        .find(|d| d.join("system.img").is_file())
}

fn dirs_data() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
}

/// A unique device name for this test process.
pub fn name(base: &str) -> String {
    jidr_backend::testing::unique_avd(&format!("{PREFIX}_{base}"))
}

pub struct Emu {
    pub tmp: tempfile::TempDir,
    pub paths: JidrPaths,
    pub daemon: Arc<Daemon>,
    pub image: ImageRef,
    guards: Mutex<Vec<AvdGuard>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<jidr_core::JidrResult<()>>>,
}

impl Emu {
    pub async fn start() -> Self {
        kill_orphans(PREFIX);
        let image = test_image();
        let dir =
            image_dir(&image).unwrap_or_else(|| panic!("{} is not installed", image.package()));
        // Short root: the socket path must stay under 104 bytes.
        let tmp = tempfile::Builder::new()
            .prefix("jd")
            .tempdir_in("/tmp")
            .unwrap();
        let paths = JidrPaths::under(tmp.path().join("j"));
        let link = paths.sdk().join(image.sysdir());
        if !dir.starts_with(test_sdk()) {
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&dir, link.trim_end()).unwrap();
        }
        let daemon = Daemon::builder(
            DaemonConfig {
                paths: paths.clone(),
                studio_sdk: Some(test_sdk()),
                idle_timeout: None,
                sdk_repo: None,
            },
            Arc::new(jidr_backend::AospBackend::new()),
        )
        .build()
        .unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(daemon.clone().serve(async {
            let _ = rx.await;
        }));
        while !paths.socket().exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self {
            tmp,
            paths,
            daemon,
            image,
            guards: Mutex::new(Vec::new()),
            stop: Some(tx),
            task: Some(task),
        }
    }

    pub async fn client(&self) -> Conn {
        let mut c = Conn::open(&self.paths.socket()).await;
        // A graceful stop (with its quick-boot save) takes minutes under load.
        c.read_timeout = Duration::from_secs(300);
        c.ok(
            "hello",
            json!({ "version": jidr_daemon::VERSION, "client": "test" }),
        )
        .await;
        c
    }

    /// Kills `avd`'s emulator when the test ends, however it ends.
    pub fn guard(&self, avd: &str) {
        self.guards.lock().unwrap().push(AvdGuard::new(avd));
    }

    /// Creates a device on the test image and guards its emulator.
    pub async fn create(&self, c: &mut Conn, name: &str, profile: &str) -> serde_json::Value {
        let d = c
            .ok(
                "device.create",
                json!({ "name": name, "image": self.image.package(), "profile": profile, "root_mode": "none" }),
            )
            .await;
        self.guard(d["avd"].as_str().unwrap());
        d
    }

    pub async fn stop(mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

/// `Path::trim_end` for a sysdir with a trailing slash.
trait TrimEnd {
    fn trim_end(&self) -> PathBuf;
}

impl TrimEnd for PathBuf {
    fn trim_end(&self) -> PathBuf {
        PathBuf::from(self.to_string_lossy().trim_end_matches('/'))
    }
}
