//! Async wrapper around the `adb` binary: devices, shell, install, push/pull,
//! logcat, root/unroot, connect. Errors map to Jidr error codes.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use jidr_core::{ErrorCode, JidrError, JidrResult};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Default timeout for one adb invocation.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct Adb {
    bin: PathBuf,
    serial: Option<String>,
    timeout: Duration,
}

/// Captured result of one adb run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeviceEntry {
    pub serial: String,
    /// `device`, `offline`, `unauthorized`, `recovery`, …
    pub state: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstallOptions {
    /// `-r`: replace an installed app.
    pub replace: bool,
    /// `-g`: grant all runtime permissions.
    pub grant_permissions: bool,
    /// `-d`: allow version downgrade.
    pub downgrade: bool,
}

/// Why an install failed, for the UI's signature-mismatch prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InstallFailure {
    /// e.g. `INSTALL_FAILED_UPDATE_INCOMPATIBLE`.
    pub reason: String,
    /// True when uninstalling the existing app would let the install succeed.
    pub signature_mismatch: bool,
}

impl Adb {
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            serial: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// The same adb, targeting one device (`-s <serial>`).
    pub fn device(&self, serial: impl Into<String>) -> Self {
        Self {
            serial: Some(serial.into()),
            ..self.clone()
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn serial(&self) -> Option<&str> {
        self.serial.as_deref()
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.bin);
        if let Some(s) = &self.serial {
            cmd.arg("-s").arg(s);
        }
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    /// Runs adb and returns its output, whatever the exit status.
    pub async fn raw(&self, args: &[&str]) -> JidrResult<Output> {
        let child = self.command(args).spawn().map_err(|e| {
            JidrError::sdk(format!("cannot run adb at {}: {e}", self.bin.display()))
        })?;
        let out = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                JidrError::unexpected(format!(
                    "adb {} timed out after {} s",
                    args.join(" "),
                    self.timeout.as_secs()
                ))
            })?
            .map_err(|e| JidrError::unexpected(format!("adb {}: {e}", args.join(" "))))?;
        Ok(Output {
            status: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Runs adb and fails unless it exits 0.
    pub async fn run(&self, args: &[&str]) -> JidrResult<Output> {
        let out = self.raw(args).await?;
        if out.success() {
            Ok(out)
        } else {
            Err(self.classify(args, &out))
        }
    }

    /// Maps adb's error text to a Jidr error code.
    fn classify(&self, args: &[&str], out: &Output) -> JidrError {
        let text = format!("{}{}", out.stderr, out.stdout);
        let msg = text.trim();
        let code = if msg.contains("not found") && msg.contains("device")
            || msg.contains("no devices/emulators found")
        {
            ErrorCode::NotFound
        } else if msg.contains("device offline") || msg.contains("device still authorizing") {
            ErrorCode::Busy
        } else {
            ErrorCode::Sdk
        };
        JidrError::new(code, format!("adb {} failed: {msg}", args.join(" ")))
            .with_data(serde_json::json!({"status": out.status, "serial": self.serial}))
    }

    pub async fn version(&self) -> JidrResult<String> {
        let out = self.run(&["version"]).await?;
        Ok(out.stdout.lines().next().unwrap_or_default().to_string())
    }

    pub async fn devices(&self) -> JidrResult<Vec<DeviceEntry>> {
        let out = self.run(&["devices"]).await?;
        Ok(parse_devices(&out.stdout))
    }

    /// Runs a shell command and returns `(exit code, stdout)`. adb's shell
    /// protocol v2 propagates the remote exit code.
    pub async fn shell_status(&self, cmd: &str) -> JidrResult<(i32, String)> {
        let out = self.raw(&["shell", cmd]).await?;
        if out.status.is_none()
            || out.stderr.contains("error: device")
            || out.stderr.contains("no devices")
        {
            return Err(self.classify(&["shell", cmd], &out));
        }
        Ok((out.status.unwrap_or(-1), out.stdout))
    }

    /// Runs a shell command; non-zero exit is an error.
    pub async fn shell(&self, cmd: &str) -> JidrResult<String> {
        let out = self.run(&["shell", cmd]).await?;
        Ok(out.stdout)
    }

    pub async fn getprop(&self, name: &str) -> JidrResult<String> {
        Ok(self
            .shell(&format!("getprop {name}"))
            .await?
            .trim()
            .to_string())
    }

    /// Installs an APK. A rejected install returns `sdk` with an
    /// [`InstallFailure`] in `data`.
    pub async fn install(&self, apk: &Path, opts: InstallOptions) -> JidrResult<()> {
        let apk_s = apk.to_string_lossy();
        let mut args = vec!["install"];
        if opts.replace {
            args.push("-r");
        }
        if opts.grant_permissions {
            args.push("-g");
        }
        if opts.downgrade {
            args.push("-d");
        }
        args.push(&apk_s);
        let out = self.raw(&args).await?;
        let text = format!("{}\n{}", out.stdout, out.stderr);
        if out.success() && text.contains("Success") {
            return Ok(());
        }
        match parse_install_failure(&text) {
            Some(f) => Err(JidrError::sdk(format!("install failed: {}", f.reason))
                .with_data(serde_json::to_value(&f).unwrap_or_default())),
            None => Err(self.classify(&args, &out)),
        }
    }

    pub async fn uninstall(&self, package: &str) -> JidrResult<()> {
        let out = self.run(&["uninstall", package]).await?;
        if out.stdout.contains("Success") {
            Ok(())
        } else {
            Err(JidrError::sdk(format!(
                "uninstall {package}: {}",
                out.stdout.trim()
            )))
        }
    }

    pub async fn push(&self, local: &Path, remote: &str) -> JidrResult<()> {
        self.run(&["push", &local.to_string_lossy(), remote])
            .await
            .map(|_| ())
    }

    pub async fn pull(&self, remote: &str, local: &Path) -> JidrResult<()> {
        self.run(&["pull", remote, &local.to_string_lossy()])
            .await
            .map(|_| ())
    }

    /// `logcat -d` with extra filter args.
    pub async fn logcat_dump(&self, args: &[&str]) -> JidrResult<String> {
        let mut all = vec!["logcat", "-d"];
        all.extend_from_slice(args);
        Ok(self.run(&all).await?.stdout)
    }

    pub async fn logcat_clear(&self) -> JidrResult<()> {
        self.run(&["logcat", "-c"]).await.map(|_| ())
    }

    /// Follows logcat; lines arrive on the returned receiver until the child
    /// exits or the receiver is dropped (which kills the child).
    pub fn logcat_follow(&self, args: &[&str]) -> JidrResult<LogcatStream> {
        let mut all = vec!["logcat"];
        all.extend_from_slice(args);
        let mut child = self
            .command(&all)
            .spawn()
            .map_err(|e| JidrError::sdk(format!("cannot run adb logcat: {e}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| JidrError::unexpected("adb logcat has no stdout"))?;
        let (tx, rx) = tokio::sync::mpsc::channel(1024);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
        });
        Ok(LogcatStream { child, lines: rx })
    }

    /// `adb root`. Images that don't allow it ("production builds") fail
    /// with `root_failed`.
    pub async fn root(&self) -> JidrResult<()> {
        self.adbd_mode("root", "restarting adbd as root", "already running as root")
            .await
    }

    pub async fn unroot(&self) -> JidrResult<()> {
        self.adbd_mode(
            "unroot",
            "restarting adbd as non root",
            "not running as root",
        )
        .await
    }

    async fn adbd_mode(&self, verb: &str, restarted: &str, already: &str) -> JidrResult<()> {
        let out = self.raw(&[verb]).await?;
        let text = format!("{}{}", out.stdout, out.stderr);
        if text.contains(restarted) || text.contains(already) {
            return Ok(());
        }
        if text.contains("cannot run as root") || text.contains("production builds") {
            return Err(JidrError::root_failed(format!(
                "adb {verb}: {}",
                text.trim()
            )));
        }
        if out.success() {
            return Ok(());
        }
        Err(self.classify(&[verb], &out))
    }

    /// `adb connect host:port` (needed past the ~16 instances adb scans, D9).
    pub async fn connect(&self, addr: &str) -> JidrResult<()> {
        let out = self.raw(&["connect", addr]).await?;
        if out.stdout.contains("connected to") {
            Ok(())
        } else {
            Err(JidrError::sdk(format!(
                "adb connect {addr}: {}",
                out.stdout.trim()
            )))
        }
    }

    pub async fn disconnect(&self, addr: &str) -> JidrResult<()> {
        self.raw(&["disconnect", addr]).await.map(|_| ())
    }

    /// Waits until `adb -s <serial>` reports `device`.
    pub async fn wait_for_device(&self, timeout: Duration) -> JidrResult<()> {
        let serial = self
            .serial
            .clone()
            .ok_or_else(|| JidrError::usage("wait_for_device needs a serial"))?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let base = Adb {
                serial: None,
                ..self.clone()
            };
            if base
                .devices()
                .await?
                .iter()
                .any(|d| d.serial == serial && d.state == "device")
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(JidrError::boot_timeout(format!(
                    "{serial} did not come online"
                )));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

/// A running `adb logcat`; dropping it kills the process.
#[derive(Debug)]
pub struct LogcatStream {
    child: Child,
    pub lines: tokio::sync::mpsc::Receiver<String>,
}

impl LogcatStream {
    pub async fn stop(mut self) {
        let _ = self.child.kill().await;
    }
}

pub fn parse_devices(text: &str) -> Vec<DeviceEntry> {
    text.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            Some(DeviceEntry {
                serial: parts.next()?.to_string(),
                state: parts.next()?.to_string(),
            })
        })
        .collect()
}

/// Extracts `Failure [REASON ...]` or `INSTALL_FAILED_…` from install output.
pub fn parse_install_failure(text: &str) -> Option<InstallFailure> {
    let reason = text
        .split(|c: char| c.is_whitespace() || c == '[' || c == ']' || c == ':')
        .find(|w| w.starts_with("INSTALL_FAILED_") || w.starts_with("INSTALL_PARSE_FAILED_"))?
        .to_string();
    let signature_mismatch = matches!(
        reason.as_str(),
        "INSTALL_FAILED_UPDATE_INCOMPATIBLE" | "INSTALL_FAILED_SHARED_USER_INCOMPATIBLE"
    );
    Some(InstallFailure {
        reason,
        signature_mismatch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_list() {
        let text = "* daemon started successfully\nList of devices attached\nemulator-5554\tdevice\n127.0.0.1:5585\toffline\n\n";
        assert_eq!(
            parse_devices(text),
            vec![
                DeviceEntry {
                    serial: "emulator-5554".into(),
                    state: "device".into()
                },
                DeviceEntry {
                    serial: "127.0.0.1:5585".into(),
                    state: "offline".into()
                },
            ]
        );
        assert!(parse_devices("List of devices attached\n").is_empty());
    }

    #[test]
    fn parses_install_failures() {
        let f = parse_install_failure(
            "Performing Streamed Install\nadb: failed to install x.apk: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package com.x signatures do not match]",
        )
        .unwrap();
        assert_eq!(f.reason, "INSTALL_FAILED_UPDATE_INCOMPATIBLE");
        assert!(f.signature_mismatch);
        let f = parse_install_failure("Failure [INSTALL_FAILED_VERSION_DOWNGRADE]").unwrap();
        assert!(!f.signature_mismatch);
        assert!(parse_install_failure("Success").is_none());
    }
}
