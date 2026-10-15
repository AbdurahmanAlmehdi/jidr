//! Emulator test setup: a real daemon (AOSP backend + the widgets provider)
//! in a temp Jidr home, one device booted from `JIDR_TEST_IMAGE` using the
//! `JIDR_TEST_SDK` SDK read-only (D3). Boots serialise on a file lock so a
//! parallel nextest run doesn't start several emulators at once.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jidr_backend::AospBackend;
use jidr_client::Client;
use jidr_core::{ConnectInfo, JidrPaths};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

pub fn test_sdk() -> PathBuf {
    std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap().join("Library/Android/sdk"))
}

pub fn test_image() -> String {
    std::env::var("JIDR_TEST_IMAGE").unwrap_or_else(|_| {
        format!(
            "system-images;android-31;google_apis;{}",
            jidr_sdk::host::host_abi()
        )
    })
}

pub struct Env {
    pub tmp: tempfile::TempDir,
    pub paths: JidrPaths,
    pub client: Client,
    pub device: String,
    pub info: ConnectInfo,
    pub daemon: Arc<Daemon>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<jidr_core::JidrResult<()>>>,
    _lock: std::fs::File,
}

impl Env {
    /// Creates and cold boots `name` (720×1280 small phone, no root).
    pub async fn boot(name: &str) -> Env {
        let lock =
            std::fs::File::create(std::env::temp_dir().join("jidr-widgets-emulator.lock")).unwrap();
        lock.lock().unwrap();
        let tmp = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        // Short root: the daemon socket path must fit in sun_path.
        let paths = JidrPaths::under(tmp.path().join("j"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            repo.join("repository2-3.xml"),
            "<sdk:sdk-repository xmlns:sdk=\"x\"/>",
        )
        .unwrap();
        let daemon = Daemon::builder(
            DaemonConfig {
                paths: paths.clone(),
                studio_sdk: Some(test_sdk()),
                idle_timeout: None,
                sdk_repo: Some(format!("file://{}/", repo.display())),
            },
            Arc::new(AospBackend::new()),
        )
        .provider(jidr_widgets::provider())
        .build()
        .unwrap();
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
        let client = Client::connect(&paths.socket(), "test").await.unwrap();
        client
            .call(
                "device.create",
                json!({ "name": name, "image": test_image(), "profile": "small-phone-5", "root_mode": "none" }),
            )
            .await
            .unwrap();
        let t = Instant::now();
        client
            .run(
                "device.start",
                json!({ "device": name, "cold_boot": true }),
                |_| {},
            )
            .await
            .unwrap();
        println!("{name}: booted in {:.1} s", t.elapsed().as_secs_f64());
        let info: ConnectInfo = client
            .call_as("device.connect_info", json!({ "device": name }))
            .await
            .unwrap();
        Env {
            tmp,
            paths,
            client,
            device: name.into(),
            info,
            daemon,
            shutdown: Some(tx),
            task: Some(task),
            _lock: lock,
        }
    }

    pub async fn call(&self, method: &str, mut params: Value) -> Value {
        params["device"] = json!(self.device);
        match self.client.call(method, params.clone()).await {
            Ok(v) => v,
            Err(e) => panic!("{method} {params}: {e}"),
        }
    }

    /// Calls a long-running command and follows its job.
    pub async fn run(&self, method: &str, mut params: Value) -> Value {
        params["device"] = json!(self.device);
        match self.client.run(method, params.clone(), |_| {}).await {
            Ok(v) => v,
            Err(e) => panic!("{method} {params}: {e}"),
        }
    }

    /// `adb -s <serial> …`: (stdout, stderr). adb reports "device offline"
    /// and closed transports on stderr.
    pub fn adb_out(&self, args: &[&str]) -> (String, String) {
        let out = std::process::Command::new(&self.info.adb_path)
            .arg("-s")
            .arg(&self.info.adb_serial)
            .args(args)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    pub fn adb(&self, args: &[&str]) -> String {
        self.adb_out(args).0
    }

    pub fn shell(&self, cmd: &str) -> String {
        self.adb(&["shell", cmd])
    }

    /// Polls `f` until it returns true (30 s). The Linux KVM runners use
    /// software rendering, and a snapshot restore or a telephony signal
    /// refresh there can take longer than 10 s.
    pub async fn eventually(&self, what: &str, f: impl Fn(&Env) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if f(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        panic!("timed out waiting for: {what}");
    }

    /// Like `eventually`, on the output of a device shell command. On timeout
    /// the panic shows the last output's lines that contain any of `show`, and
    /// `context`, so a CI failure says what the device reported.
    pub async fn eventually_shell(
        &self,
        what: &str,
        cmd: &str,
        show: &[&str],
        context: &str,
        ok: impl Fn(&str) -> bool,
    ) {
        self.eventually_shell_with(what, cmd, show, |_| context.to_string(), ok)
            .await;
    }

    /// Like `eventually_shell`, with context read from the device at the
    /// timeout (e.g. the console's `gsm status`).
    pub async fn eventually_shell_with(
        &self,
        what: &str,
        cmd: &str,
        show: &[&str],
        context: impl Fn(&Env) -> String,
        ok: impl Fn(&str) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let (mut out, mut err) = (String::new(), String::new());
        while Instant::now() < deadline {
            (out, err) = self.adb_out(&["shell", cmd]);
            if ok(&out) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let seen: Vec<&str> = out
            .lines()
            .filter(|l| show.iter().any(|k| l.contains(k)))
            .collect();
        let (state, state_err) = self.adb_out(&["get-state"]);
        panic!(
            "timed out waiting for: {what}\n`{cmd}` showed:\n{}\nstderr: {}\nadb get-state: {} {}\n{}",
            seen.join("\n"),
            err.trim(),
            state.trim(),
            state_err.trim(),
            context(self)
        );
    }

    /// Stops the device and the daemon.
    pub async fn stop(mut self) {
        let _ = self
            .client
            .call("device.stop", json!({ "device": self.device }))
            .await;
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

impl Drop for Env {
    /// Never leak an emulator, even when a test panics.
    fn drop(&mut self) {
        if self.shutdown.is_some() {
            let _ = std::process::Command::new(&self.info.adb_path)
                .args(["-s", &self.info.adb_serial, "emu", "kill"])
                .output();
        }
    }
}
