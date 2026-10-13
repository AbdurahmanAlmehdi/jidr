//! `jidr`: the Jidr CLI, daemon and MCP server.

mod cli;
mod output;
mod uninstall;

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{CommandFactory, Parser};
use cli::{
    CacheCmd, Cli, Cmd, DaemonCmd, DeviceCmd, Hardware, ImageCmd, JobsCmd, ProfileCmd, RootCmd,
    SdkCmd, Selection, SnapshotCmd,
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

async fn connect(paths: &JidrPaths, cli: &Cli, out: Out) -> JidrResult<Client> {
    let mut opts = SpawnOptions {
        program: std::env::current_exe().ok(),
        restart_stale: cli.restart_stale,
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

/// Root and snapshot commands come from other providers; say so clearly
/// while they aren't registered.
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
    Uuid::parse_str(s).map_err(|_| JidrError::usage(format!("{s:?} is not a job id")))
}

async fn run(cli: Cli, out: Out) -> JidrResult<()> {
    let paths = JidrPaths::discover()?;
    if let Cmd::Daemon {
        action: Some(DaemonCmd::Stop),
    } = &cli.cmd
    {
        let socket = paths.socket();
        if !socket.exists() {
            out.ok(&json!({ "stopped": false }), |_| {
                "The daemon isn't running.".into()
            });
            return Ok(());
        }
        jidr_client::shutdown_daemon(&socket).await?;
        out.ok(&json!({ "stopped": true }), |_| "Daemon stopped.".into());
        return Ok(());
    }
    if let Cmd::Uninstall { purge, yes } = &cli.cmd {
        let what = if *purge {
            "Remove Jidr's daemon, MCP entries, caches, logs AND all Jidr devices and its private SDK"
        } else {
            "Remove Jidr's daemon, MCP entries, caches and logs (devices are kept)"
        };
        confirm(out, *yes, what)?;
        let home = dirs_home()?;
        let report = uninstall::run(&paths, &home, *purge).await?;
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
        return Ok(());
    }
    if let Cmd::Flash { .. } = &cli.cmd {
        return Err(JidrError::usage(
            "flash isn't available yet: Genymotion's flash semantics are an open research item (docs/gaps.md)",
        ));
    }
    let client = connect(&paths, &cli, out).await?;
    let c = &client;
    match cli.cmd {
        Cmd::Completions { .. }
        | Cmd::Flash { .. }
        | Cmd::Uninstall { .. }
        | Cmd::Daemon {
            action: None | Some(DaemonCmd::Run | DaemonCmd::Stop),
        } => {}
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
                RootCmd::Verify { device } => ("root.verify", json!({ "device": device })),
                RootCmd::Status { device } => ("root.status", json!({ "device": device })),
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
            let v = exec(c, out, "device.create", Value::Object(p)).await?;
            out.ok(&v, |v| format!("Created {}", output::device(v)));
        }
        DeviceCmd::Start {
            sel,
            cold_boot,
            no_wait,
            timeout,
        } => {
            let names = select(c, &sel, |d| matches!(state_of(d), "stopped" | "crashed")).await?;
            let mut results = Vec::new();
            for name in names {
                let v = c
                    .call(
                        "device.start",
                        json!({ "device": name, "cold_boot": cold_boot }),
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
            let names = select(c, &sel, |d| !matches!(state_of(d), "stopped" | "crashed")).await?;
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
        DeviceCmd::Clone { device, name } => {
            let v = exec(
                c,
                out,
                "device.clone",
                json!({ "device": device, "name": name }),
            )
            .await?;
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
        let paths = JidrPaths::discover()?;
        let daemon = jidr_daemon::Daemon::builder(
            jidr_daemon::DaemonConfig::new(paths),
            Arc::new(jidr_backend::AospBackend::new()),
        )
        // Providers from jidr-widgets (B) and jidr-root (C) are registered
        // here once they export `provider()` (docs/gaps.md A1/A2).
        .build()?;
        daemon.serve(shutdown_signal()).await
    });
    match result {
        Ok(()) => 0,
        Err(e) => out.err(&e),
    }
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
