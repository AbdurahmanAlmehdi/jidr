//! Waiting for adb to answer after a cold boot or an adbd restart.
//!
//! When the emulator reboots, or adbd restarts for `adb root`/`unroot`, the
//! host adb server can keep a stale transport that stays `offline` for a
//! while, sometimes until `adb reconnect offline`. Linux KVM CI hit both:
//! "adb: device offline" on the Magisk app install right after the cold boot
//! (API 35), and "adbd did not settle to the expected uid" (API 30). So
//! every step that talks to the device after one waits here first.

use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::{Cancel, JidrError, JidrResult};

/// What to do next while waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Done,
    Wait,
    /// Run `adb reconnect offline`: drop the stale transport.
    Reconnect,
    GiveUp,
}

/// The wait/reconnect decision on elapsed time, so it can be tested without
/// a device: reconnect once the device hasn't answered for `grace`, then at
/// most every `every`; give up at `timeout`.
#[derive(Debug, Clone)]
pub struct Wait {
    timeout: Duration,
    grace: Duration,
    every: Duration,
    failing_since: Option<Duration>,
    last_reconnect: Option<Duration>,
}

impl Wait {
    pub fn new(timeout: Duration) -> Self {
        Self::with(timeout, Duration::from_secs(5), Duration::from_secs(10))
    }

    fn with(timeout: Duration, grace: Duration, every: Duration) -> Self {
        Self {
            timeout,
            grace,
            every,
            failing_since: None,
            last_reconnect: None,
        }
    }

    /// `answered`: the probe gave the expected answer. `now`: time since the
    /// wait began.
    pub fn step(&mut self, answered: bool, now: Duration) -> Step {
        if answered {
            return Step::Done;
        }
        if now >= self.timeout {
            return Step::GiveUp;
        }
        let since = *self.failing_since.get_or_insert(now);
        let due = self.last_reconnect.is_none_or(|t| now >= t + self.every);
        if now >= since + self.grace && due {
            self.last_reconnect = Some(now);
            return Step::Reconnect;
        }
        Step::Wait
    }
}

const POLL: Duration = Duration::from_millis(500);

/// Runs `probe` in the device shell until `ok` accepts its output,
/// reconnecting a stale transport on the way. `what` names the wait in the
/// error ("after the cold boot").
pub async fn until(
    adb: &Adb,
    cancel: Option<&Cancel>,
    timeout: Duration,
    what: &str,
    probe: &str,
    ok: impl Fn(&str) -> bool,
) -> JidrResult<()> {
    until_with(adb, cancel, Wait::new(timeout), what, probe, ok).await
}

async fn until_with(
    adb: &Adb,
    cancel: Option<&Cancel>,
    mut wait: Wait,
    what: &str,
    probe: &str,
    ok: impl Fn(&str) -> bool,
) -> JidrResult<()> {
    let timeout = wait.timeout;
    let shell = adb.clone().with_timeout(Duration::from_secs(5));
    let host = Adb::new(adb.bin()).with_timeout(Duration::from_secs(10));
    let start = tokio::time::Instant::now();
    let mut last: String;
    loop {
        if let Some(c) = cancel {
            c.check()?;
        }
        let answered = match shell.shell_status(probe).await {
            Ok((_, out)) => {
                let good = ok(&out);
                last = out.trim().to_string();
                good
            }
            Err(e) => {
                last = e.message;
                false
            }
        };
        match wait.step(answered, start.elapsed()) {
            Step::Done => return Ok(()),
            Step::Wait => {}
            Step::Reconnect => {
                // A stale transport shows as `offline`, or as `device` with a
                // shell that hangs (CI, API 30: "state: device; last: adb
                // shell id -u timed out"). `reconnect offline` only drops the
                // first kind, so also kick this device's own transport.
                // While the device is still booting it isn't `device` yet,
                // and kicking it then only slows the reconnect.
                let _ = host.raw(&["reconnect", "offline"]).await;
                if adb.serial().is_some() && state(&shell).await == "device" {
                    let _ = adb
                        .clone()
                        .with_timeout(Duration::from_secs(10))
                        .raw(&["reconnect"])
                        .await;
                }
            }
            Step::GiveUp => {
                let state = state(&shell).await;
                return Err(JidrError::root_failed(format!(
                    "{what}: adb didn't answer within {} s (state: {state}; last: {last})",
                    timeout.as_secs()
                ))
                .with_data(serde_json::json!({ "adb_state": state, "last": last })));
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// `adb get-state` for the device: `device`, `offline`, or the error text.
async fn state(adb: &Adb) -> String {
    adb.raw(&["get-state"])
        .await
        .map(|o| format!("{}{}", o.stdout, o.stderr).trim().to_string())
        .unwrap_or_default()
}

/// `adb root` (`root`) or `adb unroot`, once the shell answers.
///
/// Regression (CI, API 35): right after a boot the transport was stale and
/// `adb root` failed with "unable to connect for root: closed". So wait for
/// the shell first, and when the transport still drops during the call, wait
/// again and retry once. A refused switch ("production builds") isn't retried.
/// The caller then waits for the new uid.
pub async fn switch_adbd(
    adb: &Adb,
    cancel: Option<&Cancel>,
    root: bool,
    ready: Duration,
) -> JidrResult<()> {
    let wait = || {
        until(
            adb,
            cancel,
            ready,
            "before switching adbd",
            "echo ok",
            |o| o.trim() == "ok",
        )
    };
    let run = || async {
        if root {
            adb.root().await
        } else {
            adb.unroot().await
        }
    };
    wait().await?;
    match run().await {
        Err(e) if transport_dropped(&e) => {
            wait().await?;
            run().await
        }
        r => r,
    }
}

/// adb lost the device mid-call (as opposed to refusing the switch).
fn transport_dropped(e: &JidrError) -> bool {
    let m = e.message.to_lowercase();
    ["unable to connect", "closed", "offline", "no devices"]
        .iter()
        .any(|s| m.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    #[test]
    fn waits_then_reconnects_at_most_every_ten_seconds() {
        let mut w = Wait::new(s(90.0));
        assert_eq!(w.step(true, s(0.0)), Step::Done);

        let mut w = Wait::new(s(90.0));
        assert_eq!(w.step(false, s(0.0)), Step::Wait);
        assert_eq!(w.step(false, s(4.9)), Step::Wait);
        assert_eq!(w.step(false, s(5.0)), Step::Reconnect);
        assert_eq!(w.step(false, s(9.0)), Step::Wait);
        assert_eq!(w.step(false, s(15.0)), Step::Reconnect);
        assert_eq!(w.step(true, s(16.0)), Step::Done);
    }

    #[test]
    fn gives_up_at_the_timeout() {
        let mut w = Wait::new(s(60.0));
        assert_eq!(w.step(false, s(1.0)), Step::Wait);
        assert_eq!(w.step(false, s(60.0)), Step::GiveUp);
    }

    /// Regression (CI, API 35): the transport stayed offline after the cold
    /// boot until `adb reconnect offline`. A fake adb reports "device
    /// offline" until it is reconnected, then answers.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_offline_transport_is_reconnected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let flag = dir.path().join("reconnected");
        let bin = dir.path().join("adb");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\n\
                 case \"$1\" in\n\
                 reconnect) touch {f}; echo reconnected;;\n\
                 shell) if [ -f {f} ]; then echo ok; else echo 'error: device offline' >&2; exit 1; fi;;\n\
                 get-state) echo offline;;\n\
                 esac\n",
                f = flag.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = Adb::new(&bin).device("emulator-5554");
        let wait = Wait::with(s(10.0), s(0.2), s(1.0));
        let r = until_with(&adb, None, wait, "test", "echo ok", |o| o.trim() == "ok").await;
        assert!(r.is_ok(), "{r:?}");
        assert!(flag.exists(), "reconnect offline ran");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_device_that_never_answers_fails_with_its_state() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("adb");
        std::fs::write(
            &bin,
            "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\n\
             case \"$1\" in\n\
             shell) echo 'error: device offline' >&2; exit 1;;\n\
             get-state) echo offline;;\n\
             esac\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = Adb::new(&bin).device("emulator-5554");
        let wait = Wait::with(s(1.0), s(0.2), s(1.0));
        let e = until_with(&adb, None, wait, "after the cold boot", "echo ok", |o| {
            o.trim() == "ok"
        })
        .await
        .unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::RootFailed);
        assert!(
            e.message
                .starts_with("after the cold boot: adb didn't answer within 1 s"),
            "{}",
            e.message
        );
        assert!(e.message.contains("state: offline"), "{}", e.message);
    }

    /// Regression (CI, API 30): adb listed the device as `device` while its
    /// shell hung, and `reconnect offline` never helped. A fake adb whose
    /// shell hangs until the device's own transport is reconnected.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_hung_transport_on_a_live_device_is_kicked() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let flag = dir.path().join("kicked");
        let bin = dir.path().join("adb");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\n\
                 case \"$1\" in\n\
                 reconnect) if [ \"$2\" != offline ]; then touch {f}; fi;;\n\
                 shell) if [ -f {f} ]; then echo ok; else sleep 30; fi;;\n\
                 get-state) echo device;;\n\
                 esac\n",
                f = flag.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = Adb::new(&bin).device("emulator-5554");
        let wait = Wait::with(s(20.0), s(0.2), s(1.0));
        let r = until_with(&adb, None, wait, "test", "echo ok", |o| o.trim() == "ok").await;
        assert!(r.is_ok(), "{r:?}");
    }

    /// A fake adb whose shell answers, and whose first `adb root` fails the
    /// way the API 35 CI run did.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_closed_transport_during_adb_root_is_retried() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tried = dir.path().join("tried");
        let bin = dir.path().join("adb");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nwhile [ \"$1\" = -s ]; do shift 2; done\n\
                 case \"$1\" in\n\
                 shell) echo ok;;\n\
                 root) if [ -f {f} ]; then echo 'restarting adbd as root'; \
                 else touch {f}; echo 'adb: unable to connect for root: closed' >&2; exit 1; fi;;\n\
                 esac\n",
                f = tried.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = Adb::new(&bin).device("emulator-5554");
        let r = switch_adbd(&adb, None, true, Duration::from_secs(5)).await;
        assert!(r.is_ok(), "{r:?}");
        assert!(tried.exists());
    }

    #[test]
    fn a_refused_switch_is_not_retried() {
        assert!(transport_dropped(&JidrError::sdk(
            "adb root failed: adb: unable to connect for root: closed"
        )));
        assert!(!transport_dropped(&JidrError::root_failed(
            "adb root: adbd cannot run as root in production builds"
        )));
    }
}
