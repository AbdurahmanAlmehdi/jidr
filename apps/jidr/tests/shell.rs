//! `jidr shell` (issue #73) against the in-process fake daemon: piped lines
//! and script files, the current device, and failures with line numbers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Env;
use serde_json::Value;

/// Each stdout line of a `--json` shell run, parsed.
fn envelopes(o: &std::process::Output) -> Vec<Value> {
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad JSON {l:?}: {e}")))
        .collect()
}

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn piped_lines_run_with_the_current_device() {
    let e = Env::new();
    let o = e.jidr_stdin(
        &["--json", "shell"],
        r#"
# A device to work on.
device create Phone --image 35
device create Tablet --image 35
use Phone
device start          # fills in Phone
call device.details   # params get "device": "Phone"
jidr device details Tablet
use
device list
"#,
    );
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    let v = envelopes(&o);
    assert_eq!(v.len(), 8, "{v:#?}");
    assert!(
        v.iter().all(|e| e["jidr_json"] == 1 && e["ok"] == true),
        "{v:#?}"
    );
    assert_eq!(v[2]["result"]["device"], "Phone");
    assert_eq!(v[3]["result"][0]["name"], "Phone");
    assert_eq!(v[3]["result"][0]["status"]["state"], "running");
    assert_eq!(v[4]["result"]["device"]["name"], "Phone");
    assert_eq!(v[5]["result"]["device"]["name"], "Tablet");
    assert_eq!(v[6]["result"]["device"], Value::Null);
    let list = v[7]["result"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    // State lives in the daemon: the one-shot CLI sees the same device.
    assert_eq!(
        e.ok(&["device", "details", "Phone"])["device"]["status"]["state"],
        "running"
    );
}

#[test]
fn a_script_stops_at_its_first_failure() {
    let e = Env::new();
    std::fs::create_dir_all(&e.home).unwrap();
    let script = e.home.join("setup.jidr");
    std::fs::write(
        &script,
        "#!/usr/bin/env -S jidr shell\ndevice create P --image 35\n\n# no such device: exit 3\ndevice start nope\ndevice create Never --image 35\n",
    )
    .unwrap();
    let o = e.jidr(&["shell", script.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(3), "stderr: {}", stderr(&o));
    let err = stderr(&o);
    assert!(
        err.contains("setup.jidr:5: stopped here (exit code 3)"),
        "{err}"
    );
    let names: Vec<_> = e
        .ok(&["device", "list"])
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["P"]);

    // Usage errors keep their exit code (2), clap's included; `use` of an
    // unknown device is not found (3); `exit N` ends with N.
    for (input, code) in [
        ("device frobnicate\n", 2),
        ("call device.list '{oops\n", 2),
        ("use Ghost\n", 3),
        ("device list\nexit 7\ndevice start nope\n", 7),
        ("shell\n", 2),
    ] {
        let o = e.jidr_stdin(&["shell"], input);
        assert_eq!(o.status.code(), Some(code), "{input:?}: {}", stderr(&o));
    }
    let o = e.jidr(&["shell", "/nonexistent/x.jidr"]);
    assert_eq!(o.status.code(), Some(3));
}

#[test]
fn source_runs_a_file_inside_the_shell() {
    let e = Env::new();
    std::fs::create_dir_all(&e.home).unwrap();
    let inner = e.home.join("inner.jidr");
    std::fs::write(&inner, "device create S --image 35\nuse S\n").unwrap();
    let o = e.jidr_stdin(
        &["--json", "shell"],
        &format!("source '{}'\ndevice start\n", inner.display()),
    );
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    let v = envelopes(&o);
    assert_eq!(v.last().unwrap()["result"][0]["status"]["state"], "running");

    // A failure inside a sourced file names both files' lines.
    std::fs::write(&inner, "device list\ndevice start nope\n").unwrap();
    let o = e.jidr_stdin(
        &["shell"],
        &format!("device list\nsource {}\n", inner.display()),
    );
    assert_eq!(o.status.code(), Some(3));
    let err = stderr(&o);
    assert!(err.contains("inner.jidr:2: stopped here"), "{err}");
    assert!(err.contains("<stdin>:2: stopped here"), "{err}");
}

#[test]
fn human_output_matches_the_cli() {
    let e = Env::new();
    e.ok(&["device", "create", "Phone", "--image", "35"]);
    let o = e.jidr_stdin(&["shell"], "device list\nhelp\n");
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(&o));
    let one_shot = e.jidr(&["device", "list"]);
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        text.starts_with(&*String::from_utf8_lossy(&one_shot.stdout)),
        "{text}"
    );
    assert!(
        text.contains("use [device]") && text.contains("proxy"),
        "{text}"
    );
}
