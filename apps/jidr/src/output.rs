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
    fn device_rows() {
        let d = json!([{ "name": "P", "status": {"state": "crashed", "reason": "boom"}, "image": {"api": 35, "tag": "google_apis"}, "root": {"mode": "adb"}, "runtime": null }]);
        let t = devices(&d);
        assert!(t.contains("crashed (boom)"));
        assert!(t.contains("35:google_apis"));
        assert!(devices(&json!([])).contains("No devices"));
    }
}
