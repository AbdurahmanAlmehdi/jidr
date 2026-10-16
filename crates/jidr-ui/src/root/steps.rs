//! Full-root setup as the five steps of the Root sheet (the root design):
//! Download, Patch, Reboot, Install Magisk app, Verify. `root.set magisk`
//! reports finer job messages (`crates/jidr-root/src/magisk/mod.rs`); this
//! maps them onto the steps, monotonically, and classifies a failure.

use jidr_core::{ErrorCode, JidrError};

/// The design's steps, in order.
pub const STEPS: usize = 5;
pub const DOWNLOAD: usize = 0;
pub const PATCH: usize = 1;
pub const REBOOT: usize = 2;
pub const INSTALL: usize = 3;
pub const VERIFY: usize = 4;

/// The step a `root.set magisk` job message belongs to, given the step the
/// job is on. Unknown messages keep the current step.
pub fn step_for(message: &str, current: usize) -> usize {
    let m = message.trim();
    let mapped = if m.starts_with("Resolving the Magisk release")
        || m.starts_with("Downloading Magisk")
        || (m.starts_with("Magisk ") && m.contains("GPL"))
    {
        DOWNLOAD
    } else if m.starts_with("Patching the ramdisk") {
        PATCH
    } else if m.starts_with("Stopping before the cold boot") || m.starts_with("Cold booting") {
        // The first cold boot boots the patched ramdisk; the second one,
        // after the app and its environment, belongs to the install step.
        if current >= INSTALL { INSTALL } else { REBOOT }
    } else if m.starts_with("Installing the Magisk app")
        || m.starts_with("Setting up Magisk's environment")
        || m.starts_with("Play Store: open the Magisk app")
        || m == "Rebooting"
    {
        INSTALL
    } else if m.starts_with("Granting the adb shell su access")
        || is_grant_prompt(m)
        || m.starts_with("Verifying root")
    {
        VERIFY
    } else {
        current
    };
    mapped.max(current)
}

/// The Play Store guided tap (ADR-0002): the job waits for the user to tap
/// Grant in Magisk's prompt.
pub fn is_grant_prompt(message: &str) -> bool {
    message.starts_with("Magisk will ask to grant root to Shell")
}

/// What the Root sheet shows after a failed or stopped job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// After "Root failed: ".
    pub reason: String,
    /// The step that failed (0-based).
    pub step: usize,
    /// The guided tap was denied or timed out: "Root unverified", not failed.
    pub unverified: bool,
    /// The user stopped it.
    pub cancelled: bool,
    /// The daemon's message, for steps without a designed explanation.
    pub message: String,
}

/// Classifies the error a root job ended with, on the step it reached.
pub fn failure(err: &JidrError, step: usize) -> Failure {
    let data = err.data.as_ref();
    let flag = |k: &str| data.and_then(|d| d.get(k));
    // `jidr_core::cancel::cancelled()`: `unexpected` with `data.cancelled`.
    let cancelled = flag("cancelled").and_then(|v| v.as_bool()) == Some(true);
    let unverified = flag("state").and_then(|v| v.as_str()) == Some("awaiting_user_grant");
    let reason = if cancelled {
        "stopped".to_string()
    } else if unverified {
        "Shell wasn't granted root".to_string()
    } else if err.code == ErrorCode::BootTimeout {
        "boot timed out".to_string()
    } else if err.message.contains("su -c id") {
        "su didn't answer as root".to_string()
    } else {
        match step {
            DOWNLOAD => "download failed",
            PATCH => "patch step failed",
            REBOOT => "boot failed",
            INSTALL => "Magisk app install failed",
            _ => "verification failed",
        }
        .to_string()
    };
    Failure {
        reason,
        step: if err.code == ErrorCode::BootTimeout && step < REBOOT {
            REBOOT
        } else {
            step
        },
        unverified,
        cancelled,
        message: err.message.clone(),
    }
}

/// `m:ss` for a number of seconds.
pub fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// One `.logtail` line: `HH:MM:SS  source L text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub time: String,
    pub source: &'static str,
    /// "", "w" or "e".
    pub level: &'static str,
    pub text: String,
}

/// The last `n` progress lines of a job, plus how it ended.
pub fn log_tail(entries: &[LogEntry], n: usize) -> Vec<LogEntry> {
    entries[entries.len().saturating_sub(n)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The messages `root.set magisk` reports on a google_apis image, in
    /// order (crates/jidr-root/src/magisk/mod.rs).
    const GOOGLE_APIS: &[(&str, usize)] = &[
        ("Resolving the Magisk release", DOWNLOAD),
        (
            "Magisk 30.7 is GPL-3.0-only; source: https://github.com/topjohnwu/Magisk. Jidr downloads it at runtime and never bundles it.",
            DOWNLOAD,
        ),
        ("Downloading Magisk 30.7", DOWNLOAD),
        ("Patching the ramdisk for Magisk 30.7", PATCH),
        ("Stopping before the cold boot", REBOOT),
        ("Cold booting", REBOOT),
        ("Installing the Magisk app", INSTALL),
        ("Setting up Magisk's environment", INSTALL),
        ("Rebooting", INSTALL),
        ("Stopping before the cold boot", INSTALL),
        ("Cold booting", INSTALL),
        ("Granting the adb shell su access", VERIFY),
        ("Verifying root", VERIFY),
    ];

    #[test]
    fn job_messages_map_onto_the_five_steps() {
        let mut step = DOWNLOAD;
        for (msg, want) in GOOGLE_APIS {
            step = step_for(msg, step);
            assert_eq!(step, *want, "{msg}");
        }
    }

    #[test]
    fn play_store_messages_reach_the_guided_tap() {
        let mut step = PATCH;
        for msg in [
            "Cold booting",
            "Installing the Magisk app",
            "Play Store: open the Magisk app to finish its setup, then grant root to Shell",
            "Rebooting",
            "Cold booting",
            "Magisk will ask to grant root to Shell. Tap Grant in the device window.",
        ] {
            step = step_for(msg, step);
        }
        assert_eq!(step, VERIFY);
        assert!(is_grant_prompt(
            "Magisk will ask to grant root to Shell. Tap Grant in the device window."
        ));
    }

    #[test]
    fn steps_never_go_back() {
        assert_eq!(step_for("Downloading Magisk 30.7", INSTALL), INSTALL);
        assert_eq!(step_for("something new", REBOOT), REBOOT);
        // A cached patch skips straight from resolving to the cold boot.
        assert_eq!(step_for("Cold booting", DOWNLOAD), REBOOT);
    }

    #[test]
    fn failures_say_what_went_wrong() {
        let boot = JidrError::new(ErrorCode::BootTimeout, "dev did not boot within 120 s");
        let f = failure(&boot, REBOOT);
        assert_eq!((f.reason.as_str(), f.step), ("boot timed out", REBOOT));
        assert!(!f.unverified && !f.cancelled);

        let grant = JidrError::new(
            ErrorCode::RootFailed,
            "root grant to Shell was denied or timed out",
        )
        .with_data(json!({ "state": "awaiting_user_grant" }));
        assert!(failure(&grant, VERIFY).unverified);

        let stopped = JidrError::unexpected("cancelled").with_data(json!({ "cancelled": true }));
        let f = failure(&stopped, PATCH);
        assert!(f.cancelled);
        assert_eq!(f.reason, "stopped");

        let su = JidrError::new(
            ErrorCode::RootFailed,
            "dev: `su -c id` reported uid 2000, expected 0 (root)",
        );
        assert_eq!(failure(&su, VERIFY).reason, "su didn't answer as root");

        let other = JidrError::new(ErrorCode::Sdk, "connection reset");
        let f = failure(&other, DOWNLOAD);
        assert_eq!(f.reason, "download failed");
        assert_eq!(f.message, "connection reset");
    }

    #[test]
    fn clock_and_tail() {
        assert_eq!(clock(18), "0:18");
        assert_eq!(clock(120), "2:00");
        let e = |t: &str| LogEntry {
            time: "14:20:31".into(),
            source: "jidr",
            level: "",
            text: t.into(),
        };
        let all = vec![e("a"), e("b"), e("c")];
        assert_eq!(log_tail(&all, 2), vec![e("b"), e("c")]);
        assert_eq!(log_tail(&all, 9).len(), 3);
    }
}
