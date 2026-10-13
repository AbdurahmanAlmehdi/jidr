//! The AOSP emulator backend: spawn headless with gRPC, wait for boot, stop.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use jidr_adb::Adb;
use jidr_core::{Cancel, ErrorCode, JidrError, JidrResult, ResultExt};
use jidr_proto::control::{VmRunState, vm_run_state::RunState};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout};

use crate::grpc;
use crate::ports::{PortAllocator, Ports};
use crate::{
    Backend, BootProgress, BootStage, InstanceInfo, LaunchOptions, LaunchSpec, default_gpu,
};

/// How long the emulator gets to write its discovery file.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a graceful shutdown (which saves the quick-boot snapshot) may take.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);

/// Exit status of an emulator we spawned: `None` while running.
pub type ExitWatch = watch::Receiver<Option<Option<i32>>>;

#[derive(Debug, Default)]
pub struct AospBackend {
    ports: PortAllocator,
    /// Exit watchers of the emulators this process spawned, by pid.
    exits: Arc<Mutex<HashMap<u32, ExitWatch>>>,
    /// AVDs being launched right now (before the emulator holds its lock).
    launching: Arc<Mutex<HashSet<std::path::PathBuf>>>,
}

impl AospBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ports(&self) -> &PortAllocator {
        &self.ports
    }

    /// Watches the exit of a spawned emulator (crash detection).
    pub fn exit_watch(&self, pid: u32) -> Option<ExitWatch> {
        self.exits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&pid)
            .cloned()
    }

    /// The emulator command line (without the binary).
    pub fn args(spec: &LaunchSpec, opts: &LaunchOptions, ports: Ports) -> Vec<String> {
        let mut a: Vec<String> = [
            "-avd",
            &spec.avd,
            "-no-window",
            "-no-boot-anim",
            "-grpc-use-token",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        a.extend(["-grpc".into(), ports.grpc.to_string()]);
        a.extend(["-port".into(), ports.console.to_string()]);
        a.extend([
            "-gpu".into(),
            opts.gpu.clone().unwrap_or_else(|| default_gpu().into()),
        ]);
        if opts.cold_boot {
            a.push("-no-snapshot-load".into());
        }
        if !opts.snapshot_save {
            a.push("-no-snapshot-save".into());
        }
        if opts.wipe {
            a.push("-wipe-data".into());
        }
        if !opts.audio {
            a.push("-no-audio".into());
        }
        if let Some(r) = &opts.ramdisk {
            a.extend(["-ramdisk".into(), r.to_string_lossy().into_owned()]);
        }
        a.extend(opts.extra_args.iter().cloned());
        a
    }

    async fn launch(
        &self,
        spec: &LaunchSpec,
        opts: &LaunchOptions,
        ports: Ports,
        progress: BootProgress<'_>,
        cancel: &Cancel,
    ) -> JidrResult<InstanceInfo> {
        std::fs::create_dir_all(&spec.log_dir).ctx(
            ErrorCode::Sdk,
            format!("creating {}", spec.log_dir.display()),
        )?;
        let log_path = spec.log_dir.join(format!(
            "emulator-{}.log",
            Utc::now().format("%Y%m%d-%H%M%S")
        ));
        let log = std::fs::File::create(&log_path)
            .ctx(ErrorCode::Sdk, format!("creating {}", log_path.display()))?;
        let log_err = log
            .try_clone()
            .ctx(ErrorCode::Sdk, "duplicating log handle")?;
        let args = Self::args(spec, opts, ports);
        tracing::info!(avd = %spec.avd, ?ports, "starting emulator: {}", args.join(" "));
        let mut child = Command::new(&spec.emulator)
            .args(&args)
            .env("ANDROID_AVD_HOME", &spec.avd_home)
            .env("ANDROID_SDK_ROOT", &spec.sdk_root)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .map_err(|e| JidrError::sdk(format!("cannot run {}: {e}", spec.emulator.display())))?;
        let pid = child
            .id()
            .ok_or_else(|| JidrError::unexpected("emulator exited before it had a pid"))?;
        let (tx, rx) = watch::channel(None);
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(|s| s.code());
            let _ = tx.send(Some(code));
        });
        self.exits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pid, rx.clone());
        progress(BootStage::Spawned);

        let result = self
            .wait_boot(spec, opts, ports, pid, &log_path, &rx, progress, cancel)
            .await;
        if result.is_err() {
            kill(pid).await;
            self.exits
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&pid);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn wait_boot(
        &self,
        spec: &LaunchSpec,
        opts: &LaunchOptions,
        ports: Ports,
        pid: u32,
        log_path: &Path,
        exit: &ExitWatch,
        progress: BootProgress<'_>,
        cancel: &Cancel,
    ) -> JidrResult<InstanceInfo> {
        let started = Instant::now();
        let check = |stage: &str| -> JidrResult<()> {
            cancel.check()?;
            if let Some(code) = *exit.borrow() {
                return Err(JidrError::sdk(format!(
                    "emulator exited ({}) while {stage}; see {}",
                    code.map(|c| c.to_string())
                        .unwrap_or_else(|| "signal".into()),
                    log_path.display()
                ))
                .with_data(serde_json::json!({"log": log_path, "tail": log_tail(log_path, 20)})));
            }
            if started.elapsed() > opts.boot_timeout {
                return Err(JidrError::boot_timeout(format!(
                    "{} did not boot within {} s ({stage}); see {}",
                    spec.avd,
                    opts.boot_timeout.as_secs(),
                    log_path.display()
                ))
                .with_data(serde_json::json!({"log": log_path})));
            }
            Ok(())
        };

        // 1. Discovery file → gRPC token.
        let disc_deadline = Instant::now() + DISCOVERY_TIMEOUT;
        let (token, ports) = loop {
            check("starting")?;
            if let Some(d) = crate::discovery::find(pid, ports.grpc)
                && let Some(t) = d.grpc_token()
            {
                // Trust the ports the emulator actually bound: if another
                // process took our console port first, the emulator falls
                // back to a different one.
                let actual = d.ports().unwrap_or(ports);
                if actual != ports {
                    tracing::warn!(
                        ?ports,
                        ?actual,
                        "emulator bound different ports than requested"
                    );
                    // (If the boot then fails, the actual pair stays reserved
                    // until restart; allocation skips it, nothing breaks.)
                    self.ports().release(ports);
                    self.ports().claim(actual);
                }
                break (t.to_string(), actual);
            }
            if Instant::now() > disc_deadline {
                return Err(JidrError::sdk(format!(
                    "emulator wrote no discovery file within {} s; see {}",
                    DISCOVERY_TIMEOUT.as_secs(),
                    log_path.display()
                )));
            }
            sleep(Duration::from_millis(250)).await;
        };
        let (mut controller, _) = loop {
            check("connecting gRPC")?;
            if let Ok(c) = grpc::connect_port(ports.grpc, &token).await {
                break c;
            }
            sleep(Duration::from_millis(500)).await;
        };
        progress(BootStage::GrpcReady);

        // 2. adb.
        let serial = ports.adb_serial();
        let adb = Adb::new(&spec.adb)
            .device(&serial)
            .with_timeout(Duration::from_secs(30));
        loop {
            check("waiting for adb")?;
            if !ports.auto_discovered() {
                let _ = Adb::new(&spec.adb).connect(&serial).await;
            }
            if adb.wait_for_device(Duration::from_secs(2)).await.is_ok() {
                break;
            }
        }
        progress(BootStage::AdbOnline);

        // 3. Boot completed: the emulator's own flag and the guest property.
        loop {
            check("booting")?;
            let booted = match controller.get_status(()).await {
                Ok(s) => s.get_ref().booted,
                // A wrong token never fixes itself: fail now, not at the timeout.
                Err(s)
                    if s.code() == tonic::Code::Unauthenticated
                        || s.code() == tonic::Code::PermissionDenied =>
                {
                    return Err(grpc::status_error("emulator rejected the gRPC token", s));
                }
                Err(_) => false,
            };
            if booted && adb.getprop("sys.boot_completed").await.ok().as_deref() == Some("1") {
                break;
            }
            sleep(Duration::from_secs(1)).await;
        }
        progress(BootStage::Booted);
        tracing::info!(avd = %spec.avd, pid, secs = started.elapsed().as_secs_f32(), "booted");

        Ok(InstanceInfo {
            avd: spec.avd.clone(),
            pid,
            console_port: ports.console,
            adb_port: ports.adb,
            grpc_port: ports.grpc,
            adb_serial: serial,
            grpc_token: token,
            started_at: Utc::now(),
            log_path: Some(log_path.to_path_buf()),
        })
    }

    async fn wait_exit(&self, instance: &InstanceInfo, within: Duration) -> bool {
        if let Some(mut rx) = self.exit_watch(instance.pid) {
            return timeout(within, rx.wait_for(Option::is_some)).await.is_ok();
        }
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if !process_alive(instance.pid).await {
                return true;
            }
            sleep(Duration::from_millis(250)).await;
        }
        false
    }
}

#[async_trait]
impl Backend for AospBackend {
    async fn start(
        &self,
        spec: &LaunchSpec,
        opts: &LaunchOptions,
        progress: BootProgress<'_>,
        cancel: &Cancel,
    ) -> JidrResult<InstanceInfo> {
        // Refuse an AVD that is already running: started by Android Studio,
        // another Jidr, or a launch still in progress here.
        if jidr_sdk::AvdManager::new(&spec.avd_home).is_running(&spec.avd) {
            return Err(JidrError::busy(format!(
                "{} is already running in another emulator (Android Studio or another Jidr)",
                spec.avd
            )));
        }
        let key = spec.avd_home.join(&spec.avd);
        if !self
            .launching
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone())
        {
            return Err(JidrError::busy(format!("{} is already starting", spec.avd)));
        }
        let result = match self.ports.allocate() {
            Ok(ports) => {
                let r = self.launch(spec, opts, ports, progress, cancel).await;
                if r.is_err() {
                    self.ports.release(ports);
                }
                r
            }
            Err(e) => Err(e),
        };
        self.launching
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
        result
    }

    async fn stop(&self, instance: &InstanceInfo) -> JidrResult<()> {
        let graceful = match grpc::connect(instance).await {
            Ok((mut c, _)) => c
                .set_vm_state(VmRunState {
                    state: RunState::Shutdown as i32,
                })
                .await
                .is_ok(),
            Err(_) => false,
        };
        if !(graceful && self.wait_exit(instance, SHUTDOWN_TIMEOUT).await) {
            tracing::warn!(
                pid = instance.pid,
                graceful,
                "emulator did not shut down; killing"
            );
            kill(instance.pid).await;
            self.wait_exit(instance, Duration::from_secs(10)).await;
        }
        if !instance.ports().auto_discovered() {
            // The adb binary isn't known here; the daemon disconnects TCP serials.
            tracing::debug!(serial = %instance.adb_serial, "tcp adb serial left for the daemon to disconnect");
        }
        self.exits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&instance.pid);
        self.ports.release(instance.ports());
        Ok(())
    }

    async fn is_alive(&self, instance: &InstanceInfo) -> bool {
        match self.exit_watch(instance.pid) {
            Some(rx) => rx.borrow().is_none(),
            None => process_alive(instance.pid).await,
        }
    }
}

pub async fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn kill(pid: u32) {
    let _ = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .await;
}

/// The last `n` lines of a log file, for error reports.
pub fn log_tail(path: &Path, n: usize) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|t| {
            let lines: Vec<&str> = t.lines().collect();
            lines[lines.len().saturating_sub(n)..]
                .iter()
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec() -> LaunchSpec {
        LaunchSpec {
            avd: "dev1".into(),
            avd_home: PathBuf::from("/avd"),
            sdk_root: PathBuf::from("/sdk"),
            emulator: PathBuf::from("/sdk/emulator/emulator"),
            adb: PathBuf::from("/sdk/platform-tools/adb"),
            log_dir: PathBuf::from("/logs"),
        }
    }

    #[test]
    fn args_cover_every_option() {
        let ports = Ports {
            console: 5556,
            adb: 5557,
            grpc: 9000,
        };
        let base = AospBackend::args(&spec(), &LaunchOptions::default(), ports).join(" ");
        assert!(base.starts_with(
            "-avd dev1 -no-window -no-boot-anim -grpc-use-token -grpc 9000 -port 5556 -gpu "
        ));
        assert!(!base.contains("-no-snapshot"), "D29: quick boot by default");
        assert!(!base.contains("-no-audio"));

        let opts = LaunchOptions {
            extra_args: vec!["-feature".into(), "-SystemAsRoot".into()],
            ramdisk: Some(PathBuf::from("/avd/dev1.avd/jidr/ramdisk.magisk-27.img")),
            cold_boot: true,
            wipe: true,
            gpu: Some("swiftshader_indirect".into()),
            audio: false,
            snapshot_save: false,
            boot_timeout: Duration::from_secs(1),
        };
        let all = AospBackend::args(&spec(), &opts, ports).join(" ");
        for want in [
            "-gpu swiftshader_indirect",
            "-no-snapshot-load",
            "-no-snapshot-save",
            "-wipe-data",
            "-no-audio",
            "-ramdisk /avd/dev1.avd/jidr/ramdisk.magisk-27.img",
        ] {
            assert!(all.contains(want), "missing {want}: {all}");
        }
        assert!(all.ends_with("-feature -SystemAsRoot"), "extra args last");
    }

    #[test]
    fn log_tail_returns_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("l.log");
        std::fs::write(&p, "a\nb\nc\nd\n").unwrap();
        assert_eq!(log_tail(&p, 2), vec!["c", "d"]);
        assert!(log_tail(&dir.path().join("missing"), 2).is_empty());
    }

    /// A fake `emulator` that exits immediately: start fails fast with the
    /// log path, and the ports go back to the pool.
    #[tokio::test]
    async fn early_exit_is_reported_with_log() {
        let dir = tempfile::tempdir().unwrap();
        let emu = dir.path().join("emulator");
        std::fs::write(&emu, "#!/bin/sh\necho 'FATAL: no such AVD' >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&emu, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let s = LaunchSpec {
            emulator: emu,
            log_dir: dir.path().join("logs"),
            ..spec()
        };
        let backend = AospBackend::new();
        let stages = Mutex::new(Vec::new());
        let err = backend
            .start(
                &s,
                &LaunchOptions::default(),
                &|st| stages.lock().unwrap().push(st),
                &Cancel::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Sdk, "{err}");
        assert!(err.message.contains("exited"), "{err}");
        let tail = err.data.unwrap()["tail"].clone();
        assert!(tail.to_string().contains("no such AVD"), "{tail}");
        assert_eq!(*stages.lock().unwrap(), vec![BootStage::Spawned]);
        // Ports were released: the same pair is handed out again.
        let again = backend.ports().allocate().unwrap();
        assert_eq!(again.console % 2, 0);
    }

    #[tokio::test]
    async fn cancel_stops_a_boot() {
        let dir = tempfile::tempdir().unwrap();
        let emu = dir.path().join("emulator");
        std::fs::write(&emu, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&emu, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let s = LaunchSpec {
            emulator: emu,
            log_dir: dir.path().join("logs"),
            ..spec()
        };
        let backend = AospBackend::new();
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(300)).await;
            c2.cancel();
        });
        let err = backend
            .start(&s, &LaunchOptions::default(), &|_| {}, &cancel)
            .await
            .unwrap_err();
        assert!(jidr_core::cancel::is_cancelled(&err), "{err}");
    }
}
