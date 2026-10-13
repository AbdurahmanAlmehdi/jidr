//! `jidr uninstall [--purge]` (D31): stop everything, remove Jidr's MCP
//! entries, caches and logs, and with `--purge` all Jidr data. Android
//! Studio's SDK and `~/.android` are never touched.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jidr_core::{ErrorCode, JidrError, JidrPaths, JidrResult, ResultExt};
use serde::Serialize;
use serde_json::Value;

/// MCP client configs Jidr may have added itself to.
pub fn mcp_config_paths(home: &Path) -> Vec<PathBuf> {
    let mut v = vec![home.join(".claude.json"), home.join(".cursor/mcp.json")];
    if cfg!(target_os = "macos") {
        v.push(home.join("Library/Application Support/Claude/claude_desktop_config.json"));
    } else {
        v.push(home.join(".config/Claude/claude_desktop_config.json"));
    }
    v
}

/// An entry is Jidr's if it's named `jidr` or runs `jidr mcp`.
pub fn is_jidr_server(name: &str, server: &Value) -> bool {
    if name == "jidr" {
        return true;
    }
    let runs_jidr = server
        .get("command")
        .and_then(Value::as_str)
        .and_then(|c| {
            Path::new(c)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "jidr")
        })
        .unwrap_or(false);
    let mcp_arg = server
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|a| a.first().and_then(Value::as_str) == Some("mcp"));
    runs_jidr && mcp_arg
}

fn strip(servers: Option<&mut Value>, prefix: &str, removed: &mut Vec<String>) {
    let Some(map) = servers.and_then(Value::as_object_mut) else {
        return;
    };
    let names: Vec<String> = map
        .iter()
        .filter(|(n, v)| is_jidr_server(n, v))
        .map(|(n, _)| n.clone())
        .collect();
    for n in names {
        map.shift_remove(&n);
        removed.push(format!("{prefix}{n}"));
    }
}

/// Removes Jidr's servers from `mcpServers` and (Claude Code) from every
/// `projects.<dir>.mcpServers`. Returns what was removed.
pub fn remove_jidr_entries(doc: &mut Value) -> Vec<String> {
    let mut removed = Vec::new();
    strip(doc.get_mut("mcpServers"), "", &mut removed);
    if let Some(projects) = doc.get_mut("projects").and_then(Value::as_object_mut) {
        for (dir, project) in projects.iter_mut() {
            strip(
                project.get_mut("mcpServers"),
                &format!("{dir}: "),
                &mut removed,
            );
        }
    }
    removed
}

#[derive(Debug, Clone, Serialize)]
pub struct McpCleanup {
    pub file: PathBuf,
    pub removed: Vec<String>,
    pub backup: PathBuf,
}

/// Cleans every known MCP config under `home`, backing each changed file
/// up to `<file>.jidr-backup` first. Unparseable files are left alone.
pub fn clean_mcp_configs(home: &Path) -> Vec<McpCleanup> {
    let mut out = Vec::new();
    for file in mcp_config_paths(home) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(mut doc) = serde_json::from_str::<Value>(&text) else {
            tracing::warn!(file = %file.display(), "not JSON; left alone");
            continue;
        };
        let removed = remove_jidr_entries(&mut doc);
        if removed.is_empty() {
            continue;
        }
        let backup = PathBuf::from(format!("{}.jidr-backup", file.display()));
        let Ok(pretty) = serde_json::to_string_pretty(&doc) else {
            continue;
        };
        if std::fs::write(&backup, &text).is_ok() && std::fs::write(&file, pretty + "\n").is_ok() {
            out.push(McpCleanup {
                file,
                removed,
                backup,
            });
        }
    }
    out
}

/// Refuses to delete anything that isn't plainly a Jidr directory.
pub fn safe_to_remove(p: &Path, home: &Path) -> bool {
    let protected = [
        PathBuf::from("/"),
        home.to_path_buf(),
        home.join("Library"),
        home.join("Library/Android"),
        home.join(".android"),
    ];
    let android = [
        home.join("Library/Android"),
        home.join(".android"),
        home.join("Android"),
    ];
    p.is_absolute()
        && p.components().count() > 2
        && !protected.iter().any(|x| x == p)
        && !android.iter().any(|a| p.starts_with(a) || a.starts_with(p))
}

fn remove(p: &Path, home: &Path, removed: &mut Vec<PathBuf>) -> JidrResult<()> {
    if !p.exists() {
        return Ok(());
    }
    if !safe_to_remove(p, home) {
        return Err(JidrError::unexpected(format!(
            "refusing to delete {}",
            p.display()
        )));
    }
    std::fs::remove_dir_all(p).ctx(ErrorCode::Unexpected, format!("deleting {}", p.display()))?;
    removed.push(p.to_path_buf());
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub stopped_devices: Vec<String>,
    pub daemon_stopped: bool,
    pub killed_emulators: Vec<u32>,
    pub mcp: Vec<McpCleanup>,
    pub removed: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
    pub purged: bool,
}

async fn alive(pid: u32) -> bool {
    jidr_backend::aosp::process_alive(pid).await
}

async fn kill(pid: u32, signal: &str) {
    let _ = tokio::process::Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .await;
}

pub async fn run(paths: &JidrPaths, home: &Path, purge: bool) -> JidrResult<Report> {
    let mut report = Report {
        purged: purge,
        ..Default::default()
    };
    let socket = paths.socket();

    // 1. Stop devices gracefully through the daemon, then the daemon.
    if socket.exists() {
        if let Ok(client) = jidr_client::Client::connect(&socket, "cli").await
            && let Ok(list) = client.call("device.list", serde_json::json!({})).await
        {
            for d in list.as_array().into_iter().flatten() {
                let (Some(name), Some(state)) = (d["name"].as_str(), d["status"]["state"].as_str())
                else {
                    continue;
                };
                if !matches!(state, "stopped" | "crashed")
                    && client
                        .call("device.stop", serde_json::json!({ "device": name }))
                        .await
                        .is_ok()
                {
                    report.stopped_devices.push(name.to_string());
                }
            }
        }
        report.daemon_stopped = jidr_client::shutdown_daemon(&socket).await.is_ok();
        let _ = std::fs::remove_file(&socket);
    }

    // 2. Anything still running a Jidr AVD (e.g. after a crashed daemon).
    for d in jidr_backend::discovery::scan() {
        let Some(info) = jidr_backend::adopt::instance_from(&d, &paths.avd_home()) else {
            continue;
        };
        if !alive(info.pid).await {
            continue;
        }
        kill(info.pid, "-TERM").await;
        for _ in 0..50 {
            if !alive(info.pid).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if alive(info.pid).await {
            kill(info.pid, "-KILL").await;
        }
        report.killed_emulators.push(info.pid);
    }

    // 3. MCP entries (D31).
    report.mcp = clean_mcp_configs(home);

    // 4. Files.
    remove(&paths.cache, home, &mut report.removed)?;
    remove(&paths.logs, home, &mut report.removed)?;
    if purge {
        remove(&paths.data, home, &mut report.removed)?;
    } else if paths.data.exists() {
        report.kept.push(paths.data.clone());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_only_jidr_servers_keeping_order() {
        let mut doc = json!({
            "theme": "dark",
            "mcpServers": {
                "github": { "command": "gh-mcp" },
                "jidr": { "command": "jidr", "args": ["mcp"] },
                "phone": { "command": "/opt/homebrew/bin/jidr", "args": ["mcp", "--allow-destructive"] },
                "other-jidr-ish": { "command": "jidr", "args": ["device", "list"] }
            },
            "projects": {
                "/src/app": { "mcpServers": { "jidr": { "command": "jidr", "args": ["mcp"] } }, "x": 1 }
            }
        });
        let removed = remove_jidr_entries(&mut doc);
        assert_eq!(removed, vec!["jidr", "phone", "/src/app: jidr"]);
        let keys: Vec<&String> = doc["mcpServers"].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["github", "other-jidr-ish"],
            "others kept, order preserved"
        );
        assert_eq!(doc["projects"]["/src/app"]["x"], 1);
        let top: Vec<&String> = doc.as_object().unwrap().keys().collect();
        assert_eq!(top, ["theme", "mcpServers", "projects"]);
    }

    #[test]
    fn cleans_files_with_backup() {
        let home = tempfile::tempdir().unwrap();
        let cfg = home.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        let original =
            r#"{"mcpServers":{"jidr":{"command":"jidr","args":["mcp"]},"keep":{"command":"x"}}}"#;
        std::fs::write(&cfg, original).unwrap();
        std::fs::write(home.path().join(".claude.json"), "not json").unwrap();
        let done = clean_mcp_configs(home.path());
        assert_eq!(done.len(), 1);
        assert_eq!(std::fs::read_to_string(&done[0].backup).unwrap(), original);
        let now: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert!(now["mcpServers"].get("jidr").is_none());
        assert!(now["mcpServers"].get("keep").is_some());
        assert_eq!(
            std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            "not json"
        );
        assert!(clean_mcp_configs(home.path()).is_empty(), "idempotent");
    }

    #[test]
    fn never_removes_android_or_home_dirs() {
        let home = Path::new("/Users/me");
        for bad in [
            "/",
            "/Users/me",
            "/Users/me/Library",
            "/Users/me/Library/Android/sdk",
            "/Users/me/.android/avd",
            "relative/dir",
            "/tmp",
        ] {
            assert!(!safe_to_remove(Path::new(bad), home), "{bad}");
        }
        assert!(safe_to_remove(
            Path::new("/Users/me/Library/Application Support/Jidr"),
            home
        ));
        assert!(safe_to_remove(Path::new("/tmp/jidr-e2e.x/data"), home));
    }
}
