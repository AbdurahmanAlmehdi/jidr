//! The `jidr` binary against an in-process fake daemon (`TestDaemon`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use assert_cmd::Command;
use jidr_daemon::testing::TestDaemon;
use serde_json::Value;

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    daemon: Option<TestDaemon>,
    home: PathBuf,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let daemon = rt.block_on(TestDaemon::start(tmp.path())).unwrap();
        let home = tmp.path().join("j");
        Self {
            _tmp: tmp,
            rt,
            daemon: Some(daemon),
            home,
        }
    }

    fn backend(&self) -> &jidr_backend::fake::FakeBackend {
        &self.daemon.as_ref().unwrap().backend
    }

    fn jidr(&self, args: &[&str]) -> std::process::Output {
        Command::cargo_bin("jidr")
            .unwrap()
            .args(args)
            .env("JIDR_HOME", &self.home)
            // Never spawn a real daemon from tests.
            .env("JIDR_DAEMON_BIN", "/nonexistent/jidr")
            .env_remove("JIDR_RESTART_STALE")
            .timeout(std::time::Duration::from_secs(30))
            .output()
            .unwrap()
    }

    /// Runs with --json; returns (exit code, envelope).
    fn json(&self, args: &[&str]) -> (i32, Value) {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let o = self.jidr(&all);
        let stdout = String::from_utf8_lossy(&o.stdout);
        let v: Value = serde_json::from_str(stdout.lines().last().unwrap_or("null"))
            .unwrap_or_else(|e| {
                panic!(
                    "{args:?}: bad JSON {stdout:?} ({e}); stderr {}",
                    String::from_utf8_lossy(&o.stderr)
                )
            });
        assert_eq!(
            v["jidr_json"],
            1,
            "{args:?}: stdout {stdout:?} stderr {}",
            String::from_utf8_lossy(&o.stderr)
        );
        (o.status.code().unwrap_or(-1), v)
    }

    fn ok(&self, args: &[&str]) -> Value {
        let (code, v) = self.json(args);
        assert_eq!(code, 0, "{args:?} → {v}");
        assert_eq!(v["ok"], true);
        v["result"].clone()
    }

    fn fail(&self, args: &[&str], exit: i32) -> Value {
        let (code, v) = self.json(args);
        assert_eq!(code, exit, "{args:?} → {v}");
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["exit_code"], exit);
        v["error"].clone()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if let Some(d) = self.daemon.take() {
            self.rt.block_on(d.stop());
        }
    }
}

#[test]
fn device_lifecycle_json() {
    let e = Env::new();
    let d = e.ok(&[
        "device", "create", "Phone", "--image", "35", "--ram", "3072",
    ]);
    assert_eq!(d["name"], "Phone");
    assert_eq!(d["root"]["mode"], "adb");
    assert_eq!(e.ok(&["device", "list"]).as_array().unwrap().len(), 1);
    let started = e.ok(&["device", "start", "Phone"]);
    assert_eq!(started[0]["status"]["state"], "running");
    let ci = e.ok(&["device", "connect-info", "Phone"]);
    assert!(ci["grpc_token"].as_str().unwrap().starts_with("fake-token"));

    let apk = e.home.join("app.apk");
    std::fs::create_dir_all(&e.home).unwrap();
    std::fs::write(&apk, b"apk").unwrap();
    e.ok(&["install", "Phone", apk.to_str().unwrap()]);
    let log = e.ok(&["logcat", "Phone", "-n", "5"]);
    assert!(log["text"].as_str().unwrap().contains("hello"));
    e.ok(&["logcat", "Phone", "--clear"]);

    let details = e.ok(&["device", "details", "Phone"]);
    assert_eq!(details["config"]["hw.ramSize"], "3072");
    e.ok(&["device", "stop", "Phone"]);
    e.ok(&[
        "device", "edit", "Phone", "--rename", "Phone2", "--cpus", "2",
    ]);
    e.ok(&["device", "clone", "Phone2", "Copy"]);
    e.ok(&["device", "reset", "Copy", "--yes"]);
    let deleted = e.ok(&["device", "delete", "--all", "--yes"]);
    assert_eq!(deleted.as_array().unwrap().len(), 2);
    assert_eq!(e.ok(&["device", "list"]), serde_json::json!([]));
}

#[test]
fn exit_codes_are_error_codes() {
    let e = Env::new();
    let err = e.fail(&["device", "start", "nope"], 3);
    assert_eq!(err["code"], "not_found");
    e.fail(&["device", "create", "X", "--image", "29"], 6);
    e.fail(&["flash", "X", "/tmp/a.zip"], 2);
    e.fail(&["call", "device.list", "{not json"], 2);
    let err = e.fail(&["call", "device.start", r#"{"devise":"x"}"#], 2);
    assert!(err["message"].as_str().unwrap().contains("devise"));
    e.ok(&["device", "create", "P", "--image", "35"]);
    // Destructive without --yes, off a terminal.
    e.fail(&["device", "delete", "P"], 2);
    // Root commands aren't registered in this build yet.
    let err = e.fail(&["root", "set", "P", "magisk"], 2);
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("aren't available"),
        "{err}"
    );
    // clap usage errors exit 2 too.
    assert_eq!(e.jidr(&["device", "frobnicate"]).status.code(), Some(2));
}

#[test]
fn busy_and_timeout() {
    let e = Env::new();
    e.ok(&["device", "create", "Slow", "--image", "35"]);
    let gate = e.backend().gate_starts();
    let pending = e.ok(&["device", "start", "Slow", "--no-wait"]);
    let job = pending[0]["job"].as_str().unwrap().to_string();
    let err = e.fail(&["device", "stop", "Slow"], 4);
    assert_eq!(err["data"]["holder"], job);
    e.ok(&["jobs", "cancel", &job]);
    let (code, _) = e.json(&["jobs", "wait", &job]);
    assert_ne!(code, 0, "a cancelled job is not a success");

    // --timeout cancels the boot and exits 5 (boot_timeout).
    let err = e.fail(&["device", "start", "Slow", "--timeout", "1"], 5);
    assert!(err["message"].as_str().unwrap().contains("did not boot"));
    let d = e.ok(&["device", "details", "Slow"]);
    assert_eq!(d["device"]["status"]["state"], "stopped");
    gate.add_permits(10);
    e.ok(&["device", "start", "--all"]);
    assert_eq!(
        e.ok(&["device", "stop", "--all"]).as_array().unwrap().len(),
        1
    );
}

#[test]
fn discovery_commands() {
    let e = Env::new();
    let cmds = e.ok(&["commands"]);
    assert!(
        cmds.as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "device.start")
    );
    assert_eq!(e.ok(&["call", "device.list"]), serde_json::json!([]));
    assert!(!e.ok(&["profile", "list"]).as_array().unwrap().is_empty());
    let imgs = e.ok(&["image", "list", "--installed"]);
    assert!(imgs.as_array().unwrap().iter().any(|i| i["api"] == 35));
    let status = e.ok(&["daemon", "status"]);
    assert_eq!(status["protocol"], "0.1");
    e.ok(&["sdk", "status"]);
    assert_eq!(e.ok(&["jobs", "list"]), serde_json::json!([]));
}

#[test]
fn human_output_and_completions() {
    let e = Env::new();
    let o = e.jidr(&["device", "list"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("No devices"));
    e.ok(&[
        "device",
        "create",
        "Tab",
        "--image",
        "35",
        "--profile",
        "tablet-11",
    ]);
    let o = e.jidr(&["device", "list"]);
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.starts_with("NAME"), "{text}");
    assert!(
        text.contains("Tab") && text.contains("35:google_apis"),
        "{text}"
    );
    let o = e.jidr(&["device", "start", "nope"]);
    assert_eq!(o.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("error: "));
    let o = e.jidr(&["completions", "zsh"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("#compdef jidr"));
}

#[test]
fn daemon_stop() {
    let e = Env::new();
    let v = e.ok(&["daemon", "stop"]);
    assert_eq!(v["stopped"], true);
    let v = e.ok(&["daemon", "stop"]);
    assert_eq!(v["stopped"], false);
}
