//! Test helpers shared by the daemon, client, CLI and MCP tests: an
//! in-process daemon on a temp socket with the fake backend and a fake
//! Android Studio SDK. Not used by the product.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jidr_backend::fake::FakeBackend;
use jidr_core::provider::CommandProvider;
use jidr_core::{ImageRef, JidrPaths, JidrResult};
use jidr_sdk::host::HostMemory;

use crate::{Daemon, DaemonConfig};

/// The image the fake SDK provides (API 35 `google_apis`, host ABI), plus its
/// Play Store twin.
pub fn fake_image() -> ImageRef {
    ImageRef {
        api: 35,
        tag: "google_apis".into(),
        abi: jidr_sdk::host::host_abi().into(),
    }
}

fn write_exec(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// A Studio-like SDK: an `emulator` stub, an `adb` that answers like a
/// device, and two complete images.
pub fn fake_studio(root: &Path) -> std::io::Result<PathBuf> {
    write_exec(&root.join("emulator/emulator"), "#!/bin/sh\nexit 0\n")?;
    write_exec(
        &root.join("platform-tools/adb"),
        "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\ncase \"$1\" in\n  install) echo Success;;\n  uninstall) echo Success;;\n  logcat) echo 'I/fake: hello';;\n  shell) echo 'fake shell';;\n  *) echo ok;;\nesac\n",
    )?;
    for img in [
        fake_image(),
        ImageRef {
            tag: "google_apis_playstore".into(),
            ..fake_image()
        },
    ] {
        let dir = root.join(img.sysdir());
        std::fs::create_dir_all(&dir)?;
        for f in ["source.properties", "system.img", "ramdisk.img"] {
            std::fs::write(dir.join(f), b"x")?;
        }
    }
    Ok(root.to_path_buf())
}

/// A daemon serving on `<root>/j/data/daemon.sock`.
#[derive(Debug)]
pub struct TestDaemon {
    pub paths: JidrPaths,
    pub studio: PathBuf,
    pub backend: Arc<FakeBackend>,
    pub daemon: Arc<Daemon>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<JidrResult<()>>>,
}

impl TestDaemon {
    /// Starts serving and waits for the socket. The host pretends to have
    /// 1 TB of RAM, so the budget (D42) never gets in the way.
    pub async fn start(root: &Path) -> JidrResult<Self> {
        Self::start_with_memory(
            root,
            HostMemory {
                total_mb: 1 << 20,
                available_mb: 1 << 19,
            },
        )
        .await
    }

    /// Like [`start`](Self::start), on a host with this much RAM.
    pub async fn start_with_memory(root: &Path, memory: HostMemory) -> JidrResult<Self> {
        Self::start_inner(root, memory, Vec::new()).await
    }

    /// Like [`start`](Self::start), with extra command providers (e.g. the
    /// widgets provider, to test it against the fake backend).
    pub async fn start_with_providers(
        root: &Path,
        providers: Vec<Arc<dyn CommandProvider>>,
    ) -> JidrResult<Self> {
        Self::start_inner(
            root,
            HostMemory {
                total_mb: 1 << 20,
                available_mb: 1 << 19,
            },
            providers,
        )
        .await
    }

    async fn start_inner(
        root: &Path,
        memory: HostMemory,
        providers: Vec<Arc<dyn CommandProvider>>,
    ) -> JidrResult<Self> {
        let paths = JidrPaths::under(root.join("j"));
        let studio = fake_studio(&root.join("studio"))
            .map_err(|e| jidr_core::JidrError::unexpected(format!("fake SDK: {e}")))?;
        // An empty local SDK repository: no test ever reaches the network.
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo)
            .and_then(|()| {
                std::fs::write(
                    repo.join("repository2-3.xml"),
                    "<sdk:sdk-repository xmlns:sdk=\"x\"/>",
                )
            })
            .map_err(|e| jidr_core::JidrError::unexpected(format!("fake repo: {e}")))?;
        let backend = Arc::new(FakeBackend::new());
        let mut builder = Daemon::builder(
            DaemonConfig {
                paths: paths.clone(),
                studio_sdk: Some(studio.clone()),
                idle_timeout: None,
                sdk_repo: Some(format!("file://{}/", repo.display())),
            },
            backend.clone(),
        )
        .host_memory(memory);
        for p in providers {
            builder = builder.provider(p);
        }
        let daemon = builder.build()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let d = daemon.clone();
        let task = tokio::spawn(async move {
            d.serve(async {
                let _ = rx.await;
            })
            .await
        });
        for _ in 0..500 {
            if paths.socket().exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(Self {
            paths,
            studio,
            backend,
            daemon,
            shutdown: Some(tx),
            task: Some(task),
        })
    }

    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}
