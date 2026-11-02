//! An in-memory backend for daemon, client and CLI tests: no emulator, but
//! the same lifecycle, with hooks to hold a start, trigger a crash and see
//! what was launched.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use jidr_core::{Cancel, JidrError, JidrResult};
use tokio::sync::{Notify, Semaphore};

use crate::crash::{CrashKind, CrashReport};
use crate::{Backend, BootProgress, BootStage, InstanceInfo, LaunchOptions, LaunchSpec};

#[derive(Debug, Default)]
struct FakeState {
    next: u16,
    running: HashMap<u32, String>,
    crashes: HashMap<u32, Arc<Notify>>,
    launches: Vec<(String, LaunchOptions)>,
    specs: Vec<LaunchSpec>,
    fail_next: Option<JidrError>,
    footprints: HashMap<String, u64>,
    /// Snapshots saved: (AVD, name).
    saves: Vec<(String, String)>,
    /// `save_snapshot` writes an empty `ram.bin` (a failed save).
    empty_saves: bool,
    /// Instances stopped without the exit save.
    unsaved_stops: Vec<String>,
    clients: HashMap<String, bool>,
}

#[derive(Debug, Default)]
pub struct FakeBackend {
    state: Mutex<FakeState>,
    gate: Mutex<Option<Arc<Semaphore>>>,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// From now on each start waits for one permit: `gate.add_permits(1)`.
    pub fn gate_starts(&self) -> Arc<Semaphore> {
        let sem = Arc::new(Semaphore::new(0));
        *self.gate.lock().unwrap_or_else(|e| e.into_inner()) = Some(sem.clone());
        sem
    }

    /// The next start fails with `err`.
    pub fn fail_next_start(&self, err: JidrError) {
        self.lock().fail_next = Some(err);
    }

    /// Simulates the emulator `pid` dying.
    pub fn crash(&self, pid: u32) {
        let mut s = self.lock();
        s.running.remove(&pid);
        if let Some(n) = s.crashes.get(&pid) {
            n.notify_one();
        }
    }

    /// Every start so far: AVD and options.
    pub fn launches(&self) -> Vec<(String, LaunchOptions)> {
        self.lock().launches.clone()
    }

    /// Every start so far: what the daemon asked to launch.
    pub fn specs(&self) -> Vec<LaunchSpec> {
        self.lock().specs.clone()
    }

    pub fn running(&self) -> Vec<u32> {
        self.lock().running.keys().copied().collect()
    }

    /// From now on `save_snapshot` leaves an empty `ram.bin`.
    pub fn fail_saves(&self) {
        self.lock().empty_saves = true;
    }

    /// Every `save_snapshot` so far: (AVD, snapshot name).
    pub fn saves(&self) -> Vec<(String, String)> {
        self.lock().saves.clone()
    }

    /// AVDs stopped with `stop_without_save`.
    pub fn unsaved_stops(&self) -> Vec<String> {
        self.lock().unsaved_stops.clone()
    }

    /// Pretends a player is attached to AVD `avd` (idle detection).
    pub fn set_clients(&self, avd: &str, attached: bool) {
        self.lock().clients.insert(avd.to_string(), attached);
    }

    /// What [`Backend::footprint_mb`] reports for AVD `avd` while it runs.
    pub fn set_footprint(&self, avd: &str, mb: u64) {
        self.lock().footprints.insert(avd.to_string(), mb);
    }
}

#[async_trait]
impl Backend for FakeBackend {
    async fn start(
        &self,
        spec: &LaunchSpec,
        opts: &LaunchOptions,
        progress: BootProgress<'_>,
        cancel: &Cancel,
    ) -> JidrResult<InstanceInfo> {
        {
            let mut s = self.lock();
            if s.running.values().any(|a| a == &spec.avd) {
                return Err(JidrError::busy(format!("{} is already running", spec.avd)));
            }
            s.launches.push((spec.avd.clone(), opts.clone()));
            s.specs.push(spec.clone());
            if let Some(err) = s.fail_next.take() {
                return Err(err);
            }
        }
        progress(BootStage::Spawned);
        let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(gate) = gate {
            loop {
                cancel.check()?;
                if let Ok(Ok(permit)) =
                    tokio::time::timeout(Duration::from_millis(50), gate.acquire()).await
                {
                    permit.forget();
                    break;
                }
            }
        }
        cancel.check()?;
        for stage in [
            BootStage::GrpcReady,
            BootStage::AdbOnline,
            BootStage::Booted,
        ] {
            progress(stage);
        }
        let mut s = self.lock();
        s.next += 1;
        let pid = 4_000_000 + u32::from(s.next);
        let console = 5554 + 2 * s.next;
        s.running.insert(pid, spec.avd.clone());
        s.crashes.insert(pid, Arc::new(Notify::new()));
        Ok(InstanceInfo {
            avd: spec.avd.clone(),
            pid,
            console_port: console,
            adb_port: console + 1,
            grpc_port: 20_000 + s.next,
            adb_serial: format!("emulator-{console}"),
            grpc_token: format!("fake-token-{pid}"),
            started_at: Utc::now(),
            log_path: None,
        })
    }

    async fn stop(&self, instance: &InstanceInfo) -> JidrResult<()> {
        self.lock().running.remove(&instance.pid);
        Ok(())
    }

    async fn is_alive(&self, instance: &InstanceInfo) -> bool {
        self.lock().running.contains_key(&instance.pid)
    }

    async fn adopt(&self, _avd_home: &Path) -> Vec<InstanceInfo> {
        Vec::new()
    }

    async fn save_snapshot(
        &self,
        instance: &InstanceInfo,
        name: &str,
        avd_dir: &Path,
    ) -> JidrResult<()> {
        let empty = {
            let mut s = self.lock();
            if !s.running.contains_key(&instance.pid) {
                return Err(JidrError::not_found("not running"));
            }
            s.saves.push((instance.avd.clone(), name.to_string()));
            s.empty_saves
        };
        let dir = avd_dir.join("snapshots").join(name);
        let ram: &[u8] = if empty { b"" } else { b"fake ram" };
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join("snapshot.pb"), b"pb"))
            .and_then(|()| std::fs::write(dir.join("ram.bin"), ram))
            .map_err(|e| JidrError::unexpected(format!("fake snapshot: {e}")))
    }

    async fn stop_without_save(&self, instance: &InstanceInfo) -> JidrResult<()> {
        let mut s = self.lock();
        s.running.remove(&instance.pid);
        s.unsaved_stops.push(instance.avd.clone());
        Ok(())
    }

    async fn has_clients(&self, instance: &InstanceInfo) -> bool {
        self.lock()
            .clients
            .get(&instance.avd)
            .copied()
            .unwrap_or(false)
    }

    async fn footprint_mb(&self, instance: &InstanceInfo) -> Option<u64> {
        let s = self.lock();
        s.running
            .contains_key(&instance.pid)
            .then(|| s.footprints.get(&instance.avd).copied())
            .flatten()
    }

    async fn wait_for_crash(&self, instance: &InstanceInfo, _log_dir: &Path) -> CrashReport {
        let notify = self.lock().crashes.get(&instance.pid).cloned();
        match notify {
            Some(n) => n.notified().await,
            None => std::future::pending::<()>().await,
        }
        CrashReport {
            avd: instance.avd.clone(),
            pid: instance.pid,
            kind: CrashKind::Exited,
            reason: "fake crash".into(),
            exit_code: Some(139),
            at: Utc::now(),
            log: None,
        }
    }
}
