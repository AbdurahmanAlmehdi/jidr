//! The `jidr` binary against an in-process fake daemon (`TestDaemon`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;

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

/// Gap B4: `jidr settings` changes the daemon's defaults, and `device create`
/// without `--root` follows them.
#[test]
fn settings_set_the_default_root_mode() {
    let e = Env::new();
    assert_eq!(e.ok(&["settings", "get"])["default_root_mode"], "adb");
    let s = e.ok(&[
        "settings",
        "set",
        "--default-root",
        "none",
        "--quick-boot",
        "false",
    ]);
    assert_eq!(s["default_root_mode"], "none");
    assert_eq!(s["quick_boot"], false);
    let d = e.ok(&["device", "create", "Plain", "--image", "35"]);
    assert_eq!(d["root"]["mode"], "none");
}
