//! Crash detection (D9): notice when an emulator dies or stops answering,
//! save its logs, report. Never restarts it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, timeout};

use crate::aosp::{self, AospBackend};
use crate::{InstanceInfo, grpc};

/// Consecutive failed heartbeats before a live process counts as hung.
const HEARTBEAT_FAILURES: u32 = 3;
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrashKind {
    /// The process exited without being asked to stop.
    Exited,
    /// The process is alive but gRPC stopped answering.
    Unresponsive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CrashReport {
    pub avd: String,
    pub pid: u32,
    pub kind: CrashKind,
    pub reason: String,
    pub exit_code: Option<i32>,
    pub at: DateTime<Utc>,
    /// The saved crash log (reason + the emulator's own log).
    pub log: Option<PathBuf>,
}

impl AospBackend {
    /// Resolves when the instance crashes. The daemon aborts this future
    /// before stopping a device on purpose.
    pub async fn crash_watch(&self, info: &InstanceInfo, log_dir: &Path) -> CrashReport {
        let (kind, exit_code) = self.watch(info).await;
        let reason = match (&kind, exit_code) {
            (CrashKind::Exited, Some(c)) => format!("emulator exited with status {c}"),
            (CrashKind::Exited, None) => {
                "emulator was killed by a signal or disappeared".to_string()
            }
            (CrashKind::Unresponsive, _) => format!(
                "emulator stopped answering gRPC ({} missed heartbeats)",
                HEARTBEAT_FAILURES
            ),
        };
        tracing::warn!(avd = %info.avd, pid = info.pid, %reason, "emulator crashed");
        let at = Utc::now();
        let log = save_crash_log(info, log_dir, &reason, at);
        CrashReport {
            avd: info.avd.clone(),
            pid: info.pid,
            kind,
            reason,
            exit_code,
            at,
            log,
        }
    }

    async fn watch(&self, info: &InstanceInfo) -> (CrashKind, Option<i32>) {
        let mut failures = 0;
        let mut controller = None;
        loop {
            // Process exit: the exit watcher for owned instances, polling otherwise.
            match self.exit_watch(info.pid) {
                Some(mut rx) => {
                    if let Ok(Ok(state)) =
                        timeout(HEARTBEAT_EVERY, rx.wait_for(Option::is_some)).await
                    {
                        let code = (*state).flatten();
                        return (CrashKind::Exited, code);
                    }
                }
                None => {
                    sleep(HEARTBEAT_EVERY).await;
                    if !aosp::process_alive(info.pid).await {
                        return (CrashKind::Exited, None);
                    }
                }
            }
            // Heartbeat.
            if controller.is_none() {
                controller = grpc::connect(info).await.ok().map(|(c, _)| c);
            }
            let ok = match controller.as_mut() {
                Some(c) => timeout(HEARTBEAT_EVERY, c.get_status(()))
                    .await
                    .is_ok_and(|r| r.is_ok()),
                None => false,
            };
            if ok {
                failures = 0;
            } else {
                failures += 1;
                controller = None;
                if failures >= HEARTBEAT_FAILURES && aosp::process_alive(info.pid).await {
                    return (CrashKind::Unresponsive, None);
                }
            }
        }
    }
}

fn save_crash_log(
    info: &InstanceInfo,
    log_dir: &Path,
    reason: &str,
    at: DateTime<Utc>,
) -> Option<PathBuf> {
    std::fs::create_dir_all(log_dir).ok()?;
    let path = log_dir.join(format!("crash-{}.log", at.format("%Y%m%d-%H%M%S")));
    let emulator_log = info
        .log_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| {
            "(no emulator log: the instance was adopted after a daemon restart)".into()
        });
    let text = format!(
        "Jidr crash report\navd: {}\npid: {}\nat: {at}\nreason: {reason}\nemulator log: {}\n\n--- emulator log ---\n{emulator_log}",
        info.avd,
        info.pid,
        info.log_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    );
    std::fs::write(&path, text).ok()?;
    Some(path)
}
