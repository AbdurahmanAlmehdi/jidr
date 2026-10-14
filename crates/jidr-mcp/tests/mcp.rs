//! MCP tools against the fake daemon: tool list, generic commands, D5 policy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Mcp;
use jidr_daemon::testing::TestDaemon;
use jidr_mcp::McpOptions;
use serde_json::json;

async fn setup(opts: McpOptions) -> (tempfile::TempDir, TestDaemon, jidr_client::Client, Mcp) {
    let tmp = tempfile::tempdir().unwrap();
    let d = TestDaemon::start(tmp.path()).await.unwrap();
    let human = jidr_client::Client::connect(&d.paths.socket(), "test")
        .await
        .unwrap();
    let agent = jidr_client::Client::connect(&d.paths.socket(), "mcp")
        .await
        .unwrap();
    let mcp = Mcp::connect(agent, opts).await;
    (tmp, d, human, mcp)
}

#[tokio::test]
async fn tools_are_listed_with_annotations() {
    let (_t, d, _h, mcp) = setup(McpOptions::default()).await;
    let tools = mcp.tools().await;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for want in [
        "command_list",
        "command_run",
        "jobs_list",
        "jobs_cancel",
        "screenshot",
        "tap",
        "swipe",
        "type_text",
        "press_key",
        "ui_dump",
        "logcat_tail",
    ] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    for t in &tools {
        let a = t
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no annotations", t.name));
        assert!(
            a.read_only_hint.is_some()
                && a.destructive_hint.is_some()
                && a.open_world_hint.is_some(),
            "{}",
            t.name
        );
        assert_eq!(
            t.input_schema.get("type"),
            Some(&json!("object")),
            "{}",
            t.name
        );
        assert!(t.description.as_ref().is_some_and(|d| d.len() > 10));
    }
    let shot = tools.iter().find(|t| t.name == "screenshot").unwrap();
    assert_eq!(
        shot.annotations.as_ref().unwrap().read_only_hint,
        Some(true)
    );
    d.stop().await;
}

#[tokio::test]
async fn generic_commands_and_jobs() {
    let (_t, d, _h, mcp) = setup(McpOptions::default()).await;
    let list = mcp.ok("command_list", json!({ "prefix": "device." })).await;
    let cmds = list["commands"].as_array().unwrap();
    assert!(
        cmds.iter()
            .all(|c| c["id"].as_str().unwrap().starts_with("device."))
    );
    assert!(
        cmds.iter()
            .any(|c| c["id"] == "device.start" && c["longRunning"] == true)
    );

    let created = mcp
        .ok(
            "command_run",
            json!({ "command": "device.create", "params": { "name": "Bot", "image": "35" } }),
        )
        .await;
    assert_eq!(
        created["agent_owned"], true,
        "devices an agent creates are its own (D5)"
    );
    let started = mcp
        .ok(
            "command_run",
            json!({ "command": "device.start", "params": { "device": "Bot" } }),
        )
        .await;
    assert_eq!(started["status"]["state"], "running", "jobs are waited for");
    let logs = mcp
        .call("logcat_tail", json!({ "device": "Bot", "lines": 5 }))
        .await;
    assert!(common::text(&logs).contains("hello"));

    // A job still running at the timeout comes back as a job.
    let _gate = d.backend.gate_starts();
    mcp.ok(
        "command_run",
        json!({ "command": "device.stop", "params": { "device": "Bot" } }),
    )
    .await;
    let pending = mcp
        .ok(
            "command_run",
            json!({ "command": "device.start", "params": { "device": "Bot" }, "timeout_s": 1 }),
        )
        .await;
    assert_eq!(pending["pending"], true);
    let jobs = mcp.ok("jobs_list", json!({})).await;
    let job = jobs["jobs"][0]["id"].as_str().unwrap().to_string();
    mcp.ok("jobs_cancel", json!({ "job": job })).await;

    // Errors are tool results with a code the agent can act on.
    let e = mcp
        .err(
            "command_run",
            json!({ "command": "device.start", "params": { "device": "nope" } }),
        )
        .await;
    assert_eq!(e["code"], "not_found");
    let e = mcp
        .err(
            "command_run",
            json!({ "command": "device.start", "params": { "devise": "Bot" } }),
        )
        .await;
    assert_eq!(e["code"], "usage");
    let e = mcp
        .err("command_run", json!({ "command": "nope.nope" }))
        .await;
    assert!(e["message"].as_str().unwrap().contains("command_list"));
    for blocked in [
        "daemon.shutdown",
        "events.subscribe",
        "logcat.follow",
        "hello",
    ] {
        let e = mcp.err("command_run", json!({ "command": blocked })).await;
        assert!(
            e["message"].as_str().unwrap().contains("can't be called"),
            "{blocked}"
        );
    }
    let e = mcp.err("tap", json!({ "device": "Bot", "x": 1 })).await;
    assert_eq!(e["code"], "usage", "missing y");
    d.stop().await;
}

#[tokio::test]
async fn destructive_policy_d5() {
    let (_t, d, human, mcp) = setup(McpOptions::default()).await;
    // A device the user made: the agent may use it but not destroy it.
    human
        .call("device.create", json!({ "name": "Users", "image": "35" }))
        .await
        .unwrap();
    mcp.ok(
        "command_run",
        json!({ "command": "device.start", "params": { "device": "Users" } }),
    )
    .await;
    mcp.ok(
        "command_run",
        json!({ "command": "device.stop", "params": { "device": "Users" } }),
    )
    .await;
    for cmd in ["device.delete", "device.reset"] {
        let e = mcp
            .err(
                "command_run",
                json!({ "command": cmd, "params": { "device": "Users" } }),
            )
            .await;
        assert_eq!(e["data"]["policy"], "D5", "{cmd}: {e}");
    }
    // Its own device: allowed.
    mcp.ok(
        "command_run",
        json!({ "command": "device.create", "params": { "name": "Mine", "image": "35" } }),
    )
    .await;
    mcp.ok(
        "command_run",
        json!({ "command": "device.delete", "params": { "device": "Mine" } }),
    )
    .await;
    // Explicitly not agent-owned → refused even though the agent made it.
    mcp.ok("command_run", json!({ "command": "device.create", "params": { "name": "Shared", "image": "35", "agent_owned": false } })).await;
    mcp.err(
        "command_run",
        json!({ "command": "device.delete", "params": { "device": "Shared" } }),
    )
    .await;
    d.stop().await;

    // With --allow-destructive, anything goes.
    let (_t, d, human, mcp) = setup(McpOptions {
        allow_destructive: true,
    })
    .await;
    human
        .call("device.create", json!({ "name": "Users", "image": "35" }))
        .await
        .unwrap();
    mcp.ok(
        "command_run",
        json!({ "command": "device.delete", "params": { "device": "Users" } }),
    )
    .await;
    d.stop().await;
}
