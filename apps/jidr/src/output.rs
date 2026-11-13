//! Printing results: human text, or a versioned JSON envelope (D10).

use std::io::{IsTerminal, Write};

use jidr_core::{JidrError, Job};
use serde_json::{Value, json};

/// Version of the `--json` envelope. Bump on breaking changes (docs/cli.md).
pub const JSON_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct Out {
    pub json: bool,
    pub quiet: bool,
}

impl Out {
    /// Prints a successful result.
    pub fn ok(&self, result: &Value, human: impl FnOnce(&Value) -> String) {
        if self.json {
            println!(
                "{}",
                json!({ "jidr_json": JSON_VERSION, "ok": true, "result": result })
            );
        } else {
            let text = human(result);
            if !text.is_empty() {
                println!("{}", text.trim_end());
            }
        }
    }

    /// Prints an error (to stdout as JSON, else stderr) and returns its exit code.
    pub fn err(&self, e: &JidrError) -> u8 {
        if self.json {
            println!(
                "{}",
                json!({ "jidr_json": JSON_VERSION, "ok": false, "error": { "code": e.code, "exit_code": e.code.exit_code(), "message": e.message, "data": e.data } })
            );
        } else {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "error: {}", e.message);
            if let Some(hint) = hint(e) {
                let _ = writeln!(stderr, "hint: {hint}");
            }
        }
        e.code.exit_code()
    }

    /// Progress of a job, on stderr.
    pub fn progress(&self, job: &Job) {
        if self.json || self.quiet {
            return;
        }
        let pct = job
            .progress
            .map(|p| format!("{:>3.0}%", p * 100.0))
            .unwrap_or_else(|| "  …".into());
        let mut stderr = std::io::stderr().lock();
        if std::io::stderr().is_terminal() {
            let _ = write!(stderr, "\r\x1b[2K[{pct}] {} {}", job.kind, job.message);
        } else {
            let _ = writeln!(stderr, "[{pct}] {} {}", job.kind, job.message);
        }
        let _ = stderr.flush();
    }

    /// Ends a progress line.
    pub fn progress_done(&self) {
        if !self.json && !self.quiet && std::io::stderr().is_terminal() {
            eprintln!();
        }
    }

    /// Informational line on stderr.
    pub fn info(&self, text: &str) {
        if !self.json && !self.quiet {
            eprintln!("{text}");
        }
    }
}

fn hint(e: &JidrError) -> Option<&'static str> {
    use jidr_core::ErrorCode::*;
    match e.code {
        DaemonStale => Some("run again with --restart-stale to replace the running daemon"),
        Busy => Some("wait for the running operation, or cancel it with `jidr jobs cancel <job>`"),
        BootTimeout => Some(
            "try `jidr device start <name> --cold-boot`, or see the emulator log with `jidr device details <name>`",
        ),
        ResourceExhausted => Some(
            "see `jidr budget`; stop a device, use `--profile agent-lean`, or start with `--force`",
        ),
        _ => None,
    }
}

fn s(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Left-aligned columns.
pub fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(c.chars().count());
            }
        }
    }
    let line = |cells: Vec<String>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths.get(i).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = vec![line(headers.iter().map(|h| h.to_string()).collect())];
    out.extend(rows.into_iter().map(line));
    out.join("\n")
}

pub fn device_status(d: &Value) -> String {
    let state = d["status"]["state"].as_str().unwrap_or("?").to_string();
    match d["status"]["reason"].as_str() {
        Some(r) => format!("{state} ({r})"),
        None => state,
    }
}

pub fn devices(v: &Value) -> String {
    let rows: Vec<Vec<String>> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| {
                    vec![
                        s(d, "name"),
                        device_status(d),
                        format!("{}:{}", d["image"]["api"], s(&d["image"], "tag")),
                        s(&d["root"], "mode"),
                        s(&d["runtime"], "adb_serial"),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return "No devices. Create one with `jidr device create <name> --image 35`.".into();
    }
    table(&["NAME", "STATUS", "IMAGE", "ROOT", "SERIAL"], rows)
}

/// `host.budget`.
pub fn budget(v: &Value) -> String {
    let gb = |key: &Value| key.as_f64().unwrap_or(0.0) / 1024.0;
    let mut text = format!(
        "RAM budget: {:.1} of {:.1} GB used ({}% of {:.1} GB host RAM, {:.1} GB available now)",
        gb(&v["used_mb"]),
        gb(&v["budget_mb"]),
        v["budget_percent"],
        gb(&v["total_mb"]),
        gb(&v["available_mb"]),
    );
    let rows: Vec<Vec<String>> = v["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            vec![
                s(d, "name"),
                format!("{:.1} GB", gb(&d["expected_mb"])),
                if d["measured_mb"].is_null() {
                    "-".into()
                } else {
                    format!("{:.1} GB", gb(&d["measured_mb"]))
                },
            ]
        })
        .collect();
    if !rows.is_empty() {
        text.push('\n');
        text.push_str(&table(&["DEVICE", "EXPECTED", "MEASURED"], rows));
    }
    text
}

pub fn device(d: &Value) -> String {
    format!(
        "{}  {}  {}:{}  root {}{}",
        s(d, "name"),
        device_status(d),
        d["image"]["api"],
        s(&d["image"], "tag"),
        s(&d["root"], "mode"),
        d["runtime"]["adb_serial"]
            .as_str()
            .map(|x| format!("  {x}"))
            .unwrap_or_default()
    )
}

pub fn images(v: &Value) -> String {
    let rows: Vec<Vec<String>> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| {
                    let size = i["size"].as_u64().unwrap_or(0);
                    vec![
                        format!("{}:{}", i["api"], s(i, "tag")),
                        s(i, "abi"),
                        s(i, "revision"),
                        if size > 0 {
                            format!("{:.0} MB", size as f64 / 1e6)
                        } else {
                            String::new()
                        },
                        s(i, "support"),
                        if i["installed_in"].is_string() {
                            "installed".into()
                        } else {
                            String::new()
                        },
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&["IMAGE", "ABI", "REVISION", "SIZE", "SUPPORT", ""], rows)
}

pub fn profiles(v: &Value) -> String {
    let rows = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| {
                    vec![
                        s(p, "id"),
                        s(p, "name"),
                        format!("{}×{} @{}dpi", p["width"], p["height"], p["density"]),
                        format!("{} MB", p["ram_mb"]),
                        s(p, "cpus"),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&["ID", "NAME", "SCREEN", "RAM", "CPUS"], rows)
}

pub fn jobs(v: &Value) -> String {
    let rows = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|j| {
                    vec![
                        s(j, "id"),
                        s(j, "kind"),
                        s(j, "state"),
                        j["progress"]
                            .as_f64()
                            .map(|p| format!("{:.0}%", p * 100.0))
                            .unwrap_or_default(),
                        s(j, "message"),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&["JOB", "KIND", "STATE", "PROGRESS", "MESSAGE"], rows)
}

pub fn commands(v: &Value) -> String {
    let rows = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    let mut flags = Vec::new();
                    if c["longRunning"] == true {
                        flags.push("job");
                    }
                    if c["destructive"] == true {
                        flags.push("destructive");
                    }
                    vec![s(c, "id"), flags.join(","), s(c, "label")]
                })
                .collect()
        })
        .unwrap_or_default();
    table(&["COMMAND", "FLAGS", "DESCRIPTION"], rows)
}

/// Pretty JSON for results without a dedicated view.
/// Any `root.cacert.*` result: install, list/verify or remove.
pub fn certs(v: &Value) -> String {
    let s = |v: &Value| v.as_str().unwrap_or("?").to_string();
    if let Some(c) = v.get("cert") {
        return if v["changed"] == false {
            format!(
                "{} ({}) is already a trusted system certificate.",
                s(&c["subject"]),
                s(&c["file"])
            )
        } else {
            format!(
                "{} ({}) is now a trusted system certificate.",
                s(&c["subject"]),
                s(&c["file"])
            )
        };
    }
    if let Some(removed) = v.get("removed").and_then(Value::as_array) {
        let files: Vec<String> = removed.iter().map(|c| s(&c["file"])).collect();
        return if files.is_empty() {
            "No Jidr CA certificate to remove.".into()
        } else {
            format!("Removed {}.", files.join(", "))
        };
    }
    let certs = v["certs"].as_array().cloned().unwrap_or_default();
    if certs.is_empty() {
        return format!(
            "No CA certificates added (system store: {}).",
            s(&v["store"])
        );
    }
    let rows = certs
        .iter()
        .map(|c| {
            vec![
                s(&c["file"]),
                s(&c["subject"]),
                if c["trusted"] == true {
                    "yes".into()
                } else {
                    "after restart".into()
                },
            ]
        })
        .collect();
    format!(
        "{}\nSystem store: {}",
        table(&["FILE", "SUBJECT", "TRUSTED"], rows),
        s(&v["store"])
    )
}

/// An `audio.inject` result.
pub fn audio(v: &Value) -> String {
    let secs = |k: &str| v[k].as_f64().unwrap_or(0.0);
    let how = if v["stopped"] == true {
        "Stopped after"
    } else {
        "Played"
    };
    let loops = v["loops"].as_u64().unwrap_or(0);
    let passes = if loops > 1 {
        format!(", {loops} passes")
    } else {
        String::new()
    };
    let converted = match v["converted_from"].as_str() {
        Some(f) => format!(", converted from {f}"),
        None => String::new(),
    };
    format!(
        "{how} {:.1} s of {:.1} s into the microphone ({} Hz, {} ch, {}{converted}{passes}).",
        secs("played_s"),
        secs("duration_s"),
        v["sample_rate"].as_u64().unwrap_or(0),
        v["channels"].as_u64().unwrap_or(0),
        v["format"].as_str().unwrap_or("?"),
    )
}

/// A `network.shaping.*` result: one line when every interface agrees.
pub fn packet_loss(v: &Value) -> String {
    let ifaces = v["interfaces"].as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = ifaces.iter().filter_map(|i| i["name"].as_str()).collect();
    match v["loss_pct"].as_f64() {
        Some(0.0) => format!("No packet loss ({}).", names.join(", ")),
        Some(p) => format!("{p}% of outgoing packets dropped ({}).", names.join(", ")),
        None => table(
            &["INTERFACE", "LOSS", "QDISC"],
            ifaces
                .iter()
                .map(|i| {
                    vec![
                        i["name"].as_str().unwrap_or("?").to_string(),
                        format!("{}%", i["loss_pct"].as_f64().unwrap_or(0.0)),
                        i["qdisc"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect(),
        ),
    }
}

pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_align() {
        let t = table(
            &["A", "LONGER"],
            vec![vec!["xyz".into(), "1".into()], vec!["q".into(), "".into()]],
        );
        assert_eq!(t, "A    LONGER\nxyz  1\nq");
    }

    #[test]
    fn cert_results() {
        let c = json!({ "file": "a4ac454c.0", "subject": "Jidr Test CA", "trusted": true });
        assert!(certs(&json!({ "cert": c, "changed": true })).contains("is now a trusted"));
        assert!(certs(&json!({ "cert": c, "changed": false })).contains("already"));
        assert_eq!(certs(&json!({ "removed": [c] })), "Removed a4ac454c.0.");
        let t = certs(&json!({ "store": "/apex/x", "certs": [c] }));
        assert!(t.contains("Jidr Test CA") && t.contains("yes") && t.ends_with("/apex/x"));
        assert!(certs(&json!({ "store": "/apex/x", "certs": [] })).starts_with("No CA"));
    }

    #[test]
    fn packet_loss_results() {
        let i = |name: &str, loss: f64| json!({ "name": name, "loss_pct": loss, "qdisc": "netem" });
        let v = json!({ "loss_pct": 10.0, "interfaces": [i("radio0", 10.0), i("wlan0", 10.0)] });
        assert_eq!(
            packet_loss(&v),
            "10% of outgoing packets dropped (radio0, wlan0)."
        );
        let v = json!({ "loss_pct": 0.0, "interfaces": [i("wlan0", 0.0)] });
        assert_eq!(packet_loss(&v), "No packet loss (wlan0).");
        let v = json!({ "loss_pct": null, "interfaces": [i("radio0", 0.0), i("wlan0", 2.5)] });
        assert!(
            packet_loss(&v).contains("wlan0      2.5%"),
            "{}",
            packet_loss(&v)
        );
    }

    #[test]
    fn audio_results() {
        let v = json!({ "played_s": 2.5, "duration_s": 2.5, "loops": 1, "stopped": false,
            "sample_rate": 44100, "channels": 2, "format": "s16le", "converted_from": null });
        assert_eq!(
            audio(&v),
            "Played 2.5 s of 2.5 s into the microphone (44100 Hz, 2 ch, s16le)."
        );
        let v = json!({ "played_s": 7.0, "duration_s": 3.0, "loops": 2, "stopped": true,
            "sample_rate": 16000, "channels": 1, "format": "s16le", "converted_from": "float32" });
        assert_eq!(
            audio(&v),
            "Stopped after 7.0 s of 3.0 s into the microphone (16000 Hz, 1 ch, s16le, converted from float32, 2 passes)."
        );
    }

    #[test]
    fn device_rows() {
        let d = json!([{ "name": "P", "status": {"state": "crashed", "reason": "boom"}, "image": {"api": 35, "tag": "google_apis"}, "root": {"mode": "adb"}, "runtime": null }]);
        let t = devices(&d);
        assert!(t.contains("crashed (boom)"));
        assert!(t.contains("35:google_apis"));
        assert!(devices(&json!([])).contains("No devices"));
    }
}
