//! `jidr phone …`: physical phones over adb (#92), thin calls over the
//! `physical.*` commands.

use jidr_client::Client;
use jidr_core::{JidrError, JidrResult, PhoneConnectInfo};
use serde_json::{Value, json};

use crate::cli::PhoneCmd;
use crate::output::{self, Out};
use crate::{exec, follow};

pub async fn run(c: &Client, out: Out, cmd: PhoneCmd) -> JidrResult<()> {
    match cmd {
        PhoneCmd::List => {
            let v = exec(c, out, "physical.list", json!({})).await?;
            out.ok(&v, output::phones);
        }
        PhoneCmd::Details { phone } => {
            let v = exec(c, out, "physical.details", json!({ "device": phone })).await?;
            out.ok(&v, output::phone);
        }
        PhoneCmd::Screenshot { phone, output } => {
            let mut p = json!({ "device": phone });
            if let Some(o) = output {
                p["path"] = json!(std::path::absolute(&o).unwrap_or(o));
            }
            let v = exec(c, out, "physical.screenshot", p).await?;
            out.ok(&v, |v| {
                format!("Saved {}.", v["path"].as_str().unwrap_or("?"))
            });
        }
        PhoneCmd::Record {
            phone,
            output,
            seconds,
        } => {
            let mut p = json!({ "device": phone });
            if let Some(o) = output {
                p["path"] = json!(std::path::absolute(&o).unwrap_or(o));
            }
            if let Some(s) = seconds {
                p["max_seconds"] = json!(s);
            }
            let v = c.call("physical.record", p).await?;
            if !out.json && !out.quiet {
                eprintln!("Recording {phone}; press Ctrl-C to stop and save.");
            }
            let v = tokio::select! {
                r = follow(c, out, v.clone(), None) => r?,
                _ = tokio::signal::ctrl_c() => {
                    c.call("physical.record_stop", json!({ "device": phone })).await?;
                    follow(c, out, v, None).await?
                }
            };
            out.ok(&v, |v| {
                format!(
                    "Saved {} ({} s).",
                    v["path"].as_str().unwrap_or("?"),
                    v["seconds"]
                )
            });
        }
        PhoneCmd::Install {
            phone,
            apk,
            no_replace,
            grant,
            downgrade,
        } => {
            let apk = std::path::absolute(&apk).unwrap_or(apk);
            let v = exec(
                c,
                out,
                "physical.install",
                json!({ "device": phone, "apk": apk, "replace": !no_replace, "grant_permissions": grant, "downgrade": downgrade }),
            )
            .await?;
            out.ok(&v, |_| format!("Installed {} on {phone}.", apk.display()));
        }
        PhoneCmd::Push {
            phone,
            local,
            remote,
        } => {
            let local = std::path::absolute(&local).unwrap_or(local);
            let mut p = json!({ "device": phone, "local": local });
            if let Some(r) = remote {
                p["remote"] = json!(r);
            }
            let v = exec(c, out, "physical.push", p).await?;
            out.ok(&v, |v| {
                format!("Pushed to {}.", v["remote"].as_str().unwrap_or("?"))
            });
        }
        PhoneCmd::Pull {
            phone,
            remote,
            local,
        } => {
            let local = std::path::absolute(&local).unwrap_or(local);
            let v = exec(
                c,
                out,
                "physical.pull",
                json!({ "device": phone, "remote": remote, "local": local }),
            )
            .await?;
            out.ok(&v, |_| format!("Pulled to {}.", local.display()));
        }
        PhoneCmd::Logcat {
            phone,
            lines,
            filter,
            follow: true,
        } => {
            let _ = lines;
            let mut rx = c.events();
            c.call(
                "logcat.follow",
                json!({ "device": phone, "filter": filter }),
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
        }
        PhoneCmd::Logcat {
            phone,
            lines,
            filter,
            follow: false,
        } => {
            let mut p = json!({ "device": phone, "filter": filter });
            if let Some(n) = lines {
                p["lines"] = json!(n);
            }
            let v = exec(c, out, "physical.logcat", p).await?;
            out.ok(&v, |v| v["text"].as_str().unwrap_or_default().to_string());
        }
        PhoneCmd::Shell { phone, command } if command.is_empty() => {
            interactive_shell(c, &phone).await?;
        }
        PhoneCmd::Shell { phone, command } => {
            let v = exec(
                c,
                out,
                "physical.shell",
                json!({ "device": phone, "command": command.join(" ") }),
            )
            .await?;
            out.ok(&v, |v| {
                let mut text = v["stdout"].as_str().unwrap_or_default().to_string();
                if let Some(err) = v["stderr"].as_str().filter(|e| !e.is_empty()) {
                    eprint!("{err}");
                }
                if text.ends_with('\n') {
                    text.pop();
                }
                text
            });
            if let Some(code) = v["exit_code"].as_i64().filter(|c| *c != 0) {
                return Err(
                    JidrError::unexpected(format!("the command exited with {code}"))
                        .with_data(json!({ "exit_code": code })),
                );
            }
        }
        PhoneCmd::Reboot { phone } => {
            let v = exec(c, out, "physical.reboot", json!({ "device": phone })).await?;
            out.ok(&v, |_| format!("Rebooting {phone}."));
        }
        PhoneCmd::Tap { phone, x, y } => {
            let v = exec(
                c,
                out,
                "physical.tap",
                json!({ "device": phone, "x": x, "y": y }),
            )
            .await?;
            out.ok(&v, |_| String::new());
        }
        PhoneCmd::Swipe {
            phone,
            x1,
            y1,
            x2,
            y2,
            ms,
        } => {
            let mut p = json!({ "device": phone, "x1": x1, "y1": y1, "x2": x2, "y2": y2 });
            if let Some(ms) = ms {
                p["duration_ms"] = json!(ms);
            }
            let v = exec(c, out, "physical.swipe", p).await?;
            out.ok(&v, |_| String::new());
        }
        PhoneCmd::Key { phone, key } => {
            let v = exec(
                c,
                out,
                "physical.key",
                json!({ "device": phone, "key": key }),
            )
            .await?;
            out.ok(&v, |_| String::new());
        }
        PhoneCmd::Text { phone, text } => {
            let v = exec(
                c,
                out,
                "physical.text",
                json!({ "device": phone, "text": text }),
            )
            .await?;
            out.ok(&v, |_| String::new());
        }
    }
    Ok(())
}

/// `adb -s <serial> shell` on this terminal: an interactive shell is a
/// terminal session, not a protocol call, so the CLI runs the adb binary the
/// daemon names (`physical.connect_info`).
async fn interactive_shell(c: &Client, phone: &str) -> JidrResult<()> {
    let info: PhoneConnectInfo = c
        .call_as("physical.connect_info", json!({ "device": phone }))
        .await?;
    let status = tokio::process::Command::new(&info.adb_path)
        .args(["-s", &info.serial, "shell"])
        .status()
        .await
        .map_err(|e| JidrError::sdk(format!("cannot run adb: {e}")))?;
    match status.code() {
        Some(0) | None => Ok(()),
        Some(code) => Err(
            JidrError::unexpected(format!("adb shell exited with {code}"))
                .with_data(Value::from(code)),
        ),
    }
}
