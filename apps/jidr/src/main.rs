//! `jidr`: the Jidr CLI, daemon and MCP server.

mod cli;
mod output;
mod shell;
mod uninstall;

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{CommandFactory, Parser};
use cli::{
    AudioCmd, BaseCmd, CacheCmd, CertCmd, Cli, Cmd, DaemonCmd, DeviceCmd, Hardware, ImageCmd,
    JobsCmd, NetworkCmd, PoolCmd, ProfileCmd, ProxyCmd, RootCmd, SdkCmd, Selection, SettingsCmd,
    SnapshotCmd,
};
use jidr_client::{Client, SpawnOptions};
use jidr_core::{ErrorCode, JidrError, JidrPaths, JidrResult, Job, JobState};
use output::Out;
use serde_json::{Map, Value, json};
use uuid::Uuid;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = Out {
        json: cli.json,
        quiet: cli.quiet,
    };
    let code = match &cli.cmd {
        Cmd::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), "jidr", &mut std::io::stdout());
            0
        }
        Cmd::Daemon {
            action: None | Some(DaemonCmd::Run),
        } => run_daemon(out),
        Cmd::Shell { script } => shell::main(&cli, script.as_deref(), out),
        _ => match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => match rt.block_on(run(cli, out)) {
                Ok(()) => 0,
                Err(e) => out.err(&e),
            },
            Err(e) => out.err(&JidrError::unexpected(format!("tokio runtime: {e}"))),
        },
    };
    ExitCode::from(code)
}

fn interactive(out: Out) -> bool {
    !out.json && std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Asks a yes/no question on the terminal.
fn ask(question: &str, default_yes: bool) -> bool {
    eprint!(
        "{question} {} ",
        if default_yes { "[Y/n]" } else { "[y/N]" }
    );
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    match answer.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        a => a == "y" || a == "yes",
    }
}

fn confirm(out: Out, yes: bool, what: &str) -> JidrResult<()> {
    if yes {
        return Ok(());
    }
    if interactive(out) {
        return if ask(&format!("{what}?"), false) {
            Ok(())
        } else {
            Err(JidrError::usage("cancelled"))
        };
    }
    Err(JidrError::usage(format!("{what}: pass --yes to confirm")))
}

async fn connect(paths: &JidrPaths, restart_stale: bool, out: Out) -> JidrResult<Client> {
    let mut opts = SpawnOptions {
        program: std::env::current_exe().ok(),
        restart_stale,
        timeout: None,
    };
    match Client::connect_or_spawn(paths, "cli", &opts).await {
        Err(e) if e.code == ErrorCode::DaemonStale && interactive(out) => {
            eprintln!("{}", e.message);
            if !ask("Restart the daemon? Running devices keep running.", true) {
                return Err(e);
            }
            opts.restart_stale = true;
            Client::connect_or_spawn(paths, "cli", &opts).await
        }
        other => other,
    }
}

/// Follows a `{job, pending}` answer to its end, with an optional deadline
/// after which the job is cancelled.
async fn follow(
    client: &Client,
    out: Out,
    v: Value,
    timeout: Option<Duration>,
) -> JidrResult<Value> {
    let Some(job) = jidr_client::pending_job(&v) else {
        return Ok(v);
    };
    let deadline = timeout.map(|t| tokio::time::Instant::now() + t);
    let mut last: Option<(Option<f32>, String)> = None;
    loop {
        let j: Job = client.wait_job(job, Some(Duration::from_secs(1))).await?;
        if last.as_ref() != Some(&(j.progress, j.message.clone())) {
            out.progress(&j);
            last = Some((j.progress, j.message.clone()));
        }
        match j.state {
            JobState::Succeeded => {
                out.progress_done();
                return Ok(j.result.unwrap_or(Value::Null));
            }
            JobState::Failed => {
                out.progress_done();
                return Err(j
                    .error
                    .unwrap_or_else(|| JidrError::unexpected(format!("{} failed", j.kind))));
            }
            JobState::Cancelled => {
                out.progress_done();
                return Err(jidr_core::cancel::cancelled());
            }
            JobState::Pending | JobState::Running => {}
        }
        if let Some(d) = deadline
            && tokio::time::Instant::now() >= d
        {
            out.progress_done();
            let _ = client.cancel_job(job).await;
            let _ = client.wait_job(job, Some(Duration::from_secs(30))).await;
            let secs = timeout.map(|t| t.as_secs()).unwrap_or(0);
            return Err(if j.kind == "device.start" {
                JidrError::boot_timeout(format!(
                    "the device did not boot within {secs} s; the boot was cancelled"
                ))
            } else {
                JidrError::unexpected(format!(
                    "{} did not finish within {secs} s; it was cancelled",
                    j.kind
                ))
            }
            .with_data(json!({ "job": job })));
        }
    }
}

/// Calls a method and follows its job if it starts one.
async fn exec(client: &Client, out: Out, method: &str, params: Value) -> JidrResult<Value> {
    let v = client
        .call(method, params)
        .await
        .map_err(|e| missing_provider(method, e))?;
    follow(client, out, v, None).await
}

/// Commands whose provider isn't registered (the CLI's in-process test
/// daemon has only the core one); say so clearly.
fn missing_provider(method: &str, e: JidrError) -> JidrError {
    if e.code == ErrorCode::Usage && e.message.starts_with("unknown method") {
        let area = method.split('.').next().unwrap_or(method);
        return JidrError::usage(format!(
            "`{area}` commands aren't available in this build of Jidr yet ({method})"
        ));
    }
    e
}

/// Runs `f`; if it fails on unaccepted licenses, shows/accepts them and retries once.
async fn with_licenses<F, Fut>(client: &Client, out: Out, accept: bool, f: F) -> JidrResult<Value>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = JidrResult<Value>>,
{
    let err = match f().await {
        Ok(v) => return Ok(v),
        Err(e) => e,
    };
    let Some(pending) = err
        .data
        .as_ref()
        .and_then(|d| d.get("pending_licenses"))
        .and_then(Value::as_array)
        .cloned()
    else {
        return Err(err);
    };
    for lic in &pending {
        let (id, sha) = (
            lic["id"].as_str().unwrap_or_default(),
            lic["sha256"].as_str().unwrap_or_default(),
        );
        if accept {
            out.info(&format!(
                "Accepting license {id} (sha256 {sha}) for {}",
                lic["packages"]
            ));
        } else if interactive(out) {
            eprintln!(
                "\n----- License {id} -----\n{}\n----- end of {id} (sha256 {sha}) -----",
                lic["text"].as_str().unwrap_or_default()
            );
            if !ask(&format!("Accept license {id}?"), false) {
                return Err(JidrError::sdk(format!("license {id} was not accepted")));
            }
        } else {
            return Err(JidrError::sdk(format!(
                "{}; read them with `jidr sdk licenses …` and rerun with --accept-licenses",
                err.message
            ))
            .with_data(json!({ "pending_licenses": pending })));
        }
        client
            .call("sdk.accept_license", json!({ "id": id, "sha256": sha }))
            .await?;
    }
    f().await
}

fn hw_params(m: &mut Map<String, Value>, hw: &Hardware) {
    for (k, v) in [
        ("ram_mb", hw.ram_mb),
        ("cpus", hw.cpus),
        ("width", hw.width),
        ("height", hw.height),
        ("density", hw.density),
    ] {
        if let Some(v) = v {
            m.insert(k.into(), json!(v));
        }
    }
}

/// Resolves `names` / `--all` to device names, filtered by `keep` for `--all`.
async fn select(
    client: &Client,
    sel: &Selection,
    keep: impl Fn(&Value) -> bool,
) -> JidrResult<Vec<String>> {
    if sel.all {
        let list = client.call("device.list", json!({})).await?;
        return Ok(list
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|d| keep(d))
                    .filter_map(|d| d["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default());
    }
    if sel.names.is_empty() {
        return Err(JidrError::usage("name at least one device, or pass --all"));
    }
    Ok(sel.names.clone())
}

fn state_of(d: &Value) -> &str {
    d["status"]["state"].as_str().unwrap_or("")
}

fn dirs_home() -> JidrResult<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or_else(|| JidrError::unexpected("HOME is not set"))
}

fn parse_job(s: &str) -> JidrResult<Uuid> {
    Uuid::parse_str(s).map_err(|_| JidrError::usage(format!("{s:?} is not an id")))
}

/// `jidr proxy set`'s `[host:]port`; the host defaults to the Mac as the
/// emulator sees it.
fn proxy_target(s: &str) -> JidrResult<(String, u16)> {
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (h.trim(), p),
        None => (jidr_widgets::network::proxy::HOST_LOOPBACK, s),
    };
    match port.trim().parse::<u16>() {
        Ok(port) if port != 0 && !host.is_empty() => Ok((host.to_string(), port)),
        _ => Err(JidrError::usage(format!(
            "{s:?} isn't [host:]port, e.g. 8080 or 10.0.2.2:8080"
        ))),
    }
}

/// `jidr flash`'s human output: what happened, then the note if any.
fn flash_text(v: &Value, device: &str) -> String {
    let mut text = if v["action"] == "module_installed" {
        let m = &v["module"];
        let mut what = m["id"].as_str().unwrap_or("?").to_string();
        if let Some(version) = m["version"].as_str() {
            what = format!("{what} {version}");
        }
        format!("Installed the Magisk module {what} on {device}; it rebooted and Magisk loaded it.")
    } else {
        format!(
            "Copied to {}.",
            v["remote"].as_str().unwrap_or("/sdcard/Download/")
        )
    };
    if let Some(note) = v["note"].as_str() {
        text = format!("{text} {note}");
    }
    text
}

async fn run(cli: Cli, out: Out) -> JidrResult<()> {
    let paths = JidrPaths::discover()?;
    if before_connect(&paths, &cli.cmd, out).await? {
        return Ok(());
    }
    let client = connect(&paths, cli.restart_stale, out).await?;
    if let Cmd::Mcp { allow_destructive } = cli.cmd {
        // MCP owns stdout from here on.
        let opts = jidr_mcp::McpOptions { allow_destructive };
        return jidr_mcp::serve_stdio(client, opts).await;
    }
    dispatch(&client, cli.cmd, out).await
}

/// Commands that run without a daemon connection (`daemon stop`,
/// `uninstall`); returns whether `cmd` was one of them.
async fn before_connect(paths: &JidrPaths, cmd: &Cmd, out: Out) -> JidrResult<bool> {
    if let Cmd::Daemon {
        action: Some(DaemonCmd::Stop),
    } = cmd
    {
        let socket = paths.socket();
        if !socket.exists() {
            out.ok(&json!({ "stopped": false }), |_| {
                "The daemon isn't running.".into()
            });
            return Ok(true);
        }
        jidr_client::shutdown_daemon(&socket).await?;
        out.ok(&json!({ "stopped": true }), |_| "Daemon stopped.".into());
        return Ok(true);
    }
    if let Cmd::Uninstall { purge, yes } = cmd {
        let what = if *purge {
            "Remove Jidr's daemon, MCP entries, caches, logs AND all Jidr devices and its private SDK"
        } else {
            "Remove Jidr's daemon, MCP entries, caches and logs (devices are kept)"
        };
        confirm(out, *yes, what)?;
        let home = dirs_home()?;
        let report = uninstall::run(paths, &home, *purge).await?;
        let v = serde_json::to_value(&report).unwrap_or(Value::Null);
        out.ok(&v, |_| {
            let mut lines = Vec::new();
            if !report.stopped_devices.is_empty() {
                lines.push(format!("Stopped {}.", report.stopped_devices.join(", ")));
            }
            for m in &report.mcp {
                lines.push(format!("Removed MCP entries {:?} from {} (backup: {}).", m.removed, m.file.display(), m.backup.display()));
            }
            for p in &report.removed {
                lines.push(format!("Deleted {}.", p.display()));
            }
            for p in &report.kept {
                lines.push(format!("Kept {} (use --purge to delete devices and the private SDK).", p.display()));
            }
            lines.push("Android Studio's SDK and devices were not touched. Remove the app itself with `brew uninstall --cask jidr` or by moving Jidr.app to the Trash.".into());
            lines.join("\n")
        });
        return Ok(true);
    }
    Ok(false)
}

/// Runs a command that needs the daemon over an open connection (the CLI's
/// one-shot path and every line of `jidr shell`).
async fn dispatch(c: &Client, cmd: Cmd, out: Out) -> JidrResult<()> {
    match cmd {
        Cmd::Completions { .. }
        | Cmd::Uninstall { .. }
        | Cmd::Daemon {
            action: None | Some(DaemonCmd::Run | DaemonCmd::Stop),
        } => {}
        Cmd::Mcp { .. } | Cmd::Shell { .. } => {
            return Err(JidrError::usage(
                "`mcp` and `shell` can't run inside `jidr shell`",
            ));
        }
        Cmd::Daemon {
            action: Some(DaemonCmd::Status),
        } => {
            let v = c.call("daemon.status", json!({})).await?;
            out.ok(&v, |v| {
                format!(
                    "Jidr daemon {} (pid {}), up {} s, {} devices ({} running), socket {}",
                    v["version"].as_str().unwrap_or("?"),
                    v["pid"],
                    v["uptime_s"],
                    v["devices"],
                    v["running"],
                    v["socket"].as_str().unwrap_or("?")
                )
            });
        }
        Cmd::Device { cmd } => device(c, out, cmd).await?,
        Cmd::Install {
            device,
            apk,
            no_replace,
            grant,
            downgrade,
        } => {
            let apk = std::path::absolute(&apk).unwrap_or(apk);
            let v = exec(c, out, "file.install", json!({ "device": device, "apk": apk, "replace": !no_replace, "grant_permissions": grant, "downgrade": downgrade })).await?;
            out.ok(&v, |_| format!("Installed {} on {device}.", apk.display()));
        }
        Cmd::Push {
            device,
            local,
            remote,
        } => {
            let local = std::path::absolute(&local).unwrap_or(local);
            let mut p = json!({ "device": device, "local": local });
            if let Some(r) = remote {
                p["remote"] = json!(r);
            }
            let v = exec(c, out, "file.push", p).await?;
            out.ok(&v, |v| {
                format!("Pushed to {}.", v["remote"].as_str().unwrap_or("?"))
            });
        }
        Cmd::Flash { device, zip } => {
            let zip = std::path::absolute(&zip).unwrap_or(zip);
            let v = exec(
                c,
                out,
                "file.flash",
                json!({ "device": device, "path": zip }),
            )
            .await?;
            out.ok(&v, |v| flash_text(v, &device));
        }
        Cmd::Pull {
            device,
            remote,
            local,
        } => {
            let local = std::path::absolute(&local).unwrap_or(local);
            let v = exec(
                c,
                out,
                "file.pull",
                json!({ "device": device, "remote": remote, "local": local }),
            )
            .await?;
            out.ok(&v, |_| format!("Pulled to {}.", local.display()));
        }
        Cmd::Logcat {
            device,
            lines,
            filter,
            follow,
            clear,
        } => {
            if clear {
                let v = exec(c, out, "logcat.clear", json!({ "device": device })).await?;
                out.ok(&v, |_| String::new());
            } else if follow {
                let mut rx = c.events();
                c.call(
                    "logcat.follow",
                    json!({ "device": device, "filter": filter }),
                )
                .await?;
                loop {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => break,
                        ev = rx.recv() => match ev {
                            Ok(ev) if ev.topic == "logcat" => {
                                if out.json {
                                    println!("{}", json!({ "jidr_json": output::JSON_VERSION, "ok": true, "event": ev.data }));
                                } else {
                                    println!("{}", ev.data["line"].as_str().unwrap_or_default());
                                }
                            }
                            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(_) => break,
                        }
                    }
                }
            } else {
                let mut p = json!({ "device": device, "filter": filter });
                if let Some(n) = lines {
                    p["lines"] = json!(n);
                }
                let v = exec(c, out, "logcat.dump", p).await?;
                out.ok(&v, |v| v["text"].as_str().unwrap_or_default().to_string());
            }
        }
        Cmd::Image {
            cmd: ImageCmd::List { installed },
        } => {
            let v = exec(c, out, "image.list", json!({ "available": !installed })).await?;
            out.ok(&v, output::images);
        }
        Cmd::Image {
            cmd:
                ImageCmd::Install {
                    image,
                    accept_licenses,
                },
        } => {
            let v = with_licenses(c, out, accept_licenses, || {
                exec(c, out, "image.install", json!({ "image": image }))
            })
            .await?;
            out.ok(&v, |v| {
                v["message"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Installed {image}."))
            });
        }
        Cmd::Profile {
            cmd: ProfileCmd::List,
        } => {
            let v = exec(c, out, "profile.list", json!({})).await?;
            out.ok(&v, output::profiles);
        }
        Cmd::Sdk {
            cmd: SdkCmd::Status,
        } => {
            let v = exec(c, out, "sdk.status", json!({})).await?;
            out.ok(&v, output::pretty);
        }
        Cmd::Sdk {
            cmd: SdkCmd::Licenses { packages },
        } => {
            let v = exec(c, out, "sdk.licenses", json!({ "packages": packages })).await?;
            out.ok(&v, |v| {
                let list = v.as_array().cloned().unwrap_or_default();
                if list.is_empty() {
                    return "All licenses for these packages are accepted.".into();
                }
                list.iter()
                    .map(|l| {
                        format!(
                            "----- License {} (sha256 {}) for {} -----\n{}\n",
                            l["id"].as_str().unwrap_or_default(),
                            l["sha256"].as_str().unwrap_or_default(),
                            l["packages"],
                            l["text"].as_str().unwrap_or_default()
                        )
                    })
                    .collect()
            });
        }
        Cmd::Sdk {
            cmd:
                SdkCmd::Install {
                    packages,
                    accept_licenses,
                },
        } => {
            let v = with_licenses(c, out, accept_licenses, || {
                exec(c, out, "sdk.install", json!({ "packages": packages }))
            })
            .await?;
            out.ok(&v, |_| "Installed.".into());
        }
        Cmd::Settings { cmd } => {
            let v = match cmd {
                SettingsCmd::Get => c.call("settings.get", json!({})).await?,
                SettingsCmd::Set {
                    default_root,
                    quick_boot,
                    budget_percent,
                    auto_hibernate_minutes,
                } => {
                    let mut p = Map::new();
                    if let Some(r) = default_root {
                        p.insert("default_root_mode".into(), json!(r.as_str()));
                    }
                    if let Some(q) = quick_boot {
                        p.insert("quick_boot".into(), json!(q));
                    }
                    if let Some(b) = budget_percent {
                        p.insert("budget_percent".into(), json!(b));
                    }
                    if let Some(m) = auto_hibernate_minutes {
                        p.insert("auto_hibernate_minutes".into(), json!(m));
                    }
                    c.call("settings.set", Value::Object(p)).await?
                }
            };
            out.ok(&v, |v| {
                format!(
                    "Default root mode: {}\nQuick boot on stop: {}\nRAM budget: {}% of host RAM\nAuto-hibernate: {}",
                    v["default_root_mode"].as_str().unwrap_or("?"),
                    if v["quick_boot"].as_bool().unwrap_or(true) {
                        "on"
                    } else {
                        "off"
                    },
                    v["budget_percent"],
                    match v["auto_hibernate_minutes"].as_u64() {
                        Some(0) | None => "off".to_string(),
                        Some(m) => format!("after {m} idle minutes"),
                    }
                )
            });
        }
        Cmd::Cache {
            cmd: CacheCmd::List,
        } => {
            let v = c.call("cache.list", json!({})).await?;
            out.ok(&v, |v| {
                output::table(
                    &["SHA256", "SIZE", "PATH"],
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .map(|e| {
                            vec![
                                e["sha256"].as_str().unwrap_or_default().to_string(),
                                format!("{:.1} MB", e["size"].as_f64().unwrap_or(0.0) / 1e6),
                                e["path"].as_str().unwrap_or_default().to_string(),
                            ]
                        })
                        .collect(),
                )
            });
        }
        Cmd::Cache {
            cmd: CacheCmd::Import { path, sha256 },
        } => {
            let path = std::path::absolute(&path).unwrap_or(path);
            let v = c
                .call("cache.import", json!({ "path": path, "sha256": sha256 }))
                .await?;
            out.ok(&v, |v| {
                format!(
                    "Cached {} (sha256 {}).",
                    path.display(),
                    v["sha256"].as_str().unwrap_or("?")
                )
            });
        }
        Cmd::Cache {
            cmd: CacheCmd::Prune { older_than_days },
        } => {
            let v = c
                .call("cache.prune", json!({ "older_than_days": older_than_days }))
                .await?;
            out.ok(&v, |v| {
                format!(
                    "Removed {} files ({:.1} MB).",
                    v["removed"],
                    v["bytes"].as_f64().unwrap_or(0.0) / 1e6
                )
            });
        }
        Cmd::Root { cmd } => {
            let (method, params) = match cmd {
                RootCmd::Set {
                    device,
                    mode,
                    magisk,
                } => {
                    let mut p = json!({ "device": device, "mode": mode.as_str() });
                    if let Some(apk) = magisk {
                        let apk = std::path::absolute(&apk).unwrap_or(apk);
                        let entry = c.call("cache.import", json!({ "path": apk })).await?;
                        p["magisk_sha256"] = entry["sha256"].clone();
                    }
                    ("root.set", p)
                }
                RootCmd::WritableSystem { device, state } => (
                    "root.writable_system",
                    json!({ "device": device, "enabled": state.enabled() }),
                ),
                RootCmd::Verify { device } => ("root.verify", json!({ "device": device })),
                RootCmd::Status { device } => ("root.status", json!({ "device": device })),
                RootCmd::Helper { device } => ("root.helper.status", json!({ "device": device })),
                RootCmd::AutoGrant { device, state } => (
                    "root.auto_grant",
                    json!({ "device": device, "enabled": state.enabled() }),
                ),
            };
            let v = exec(c, out, method, params).await?;
            out.ok(&v, output::pretty);
        }
        Cmd::Snapshot { cmd } => {
            let (method, params) = match cmd {
                SnapshotCmd::List { device } => ("snapshot.list", json!({ "device": device })),
                SnapshotCmd::Save { device, name } => {
                    ("snapshot.save", json!({ "device": device, "name": name }))
                }
                SnapshotCmd::Load { device, name } => {
                    ("snapshot.load", json!({ "device": device, "name": name }))
                }
                SnapshotCmd::Delete { device, name } => {
                    ("snapshot.delete", json!({ "device": device, "name": name }))
                }
            };
            let v = exec(c, out, method, params).await?;
            out.ok(&v, output::pretty);
        }
        Cmd::Cert { cmd } => {
            let (method, params) = match cmd {
                CertCmd::Install { device, file } => {
                    let file = std::path::absolute(&file).unwrap_or(file);
                    (
                        "root.cacert.install",
                        json!({ "device": device, "path": file }),
                    )
                }
                CertCmd::List { device } => ("root.cacert.list", json!({ "device": device })),
                CertCmd::Verify { device } => ("root.cacert.verify", json!({ "device": device })),
                CertCmd::Remove { device, cert, all } => (
                    "root.cacert.remove",
                    json!({ "device": device, "cert": cert, "all": all }),
                ),
            };
            let v = exec(c, out, method, params).await?;
            out.ok(&v, output::certs);
        }
        Cmd::Proxy { cmd } => {
            let (method, params) = match cmd {
                ProxyCmd::Set { device, proxy } => {
                    let (host, port) = proxy_target(&proxy)?;
                    (
                        "network.proxy.set",
                        json!({ "device": device, "host": host, "port": port }),
                    )
                }
                ProxyCmd::Clear { device } => ("network.proxy.clear", json!({ "device": device })),
                ProxyCmd::Show { device } => ("network.proxy.get", json!({ "device": device })),
            };
            let v = c.call(method, params).await?;
            out.ok(&v, |v| match v["proxy"].as_str() {
                Some(p) => format!("HTTP proxy: {p}"),
                None => "No HTTP proxy.".to_string(),
            });
        }
        Cmd::Network {
            cmd: NetworkCmd::Loss { device, percent },
        } => {
            let v = match percent {
                Some(p) => {
                    c.call(
                        "network.shaping.set",
                        json!({ "device": device, "loss_pct": p }),
                    )
                    .await?
                }
                None => {
                    c.call("network.shaping.get", json!({ "device": device }))
                        .await?
                }
            };
            out.ok(&v, output::shaping);
        }
        Cmd::Network {
            cmd:
                NetworkCmd::Dns {
                    device,
                    mode,
                    delay_ms,
                },
        } => {
            let v = if mode.is_none() && delay_ms.is_none() {
                c.call("network.shaping.get", json!({ "device": device }))
                    .await?
            } else {
                let mut p = json!({ "device": device });
                if let Some(m) = mode {
                    p["dns"] = json!(m.as_str());
                }
                if let Some(ms) = delay_ms {
                    p["dns_delay_ms"] = json!(ms);
                }
                c.call("network.shaping.set", p).await?
            };
            out.ok(&v, output::shaping);
        }
        Cmd::Audio {
            cmd:
                AudioCmd::Inject {
                    device,
                    file,
                    repeat,
                    rate,
                    channels,
                    no_wait,
                },
        } => {
            let file = std::path::absolute(&file).unwrap_or(file);
            let mut p = json!({ "device": device, "path": file, "loop": repeat });
            if let Some(r) = rate {
                p["sample_rate"] = json!(r);
            }
            if let Some(ch) = channels {
                p["channels"] = json!(ch);
            }
            let v = c
                .call("audio.inject", p)
                .await
                .map_err(|e| missing_provider("audio.inject", e))?;
            if no_wait {
                out.ok(&v, |v| {
                    format!(
                        "Audio job {}; stop it with `jidr audio stop {device}`",
                        v["job"].as_str().unwrap_or("?")
                    )
                });
            } else {
                let v = follow(c, out, v, None).await?;
                out.ok(&v, output::audio);
            }
        }
        Cmd::Audio {
            cmd: AudioCmd::Stop { device },
        } => {
            let v = c.call("audio.stop", json!({ "device": device })).await?;
            out.ok(&v, |v| {
                if v["stopped"] == true {
                    format!("Stopping the audio on {device}.")
                } else {
                    format!("No audio is playing on {device}.")
                }
            });
        }
        Cmd::Jobs {
            cmd: JobsCmd::List { all },
        } => {
            let v = c.call("jobs.list", json!({ "all": all })).await?;
            out.ok(&v, output::jobs);
        }
        Cmd::Jobs {
            cmd: JobsCmd::Wait { job },
        } => {
            let id = parse_job(&job)?;
            let v = follow(c, out, json!({ "job": id, "pending": true }), None).await?;
            out.ok(&v, output::pretty);
        }
        Cmd::Jobs {
            cmd: JobsCmd::Cancel { job },
        } => {
            let j = c.cancel_job(parse_job(&job)?).await?;
            out.ok(&serde_json::to_value(&j).unwrap_or(Value::Null), |_| {
                format!("Cancelling {job}.")
            });
        }
        Cmd::Pool { cmd } => pool(c, out, cmd).await?,
        Cmd::Base { cmd } => base(c, out, cmd).await?,
        Cmd::Budget => {
            let v = c.call("host.budget", json!({})).await?;
            out.ok(&v, output::budget);
        }
        Cmd::Commands => {
            let v = c.call("commands.list", json!({})).await?;
            out.ok(&v, output::commands);
        }
        Cmd::Call { method, params } => {
            let params: Value = match params {
                Some(p) => serde_json::from_str(&p)
                    .map_err(|e| JidrError::usage(format!("params must be JSON: {e}")))?,
                None => json!({}),
            };
            let v = exec(c, out, &method, params).await?;
            out.ok(&v, output::pretty);
        }
    }
    Ok(())
}

async fn device(c: &Client, out: Out, cmd: DeviceCmd) -> JidrResult<()> {
    match cmd {
        DeviceCmd::Create {
            name,
            image,
            profile,
            hw,
            data_mb,
            root,
            agent_owned,
        } => {
            let mut p = Map::new();
            p.insert("name".into(), json!(name));
            p.insert("image".into(), json!(image));
            if let Some(pr) = profile {
                p.insert("profile".into(), json!(pr));
            }
            hw_params(&mut p, &hw);
            if let Some(d) = data_mb {
                p.insert("data_mb".into(), json!(d));
            }
            if let Some(r) = root {
                p.insert("root_mode".into(), json!(r.as_str()));
            }
            if agent_owned {
                p.insert("agent_owned".into(), json!(true));
            }
            let mut v = exec(c, out, "device.create", Value::Object(p)).await?;
            // Full root needs its setup (download, patch, boot, verify), not
            // just the stored mode.
            if v["root"]["mode"] == "magisk"
                && let Some(id) = v["id"].as_str().map(str::to_string)
            {
                out.info(&format!("Setting up Full root on {}…", output::device(&v)));
                exec(
                    c,
                    out,
                    "root.set",
                    json!({ "device": id, "mode": "magisk" }),
                )
                .await?;
                v = c.call("device.details", json!({ "device": id })).await?["device"].take();
            }
            out.ok(&v, |v| format!("Created {}", output::device(v)));
        }
        DeviceCmd::Start {
            sel,
            cold_boot,
            no_wait,
            timeout,
            force,
        } => {
            let names = select(c, &sel, |d| {
                matches!(state_of(d), "stopped" | "crashed" | "hibernated")
            })
            .await?;
            let mut results = Vec::new();
            for name in names {
                let v = c
                    .call(
                        "device.start",
                        json!({ "device": name, "cold_boot": cold_boot, "force": force }),
                    )
                    .await?;
                if no_wait {
                    results.push(v);
                    continue;
                }
                out.info(&format!("Starting {name}…"));
                results.push(follow(c, out, v, timeout.map(Duration::from_secs)).await?);
            }
            out.ok(&Value::Array(results.clone()), |_| {
                results
                    .iter()
                    .map(|r| {
                        if r["pending"] == true {
                            format!("Boot job {}", r["job"].as_str().unwrap_or("?"))
                        } else {
                            output::device(r)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        DeviceCmd::Stop { sel } => {
            let names = select(c, &sel, |d| {
                !matches!(state_of(d), "stopped" | "crashed" | "hibernated")
            })
            .await?;
            let mut results = Vec::new();
            for name in names {
                results.push(exec(c, out, "device.stop", json!({ "device": name })).await?);
            }
            out.ok(&Value::Array(results.clone()), |_| {
                results
                    .iter()
                    .map(output::device)
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        DeviceCmd::Hibernate { sel } => {
            let names = select(c, &sel, |d| state_of(d) == "running").await?;
            let mut results = Vec::new();
            for name in names {
                out.info(&format!("Hibernating {name}…"));
                results.push(exec(c, out, "device.hibernate", json!({ "device": name })).await?);
            }
            out.ok(&Value::Array(results.clone()), |_| {
                results
                    .iter()
                    .map(output::device)
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        DeviceCmd::List => {
            let v = c.call("device.list", json!({})).await?;
            out.ok(&v, output::devices);
        }
        DeviceCmd::Details { device } => {
            let v = c
                .call("device.details", json!({ "device": device }))
                .await?;
            out.ok(&v, |v| {
                format!(
                    "{}\nAVD: {}\nDisk: {:.1} GB\n{}",
                    output::device(&v["device"]),
                    v["avd_dir"].as_str().unwrap_or("?"),
                    v["disk_usage"].as_f64().unwrap_or(0.0) / 1e9,
                    output::pretty(&v["config"])
                )
            });
        }
        DeviceCmd::Clone {
            device,
            name,
            keep_snapshot,
        } => {
            let mut p = Map::new();
            p.insert("device".into(), json!(device));
            p.insert("name".into(), json!(name));
            if let Some(k) = keep_snapshot {
                p.insert("keep_snapshot".into(), json!(k));
            }
            let v = exec(c, out, "device.clone", Value::Object(p)).await?;
            out.ok(&v, |v| format!("Created {}", output::device(v)));
        }
        DeviceCmd::Reset { device, yes } => {
            confirm(out, yes, &format!("Wipe all data on {device}"))?;
            let v = exec(c, out, "device.reset", json!({ "device": device })).await?;
            out.ok(&v, |_| format!("{device} was reset to factory state."));
        }
        DeviceCmd::Delete { sel, yes } => {
            let names = select(c, &sel, |_| true).await?;
            confirm(out, yes, &format!("Delete {}", names.join(", ")))?;
            let mut results = Vec::new();
            for name in names {
                results.push(exec(c, out, "device.delete", json!({ "device": name })).await?);
            }
            out.ok(&Value::Array(results.clone()), |_| {
                results
                    .iter()
                    .map(|r| format!("Deleted {}.", r["name"].as_str().unwrap_or("?")))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        DeviceCmd::Edit {
            device,
            rename,
            hw,
            agent_owned,
            auto_hibernate,
            auto_hibernate_default,
        } => {
            let mut p = Map::new();
            p.insert("device".into(), json!(device));
            if let Some(n) = rename {
                p.insert("name".into(), json!(n));
            }
            hw_params(&mut p, &hw);
            if let Some(a) = agent_owned {
                p.insert("agent_owned".into(), json!(a));
            }
            if let Some(m) = auto_hibernate {
                p.insert("auto_hibernate_minutes".into(), json!(m));
            } else if auto_hibernate_default {
                p.insert("auto_hibernate_minutes".into(), Value::Null);
            }
            let v = exec(c, out, "device.edit", Value::Object(p)).await?;
            out.ok(&v, output::device);
        }
        DeviceCmd::Logs { device, zip } => {
            let zip = zip.map(|z| std::path::absolute(&z).unwrap_or(z));
            let v = exec(
                c,
                out,
                "device.logs",
                json!({ "device": device, "output": zip }),
            )
            .await?;
            out.ok(&v, |v| {
                format!(
                    "Wrote {} ({} files).",
                    v["path"].as_str().unwrap_or("?"),
                    v["files"]
                )
            });
        }
        DeviceCmd::ConnectInfo { device } => {
            let v = c
                .call("device.connect_info", json!({ "device": device }))
                .await?;
            out.ok(&v, output::pretty);
        }
    }
    Ok(())
}

fn ttl_s(ttl: &str) -> JidrResult<u64> {
    cli::parse_duration(ttl).map_err(JidrError::usage)
}

async fn pool(c: &Client, out: Out, cmd: PoolCmd) -> JidrResult<()> {
    match cmd {
        PoolCmd::Get { base, ttl, timeout } => {
            let mut p = Map::new();
            p.insert("base".into(), json!(base));
            p.insert("ttl_s".into(), json!(ttl_s(&ttl)?));
            if let Some(t) = timeout {
                p.insert("timeout_s".into(), json!(t));
            }
            out.info(&format!("Leasing a clone of {base}…"));
            let v = exec(c, out, "pool.get", Value::Object(p)).await?;
            out.ok(&v, |v| {
                format!(
                    "{}  serial {}\nLease {} (expires {}); `jidr pool release {}` when done.",
                    v["device"]["name"].as_str().unwrap_or("?"),
                    v["serial"].as_str().unwrap_or("?"),
                    v["lease"]["id"].as_str().unwrap_or("?"),
                    v["lease"]["expires_at"]
                        .as_str()
                        .unwrap_or("with this connection"),
                    v["device"]["name"].as_str().unwrap_or("?"),
                )
            });
        }
        PoolCmd::Release { lease_or_device } => {
            let by_lease = Uuid::parse_str(&lease_or_device)
                .ok()
                .map(|l| json!({ "lease": l }));
            let v = match by_lease {
                Some(p) => match c.call("pool.release", p).await {
                    Err(e) if e.code == jidr_core::ErrorCode::NotFound => {
                        c.call("pool.release", json!({ "device": lease_or_device }))
                            .await?
                    }
                    r => r?,
                },
                None => {
                    c.call("pool.release", json!({ "device": lease_or_device }))
                        .await?
                }
            };
            out.ok(&v, |v| {
                format!("Released {}.", v["name"].as_str().unwrap_or("?"))
            });
        }
        PoolCmd::Renew { lease, ttl } => {
            let v = c
                .call(
                    "pool.renew",
                    json!({ "lease": parse_job(&lease)?, "ttl_s": ttl_s(&ttl)? }),
                )
                .await?;
            out.ok(&v, |v| {
                format!(
                    "Lease {} expires {}.",
                    v["id"].as_str().unwrap_or("?"),
                    v["expires_at"].as_str().unwrap_or("with its connection")
                )
            });
        }
        PoolCmd::Ls => {
            let v = c.call("pool.list", json!({})).await?;
            out.ok(&v, |v| {
                let rows: Vec<Vec<String>> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        vec![
                            r["device"].as_str().unwrap_or("?").to_string(),
                            r["base"].as_str().unwrap_or("?").to_string(),
                            r["serial"].as_str().unwrap_or("-").to_string(),
                            r["lease"]["expires_at"]
                                .as_str()
                                .unwrap_or("with its connection")
                                .to_string(),
                            r["lease"]["id"].as_str().unwrap_or("?").to_string(),
                        ]
                    })
                    .collect();
                if rows.is_empty() {
                    "No leases.".into()
                } else {
                    output::table(&["DEVICE", "BASE", "SERIAL", "EXPIRES", "LEASE"], rows)
                }
            });
        }
    }
    Ok(())
}

async fn base(c: &Client, out: Out, cmd: BaseCmd) -> JidrResult<()> {
    match cmd {
        BaseCmd::Create { device, name } => {
            out.info(&format!("Saving {device} as base {name}…"));
            let v = exec(
                c,
                out,
                "base.create",
                json!({ "from_device": device, "name": name }),
            )
            .await?;
            out.ok(&v, |v| format!("Created base {}", output::device(v)));
        }
        BaseCmd::Ls => {
            let v = c.call("base.list", json!({})).await?;
            out.ok(&v, |v| {
                let rows: Vec<Vec<String>> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| {
                        vec![
                            b["name"].as_str().unwrap_or("?").to_string(),
                            format!(
                                "{}:{}",
                                b["image"]["api"],
                                b["image"]["tag"].as_str().unwrap_or("")
                            ),
                            b["profile"].as_str().unwrap_or("?").to_string(),
                            b["root"].as_str().unwrap_or("?").to_string(),
                            b["leases"].to_string(),
                        ]
                    })
                    .collect();
                if rows.is_empty() {
                    "No bases. Save one with `jidr base create <device> <name>`.".into()
                } else {
                    output::table(&["BASE", "IMAGE", "PROFILE", "ROOT", "LEASED"], rows)
                }
            });
        }
        BaseCmd::Rm { base, yes } => {
            confirm(out, yes, &format!("Delete base {base}"))?;
            let v = c.call("base.delete", json!({ "base": base })).await?;
            out.ok(&v, |v| {
                format!("Deleted base {}.", v["name"].as_str().unwrap_or("?"))
            });
        }
    }
    Ok(())
}

/// `jidr daemon` (run in the foreground; what auto-start spawns).
fn run_daemon(out: Out) -> u8 {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("JIDR_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return out.err(&JidrError::unexpected(format!("tokio runtime: {e}"))),
    };
    let result: JidrResult<()> = rt.block_on(async {
        let daemon = build_daemon(JidrPaths::discover()?)?;
        daemon.serve(shutdown_signal()).await
    });
    match result {
        Ok(()) => 0,
        Err(e) => out.err(&e),
    }
}

/// The daemon `jidr daemon` serves: the AOSP backend plus every provider.
/// Root comes first: boot hooks run in this order, and its hook restores
/// `adb root`, which the widgets' hook (packet loss, bad DNS) needs.
fn build_daemon(paths: JidrPaths) -> JidrResult<Arc<jidr_daemon::Daemon>> {
    jidr_daemon::Daemon::builder(
        jidr_daemon::DaemonConfig::new(paths),
        Arc::new(jidr_backend::AospBackend::new()),
    )
    .provider(jidr_root::provider())
    .provider(jidr_widgets::provider())
    .build()
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_targets_default_to_the_mac() {
        assert_eq!(proxy_target("8080").unwrap(), ("10.0.2.2".into(), 8080));
        assert_eq!(
            proxy_target("192.168.1.5:3128").unwrap(),
            ("192.168.1.5".into(), 3128)
        );
        for bad in ["", "x", ":8080", "h:0", "h:70000"] {
            assert!(proxy_target(bad).is_err(), "{bad}");
        }
    }

    // G6/G7: without the widgets and root providers, the CLI and MCP reject
    // every widget and root command.
    #[test]
    fn daemon_serves_widget_and_root_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let daemon = build_daemon(JidrPaths::under(tmp.path())).unwrap();
        for method in [
            "device.list",
            "gps.set",
            "battery.set",
            "snapshot.save",
            "root.set",
            "root.writable_system",
        ] {
            assert!(daemon.registry().get(method).is_some(), "{method}");
        }
    }

    /// docs/control-protocol.md's command reference must match what
    /// `jidr daemon` serves: core, jobs, widgets and root (gap G16; the old
    /// check in jidr-daemon saw only the core). `JIDR_UPDATE_DOCS=1 cargo
    /// nextest run -p jidr protocol_doc` rewrites it.
    #[test]
    fn protocol_doc_matches_commands_list() {
        use jidr_daemon::protocol_doc::{render, splice};
        let tmp = tempfile::tempdir().unwrap();
        let daemon = build_daemon(JidrPaths::under(tmp.path())).unwrap();
        let generated = render(&daemon.registry().list());
        assert!(generated.contains("root.set") && generated.contains("gps.set"));
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/control-protocol.md");
        let doc = std::fs::read_to_string(&path).unwrap();
        let updated =
            splice(&doc, &generated).expect("docs/control-protocol.md lost its commands markers");
        if std::env::var_os("JIDR_UPDATE_DOCS").is_some() {
            std::fs::write(&path, &updated).unwrap();
            return;
        }
        assert!(
            updated == doc,
            "docs/control-protocol.md is out of date with commands.list; run\n  JIDR_UPDATE_DOCS=1 cargo nextest run -p jidr protocol_doc"
        );
    }
}
