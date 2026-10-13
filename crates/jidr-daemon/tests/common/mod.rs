//! A daemon on a temp socket with the fake backend and a fake Studio SDK.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jidr_backend::fake::FakeBackend;
use jidr_core::{ImageRef, JidrPaths};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub struct Harness {
    pub tmp: tempfile::TempDir,
    pub paths: JidrPaths,
    pub backend: Arc<FakeBackend>,
    pub daemon: Arc<Daemon>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

pub fn image() -> ImageRef {
    ImageRef {
        api: 35,
        tag: "google_apis".into(),
        abi: jidr_sdk::host::host_abi().into(),
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A Studio-like SDK with an emulator, an adb that echoes, and one image.
pub fn fake_studio(root: &Path) -> PathBuf {
    write_exec(&root.join("emulator/emulator"), "#!/bin/sh\nexit 0\n");
    write_exec(
        &root.join("platform-tools/adb"),
        "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\ncase \"$1\" in\n  install) echo Success;;\n  logcat) echo 'I/fake: hello';;\n  *) echo ok;;\nesac\n",
    );
    for img in [
        image(),
        ImageRef {
            tag: "google_apis_playstore".into(),
            ..image()
        },
    ] {
        let dir = root.join(img.sysdir());
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["source.properties", "system.img", "ramdisk.img"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
    }
    root.to_path_buf()
}

impl Harness {
    pub async fn start() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let paths = JidrPaths::under(tmp.path().join("j"));
        let studio = fake_studio(&tmp.path().join("studio"));
        let backend = Arc::new(FakeBackend::new());
        let daemon = Daemon::builder(
            DaemonConfig {
                paths: paths.clone(),
                studio_sdk: Some(studio),
            },
            backend.clone(),
        )
        .build()
        .unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let d = daemon.clone();
        let task = tokio::spawn(async move {
            d.serve(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
        });
        for _ in 0..200 {
            if paths.socket().exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Self {
            tmp,
            paths,
            backend,
            daemon,
            shutdown: Some(tx),
            task: Some(task),
        }
    }

    pub async fn conn(&self) -> Conn {
        Conn::open(&self.paths.socket()).await
    }

    /// A connection that already said hello.
    pub async fn client(&self) -> Conn {
        let mut c = self.conn().await;
        let r = c
            .call(
                "hello",
                json!({ "version": jidr_daemon::VERSION, "client": "test" }),
            )
            .await;
        assert!(r.get("result").is_some(), "{r}");
        c
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

pub struct Conn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next: u64,
}

impl Conn {
    pub async fn open(path: &Path) -> Self {
        let s = UnixStream::connect(path).await.unwrap();
        let (r, w) = s.into_split();
        Self {
            reader: BufReader::new(r),
            writer: w,
            next: 1,
        }
    }

    pub async fn send_raw(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    pub async fn read(&mut self) -> Value {
        let mut l = String::new();
        tokio::time::timeout(Duration::from_secs(10), self.reader.read_line(&mut l))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_str(&l).unwrap()
    }

    /// Sends a request and returns the full response object.
    pub async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send_raw(&req.to_string()).await;
        loop {
            let v = self.read().await;
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    pub async fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params).await;
        r.get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} failed: {r}"))
    }

    pub async fn err(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params).await;
        r.get("error")
            .cloned()
            .unwrap_or_else(|| panic!("{method} unexpectedly succeeded: {r}"))
    }
}
