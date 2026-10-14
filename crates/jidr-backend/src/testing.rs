//! Emulator cleanup for tests that boot real emulators. Not used by the
//! product: there, emulators outlive the daemon on purpose (re-adoption).
//!
//! Three layers, so no test leaves an emulator behind whatever happens:
//! 1. [`AvdGuard`]: kills the AVD's emulator when dropped (pass, failure, panic).
//! 2. A reaper process per test process, in its own process group: when the
//!    test process dies (SIGKILL, Ctrl-C, nextest timeout, where `Drop` never
//!    runs) it kills every emulator whose AVD the test registered.
//! 3. [`kill_orphans`]: at startup, kills orphaned emulators (parent gone)
//!    left by earlier runs of test AVDs with a given prefix.
//!
//! Test AVD names carry the test process id ([`unique_avd`]) so a cleanup
//! never touches another run's emulators.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// `<base>-<pid>`: an AVD name unique to this test process.
pub fn unique_avd(base: &str) -> String {
    format!("{base}-{}", std::process::id())
}

/// Pids of processes running AVD `avd` (`-avd <avd>` in their command line:
/// the `emulator` launcher or qemu). Test AVD names are unique per process,
/// so an exact match can't hit anyone else's emulator.
fn emulator_pids(avd: &str) -> Vec<u32> {
    let Ok(out) = Command::new("ps").args(["-axo", "pid=,command="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let pid: u32 = parts.next()?.parse().ok()?;
            let args: Vec<&str> = parts.collect();
            args.windows(2)
                .any(|w| w[0] == "-avd" && w[1] == avd)
                .then_some(pid)
        })
        .collect()
}

/// Kills every emulator running `avd` (SIGKILL: tests don't need a snapshot).
pub fn kill_avd(avd: &str) {
    for pid in emulator_pids(avd) {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

/// Kills orphaned emulators (reparented to launchd/init) whose AVD starts
/// with `prefix`: leftovers of earlier, killed test runs.
pub fn kill_orphans(prefix: &str) -> usize {
    let Ok(out) = Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
    else {
        return 0;
    };
    let mut killed = 0;
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = l.split_whitespace();
        let (Some(pid), Some(ppid)) = (parts.next(), parts.next()) else {
            continue;
        };
        let args: Vec<&str> = parts.collect();
        let orphan = ppid == "1";
        let emulator = args.first().is_some_and(|c| c.contains("qemu-system"));
        let ours = args
            .windows(2)
            .any(|w| w[0] == "-avd" && w[1].starts_with(prefix));
        if orphan
            && emulator
            && ours
            && Command::new("kill")
                .args(["-9", pid])
                .status()
                .is_ok_and(|s| s.success())
        {
            killed += 1;
        }
    }
    killed
}

struct Reaper {
    list: PathBuf,
}

/// Starts (once per process) the watcher that cleans up after this process.
fn reaper() -> Option<&'static Reaper> {
    static REAPER: OnceLock<Option<Reaper>> = OnceLock::new();
    REAPER
        .get_or_init(|| {
            let me = std::process::id();
            let list = std::env::temp_dir().join(format!("jidr-test-reaper-{me}.avds"));
            std::fs::write(&list, "").ok()?;
            // Waits for this process to exit, then kills the listed AVDs'
            // emulators by exact `-avd <name>` match.
            let script = format!(
                r#"while kill -0 {me} 2>/dev/null; do sleep 2; done
while IFS= read -r avd; do
  [ -n "$avd" ] || continue
  ps -axo pid=,command= | awk -v a="$avd" '{{ for (i = 2; i < NF; i++) if ($i == "-avd" && $(i+1) == a) print $1 }}' | xargs kill -9 2>/dev/null
done < "{list}"
rm -f "{list}""#,
                list = list.display()
            );
            let mut cmd = Command::new("sh");
            cmd.args(["-c", &script]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                // Its own process group: a Ctrl-C to the test run doesn't kill it.
                cmd.process_group(0);
            }
            cmd.spawn().ok()?;
            Some(Reaper { list })
        })
        .as_ref()
}

/// Ensures `avd`'s emulator dies with the test: on drop, and via the reaper
/// if the test process is killed.
#[derive(Debug)]
pub struct AvdGuard {
    avd: String,
}

impl AvdGuard {
    pub fn new(avd: &str) -> Self {
        if let Some(r) = reaper()
            && let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&r.list)
        {
            let _ = writeln!(f, "{avd}");
        }
        Self {
            avd: avd.to_string(),
        }
    }
}

impl Drop for AvdGuard {
    fn drop(&mut self) {
        kill_avd(&self.avd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in process with `-avd <name>` in its command line dies when
    /// its guard drops.
    #[test]
    fn guard_kills_its_avd_on_drop() {
        let avd = unique_avd("jidr_guard_test");
        let mut child = Command::new("sh")
            .args(["-c", "sleep 300; :", "emulator", "-avd", &avd])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(emulator_pids(&avd), vec![child.id()]);
        drop(AvdGuard::new(&avd));
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(child.try_wait().unwrap().is_some(), "the guard killed it");
        assert!(emulator_pids(&avd).is_empty());
    }

    #[test]
    fn unique_names_carry_the_pid() {
        assert!(unique_avd("x").ends_with(&std::process::id().to_string()));
    }
}
