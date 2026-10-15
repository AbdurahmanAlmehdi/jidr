//! A running device as the widgets see it: adb, gRPC and console handles.

use std::time::Duration;

use jidr_adb::Adb;
use jidr_backend::Console;
use jidr_core::provider::CallCtx;
use jidr_core::{ConnectInfo, Device, JidrError, JidrResult};
use jidr_proto::{Controller, Snapshots};

#[derive(Debug, Clone)]
pub struct Target {
    pub device: Device,
    pub info: ConnectInfo,
}

/// Resolves a running device (`not_found` when it's stopped).
pub async fn target(ctx: &CallCtx, selector: &str) -> JidrResult<Target> {
    let device = ctx.daemon.device(selector).await?;
    let info = ctx.daemon.connect_info(device.id).await.map_err(|e| {
        if e.code == jidr_core::ErrorCode::NotFound {
            JidrError::not_found(format!("{} isn't running; start it first", device.name))
        } else {
            e
        }
    })?;
    Ok(Target { device, info })
}

impl Target {
    pub fn adb(&self) -> Adb {
        Adb::new(self.info.adb_path.clone()).device(self.info.adb_serial.clone())
    }

    pub async fn grpc(&self) -> JidrResult<Controller> {
        Ok(
            jidr_backend::grpc::connect_port(self.info.grpc_port, &self.info.grpc_token)
                .await?
                .0,
        )
    }

    pub async fn snapshots(&self) -> JidrResult<Snapshots> {
        Ok(
            jidr_backend::grpc::connect_port(self.info.grpc_port, &self.info.grpc_token)
                .await?
                .1,
        )
    }

    pub async fn console(&self) -> JidrResult<Console> {
        let token = jidr_backend::console::default_token_path();
        Console::connect(self.info.console_port, token.as_deref()).await
    }

    /// Runs a console command and fails on `KO:` replies.
    pub async fn console_cmd(&self, cmd: &str) -> JidrResult<Vec<String>> {
        let mut c = self.console().await?;
        c.command(cmd).await
    }
}

/// Maps a gRPC failure to a Jidr error.
pub fn grpc_err(what: &str) -> impl FnOnce(tonic::Status) -> JidrError + '_ {
    move |s| jidr_backend::grpc::status_error(what, s)
}

/// What to do next while waiting for a device to answer adb again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitStep {
    Done,
    Wait,
    /// Run `adb reconnect offline`: the host adb server holds a stale
    /// transport (after a snapshot restore it can stay `offline` for good).
    Reconnect,
    GiveUp,
}

/// The retry/reconnect decision for [`wait_for_adb`], on elapsed time so it
/// can be tested without a device.
#[derive(Debug, Clone)]
pub struct AdbWait {
    timeout: Duration,
    /// Reconnect after the device hasn't answered for this long…
    grace: Duration,
    /// …and at most this often.
    every: Duration,
    failing_since: Option<Duration>,
    last_reconnect: Option<Duration>,
}

impl AdbWait {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            grace: Duration::from_secs(5),
            every: Duration::from_secs(10),
            failing_since: None,
            last_reconnect: None,
        }
    }

    /// `answered`: did `adb shell echo ok` print `ok`? `now`: time since
    /// the wait began.
    pub fn step(&mut self, answered: bool, now: Duration) -> WaitStep {
        if answered {
            return WaitStep::Done;
        }
        if now >= self.timeout {
            return WaitStep::GiveUp;
        }
        let since = *self.failing_since.get_or_insert(now);
        let due = self.last_reconnect.is_none_or(|t| now >= t + self.every);
        if now >= since + self.grace && due {
            self.last_reconnect = Some(now);
            return WaitStep::Reconnect;
        }
        WaitStep::Wait
    }
}

/// Waits until the device answers `adb shell echo ok`, reconnecting stale
/// transports on the way. `what` names the step for the error ("restored").
pub async fn wait_for_adb(
    ctx: &CallCtx,
    t: &Target,
    timeout: Duration,
    what: &str,
) -> JidrResult<()> {
    let adb = t.adb().with_timeout(Duration::from_secs(5));
    let host = Adb::new(t.info.adb_path.clone()).with_timeout(Duration::from_secs(10));
    let start = tokio::time::Instant::now();
    let mut wait = AdbWait::new(timeout);
    let mut last = String::new();
    let mut reported = false;
    loop {
        if ctx.cancel.is_cancelled() {
            return Err(jidr_core::cancel::cancelled());
        }
        let answered = match adb.raw(&["shell", "echo ok"]).await {
            Ok(out) if out.stdout.trim() == "ok" => true,
            Ok(out) => {
                last = format!("{}{}", out.stderr, out.stdout).trim().to_string();
                false
            }
            Err(e) => {
                last = e.message;
                false
            }
        };
        match wait.step(answered, start.elapsed()) {
            WaitStep::Done => return Ok(()),
            WaitStep::GiveUp => {
                let state = adb
                    .raw(&["get-state"])
                    .await
                    .map(|o| format!("{}{}", o.stdout, o.stderr).trim().to_string())
                    .unwrap_or_default();
                return Err(JidrError::boot_timeout(format!(
                    "{} {what}, but it didn't answer adb within {} s (state: {state}; last: {last})",
                    t.device.name,
                    timeout.as_secs()
                ))
                .with_data(serde_json::json!({ "adb_state": state, "last_error": last })));
            }
            WaitStep::Reconnect => {
                tracing::info!(serial = %t.info.adb_serial, %last, "adb reconnect offline");
                let _ = host.raw(&["reconnect", "offline"]).await;
            }
            WaitStep::Wait => {}
        }
        if !reported {
            ctx.report(None, "Waiting for the device");
            reported = true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    /// Regression (PR #7 CI, API 33/35): after a snapshot restore adb shell
    /// printed nothing for 30 s. snapshot.load now waits, and reconnects a
    /// transport that stays offline.
    #[test]
    fn waits_then_reconnects_offline_devices_at_most_every_ten_seconds() {
        let mut w = AdbWait::new(s(60.0));
        assert_eq!(w.step(true, s(0.0)), WaitStep::Done);

        let mut w = AdbWait::new(s(60.0));
        assert_eq!(w.step(false, s(0.0)), WaitStep::Wait);
        assert_eq!(w.step(false, s(4.9)), WaitStep::Wait);
        assert_eq!(w.step(false, s(5.0)), WaitStep::Reconnect);
        assert_eq!(w.step(false, s(9.0)), WaitStep::Wait);
        assert_eq!(w.step(false, s(15.0)), WaitStep::Reconnect);
        assert_eq!(w.step(true, s(16.0)), WaitStep::Done);
    }

    #[test]
    fn gives_up_at_the_timeout() {
        let mut w = AdbWait::new(s(60.0));
        assert_eq!(w.step(false, s(1.0)), WaitStep::Wait);
        assert_eq!(w.step(false, s(60.0)), WaitStep::GiveUp);
        // A late answer still counts.
        assert_eq!(w.step(true, s(61.0)), WaitStep::Done);
    }
}
