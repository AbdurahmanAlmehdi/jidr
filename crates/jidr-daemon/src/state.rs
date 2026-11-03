//! Daemon state: devices, running instances, crash watchers. Implements the
//! `DaemonContext` providers use.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use async_trait::async_trait;
use jidr_backend::{Backend, BootStage, InstanceInfo, LaunchOptions, LaunchSpec};
use jidr_core::provider::{CallCtx, CommandProvider, DaemonContext, DeviceUpdate};
use jidr_core::{
    Cancel, ConnectInfo, Device, DeviceStatus, JidrError, JidrPaths, JidrResult, Settings,
};
use jidr_sdk::host::HostMemory;
use jidr_sdk::{AvdManager, Sdk, SdkSet};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::budget::{Budget, DeviceUse};
use crate::locks::DeviceLocks;
use crate::store::DeviceStore;
use jidr_core::rpc::Event;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub paths: JidrPaths,
    /// Android Studio's SDK (read-only). `None` = auto-detect.
    pub studio_sdk: Option<PathBuf>,
    /// Exit after this long idle (D4: 10 min). `None` = never.
    pub idle_timeout: Option<std::time::Duration>,
    /// SDK repository base URL (mirrors, tests). `None` = `$JIDR_SDK_REPO`
    /// or Google's.
    pub sdk_repo: Option<String>,
}

impl DaemonConfig {
    /// Product defaults: auto-detected Studio SDK, 10-minute idle exit (D4).
    pub fn new(paths: JidrPaths) -> Self {
        Self {
            paths,
            studio_sdk: None,
            idle_timeout: Some(crate::lifecycle::IDLE_TIMEOUT),
            sdk_repo: None,
        }
    }
}

pub struct State {
    pub(crate) paths: JidrPaths,
    studio_sdk: Option<PathBuf>,
    pub(crate) sdk_repo: Option<String>,
    pub(crate) backend: Arc<dyn Backend>,
    store: Mutex<DeviceStore>,
    settings: Mutex<Settings>,
    pub(crate) instances: Mutex<HashMap<Uuid, InstanceInfo>>,
    pub(crate) watchers: Mutex<HashMap<Uuid, JoinHandle<()>>>,
    pub(crate) locks: DeviceLocks,
    providers: OnceLock<Vec<Arc<dyn CommandProvider>>>,
    events: broadcast::Sender<Event>,
    me: Weak<State>,
    pub(crate) started_at: std::time::Instant,
    /// Host RAM override (tests); `None` probes the host.
    host_memory: Option<HostMemory>,
    /// Footprint samples of running devices (D42 cost model).
    footprints: Mutex<HashMap<Uuid, Footprint>>,
    /// Serialises budget admission so parallel starts can't both slip in.
    budget_gate: Mutex<()>,
    /// When each running device last saw activity (auto-hibernate).
    pub(crate) activity: Mutex<HashMap<Uuid, std::time::Instant>>,
    /// Pool leases (D42, Phase 13d).
    pub(crate) leases: Mutex<crate::leases::Leases>,
}

/// Latest and peak footprint of a device's current run, in MB.
#[derive(Debug, Clone, Copy, Default)]
struct Footprint {
    current: u64,
    peak: u64,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `settings.json`, or the defaults when it is missing or unreadable.
fn load_settings(path: &std::path::Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("{}: {e}; using the default settings", path.display());
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

fn save_settings(path: &std::path::Path, s: &Settings) -> JidrResult<()> {
    let text = serde_json::to_string_pretty(s).map_err(|e| JidrError::unexpected(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| JidrError::unexpected(format!("{}: {e}", dir.display())))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| JidrError::unexpected(format!("{}: {e}", path.display())))
}

impl State {
    pub fn new(
        config: DaemonConfig,
        backend: Arc<dyn Backend>,
        host_memory: Option<HostMemory>,
    ) -> JidrResult<Arc<Self>> {
        let store = DeviceStore::load(config.paths.devices_file())?;
        let settings = load_settings(&config.paths.settings_file());
        let (events, _) = broadcast::channel(1024);
        Ok(Arc::new_cyclic(|me| Self {
            paths: config.paths,
            studio_sdk: config.studio_sdk,
            sdk_repo: config.sdk_repo,
            backend,
            store: Mutex::new(store),
            settings: Mutex::new(settings),
            instances: Mutex::new(HashMap::new()),
            watchers: Mutex::new(HashMap::new()),
            locks: DeviceLocks::default(),
            providers: OnceLock::new(),
            events,
            me: me.clone(),
            started_at: std::time::Instant::now(),
            host_memory,
            footprints: Mutex::new(HashMap::new()),
            budget_gate: Mutex::new(()),
            activity: Mutex::new(HashMap::new()),
            leases: Mutex::new(crate::leases::Leases::default()),
        }))
        .inspect(|s| s.adopt_pool_clones())
    }

    pub fn host_memory(&self) -> JidrResult<HostMemory> {
        match self.host_memory {
            Some(m) => Ok(m),
            None => jidr_sdk::host::memory(),
        }
    }

    /// Guest RAM of a device: `hw.ramSize` in its `config.ini`, else its
    /// profile's.
    fn ram_mb(&self, d: &Device) -> u32 {
        self.avds()
            .config(&d.avd)
            .ok()
            .and_then(|c| c.get("hw.ramSize").and_then(|v| v.trim().parse().ok()))
            .or_else(|| jidr_sdk::profiles::find(&d.profile).ok().map(|p| p.ram_mb))
            .unwrap_or(4096)
    }

    pub fn expected_mb(&self, d: &Device) -> u64 {
        crate::budget::expected_mb(self.ram_mb(d), d.footprint_mb)
    }

    /// The budget with every active device except `except` counted.
    fn budget_except(&self, except: Option<Uuid>) -> JidrResult<Budget> {
        let mem = self.host_memory()?;
        let devices = self.with_store(|s| s.all().to_vec());
        let samples = lock(&self.footprints).clone();
        let used = devices
            .iter()
            .filter(|d| d.status.is_active() && Some(d.id) != except)
            .map(|d| DeviceUse {
                id: d.id,
                name: d.name.clone(),
                expected_mb: self.expected_mb(d),
                measured_mb: samples.get(&d.id).map(|f| f.current).filter(|m| *m > 0),
            })
            .collect();
        Ok(Budget::new(mem, self.settings().budget_percent, used))
    }

    /// `host.budget`.
    pub fn budget(&self) -> JidrResult<Budget> {
        self.budget_except(None)
    }

    /// Admits a start: past the budget it fails with `resource_exhausted`
    /// unless `force`. Marks the device `starting` under the gate, so it
    /// counts for the next admission at once.
    pub fn admit(&self, id: Uuid, force: bool) -> JidrResult<()> {
        let _gate = lock(&self.budget_gate);
        let d = self.with_store(|s| s.get(id).cloned())?;
        if d.status.is_active() {
            return Ok(());
        }
        if !force {
            let budget = self.budget_except(Some(id))?;
            let need = self.expected_mb(&d);
            if !budget.fits(need) {
                let mut e = budget.refusal(&d.name, need);
                if let Some(Value::Object(data)) = e.data.as_mut() {
                    // The longest-idle running devices first: hibernating
                    // one frees all of its RAM (D42).
                    data.insert("hibernate_candidates".into(), json!(self.idle_devices()));
                }
                return Err(e);
            }
        }
        self.set_status(id, DeviceStatus::Starting, None);
        Ok(())
    }

    /// [`admit`](Self::admit), then start. A start that fails before the
    /// emulator runs puts the device back to `stopped`.
    pub async fn start_admitted(
        &self,
        id: Uuid,
        cold_boot: bool,
        force: bool,
        cancel: &Cancel,
    ) -> JidrResult<Device> {
        let d = self.with_store(|s| s.get(id).cloned())?;
        if d.base {
            return Err(JidrError::usage(format!(
                "{} is a base (read-only); lease a clone of it with pool.get",
                d.name
            )));
        }
        self.admit(id, force)?;
        let r = self.start_device(id, cold_boot, cancel).await;
        if r.is_err() && self.instance(id).is_none() {
            let starting = self.with_store(|s| {
                s.get(id)
                    .is_ok_and(|d| matches!(d.status, DeviceStatus::Starting))
            });
            if starting {
                self.set_status(id, DeviceStatus::Stopped, Some(None));
            }
        }
        r
    }

    /// Samples every running device's footprint (the daemon does this every
    /// 30 s while serving).
    pub async fn sample_footprints(&self) {
        let running: Vec<(Uuid, InstanceInfo)> = lock(&self.instances)
            .iter()
            .map(|(id, i)| (*id, i.clone()))
            .collect();
        for (id, info) in running {
            let Some(mb) = self.backend.footprint_mb(&info).await else {
                continue;
            };
            let mut f = lock(&self.footprints);
            let e = f.entry(id).or_default();
            e.current = mb;
            e.peak = e.peak.max(mb);
        }
    }

    /// Ends a run's sampling: its peak becomes the device's `footprint_mb`.
    pub(crate) fn finish_footprint(&self, id: Uuid) {
        let Some(f) = lock(&self.footprints).remove(&id) else {
            return;
        };
        if f.peak > 0 {
            let peak = u32::try_from(f.peak).unwrap_or(u32::MAX);
            let _ = self.with_store(|s| s.update(id, |d| d.footprint_mb = Some(peak)));
        }
    }

    /// Daemon-wide settings (gap B4).
    pub fn settings(&self) -> Settings {
        *lock(&self.settings)
    }

    /// Changes the settings and saves them to `settings.json`.
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> JidrResult<Settings> {
        let mut current = lock(&self.settings);
        let mut next = *current;
        f(&mut next);
        save_settings(&self.paths.settings_file(), &next)?;
        *current = next;
        Ok(next)
    }

    pub(crate) fn set_providers(&self, providers: Vec<Arc<dyn CommandProvider>>) {
        let _ = self.providers.set(providers);
    }

    fn arc(&self) -> JidrResult<Arc<State>> {
        self.me
            .upgrade()
            .ok_or_else(|| JidrError::unexpected("daemon is shutting down"))
    }

    pub fn sdks(&self) -> SdkSet {
        SdkSet {
            private: Sdk::private(&self.paths),
            studio: match &self.studio_sdk {
                Some(p) => Some(Sdk::read_only(p)),
                None => Sdk::studio(),
            },
        }
    }

    pub fn avds(&self) -> AvdManager {
        AvdManager::new(self.paths.avd_home())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Runs `f` on the device store.
    pub fn with_store<T>(&self, f: impl FnOnce(&mut DeviceStore) -> T) -> T {
        f(&mut lock(&self.store))
    }

    pub fn instance(&self, id: Uuid) -> Option<InstanceInfo> {
        lock(&self.instances).get(&id).cloned()
    }

    pub fn running_count(&self) -> usize {
        lock(&self.instances).len()
    }

    pub(crate) fn set_status(
        &self,
        id: Uuid,
        status: DeviceStatus,
        runtime: Option<Option<jidr_core::DeviceRuntime>>,
    ) {
        let updated = self.with_store(|s| {
            s.set_runtime(id, |d| {
                d.status = status;
                if let Some(rt) = runtime {
                    d.runtime = rt;
                }
            })
        });
        if let Some(d) = updated {
            self.emit(
                "device.status",
                json!({ "device": d.id, "name": d.name, "status": d.status }),
            );
        }
    }

    /// Re-adopts emulators that kept running across a daemon restart.
    pub async fn adopt_running(&self) {
        for info in self.backend.adopt(&self.paths.avd_home()).await {
            let id = self.with_store(|s| s.all().iter().find(|d| d.avd == info.avd).map(|d| d.id));
            let Some(id) = id else { continue };
            lock(&self.instances).insert(id, info.clone());
            self.set_status(id, DeviceStatus::Running, Some(Some(info.runtime())));
            self.watch_crash(id, info);
            self.touch(id);
        }
    }

    pub(crate) fn watch_crash(&self, id: Uuid, info: InstanceInfo) {
        let Ok(me) = self.arc() else { return };
        let handle = tokio::spawn(async move {
            let log_dir = me.paths.device_logs(&info.avd);
            let report = me.backend.wait_for_crash(&info, &log_dir).await;
            let still_ours = lock(&me.instances)
                .get(&id)
                .is_some_and(|i| i.pid == info.pid);
            if !still_ours {
                return;
            }
            lock(&me.instances).remove(&id);
            lock(&me.watchers).remove(&id);
            me.finish_footprint(id);
            // Releases ports; the process is already gone.
            let _ = me.backend.stop(&info).await;
            me.set_status(
                id,
                DeviceStatus::Crashed {
                    reason: report.reason.clone(),
                    log: report.log.clone(),
                },
                Some(None),
            );
            me.emit("device.crashed", json!({ "device": id, "report": report }));
        });
        if let Some(old) = lock(&self.watchers).insert(id, handle) {
            old.abort();
        }
    }

    fn launch_spec(&self, d: &Device) -> JidrResult<(LaunchSpec, Sdk)> {
        let sdks = self.sdks();
        let image_sdk = sdks.find_image(&d.image).cloned().ok_or_else(|| {
            JidrError::sdk(format!(
                "system image {} is not installed; run `jidr image install {}:{}`",
                d.image.package(),
                d.image.api,
                d.image.tag
            ))
        })?;
        let tools = sdks.tools()?.clone();
        Ok((
            LaunchSpec {
                avd: d.avd.clone(),
                avd_home: self.paths.avd_home(),
                sdk_root: image_sdk.root.clone(),
                sysdir: image_sdk.image_dir(&d.image),
                emulator: tools.emulator(),
                adb: tools.adb(),
                log_dir: self.paths.device_logs(&d.avd),
            },
            image_sdk,
        ))
    }

    async fn run_boot_hooks(&self, device: &Device) {
        let Ok(me) = self.arc() else { return };
        let providers = self.providers.get().cloned().unwrap_or_default();
        let ctx = CallCtx::new(me);
        for p in providers {
            if let Err(e) = p.on_booted(&ctx, device).await {
                tracing::warn!(provider = p.name(), device = %device.name, "boot hook failed: {e}");
                self.emit(
                    "device.hook_failed",
                    json!({ "device": device.id, "provider": p.name(), "error": e }),
                );
            }
        }
    }
}

#[async_trait]
impl DaemonContext for State {
    fn paths(&self) -> &JidrPaths {
        &self.paths
    }

    fn adb_path(&self) -> JidrResult<PathBuf> {
        Ok(self.sdks().tools()?.adb())
    }

    async fn device(&self, selector: &str) -> JidrResult<Device> {
        self.with_store(|s| s.find(selector).cloned())
    }

    async fn devices(&self) -> Vec<Device> {
        self.with_store(|s| s.all().to_vec())
    }

    async fn connect_info(&self, device: Uuid) -> JidrResult<ConnectInfo> {
        let info = self
            .instance(device)
            .ok_or_else(|| JidrError::not_found("device is not running; start it first"))?;
        Ok(ConnectInfo {
            device,
            grpc_endpoint: format!("127.0.0.1:{}", info.grpc_port),
            grpc_port: info.grpc_port,
            grpc_token: info.grpc_token.clone(),
            adb_serial: info.adb_serial.clone(),
            adb_path: self.adb_path()?,
            console_port: info.console_port,
        })
    }

    async fn update_device(&self, device: Uuid, f: DeviceUpdate) -> JidrResult<Device> {
        let d = self.with_store(|s| s.update(device, f))?;
        self.emit("device.updated", json!({ "device": d.id, "name": d.name }));
        Ok(d)
    }

    async fn start_device(&self, id: Uuid, cold_boot: bool, cancel: &Cancel) -> JidrResult<Device> {
        let device = self.with_store(|s| s.get(id).cloned())?;
        if let Some(info) = self.instance(id) {
            if self.backend.is_alive(&info).await {
                return Ok(device);
            }
            lock(&self.instances).remove(&id);
        }
        let (spec, _) = self.launch_spec(&device)?;
        for cam in [&device.launch.camera_back, &device.launch.camera_front]
            .into_iter()
            .flatten()
        {
            if !jidr_backend::valid_camera(cam) {
                return Err(JidrError::usage(format!(
                    "invalid camera {cam:?}: use emulated, virtualscene, none or webcamN"
                )));
            }
        }
        let resume = device
            .launch
            .resume_snapshot
            .clone()
            .filter(|_| !cold_boot && !device.launch.force_cold_boot);
        let opts = LaunchOptions {
            ramdisk: device.launch.ramdisk.clone(),
            camera_back: device.launch.camera_back.clone(),
            camera_front: device.launch.camera_front.clone(),
            extra_args: device.launch.extra_args.clone(),
            gpu: device.launch.gpu.clone(),
            audio: device.launch.audio.unwrap_or(true),
            cold_boot: cold_boot || device.launch.force_cold_boot,
            snapshot: resume.clone(),
            // D29 / gap B4: save a quick-boot snapshot on stop unless off. A
            // resumed run doesn't: the exit save would go into the named
            // snapshot, not `default_boot`.
            snapshot_save: self.settings().quick_boot && resume.is_none(),
            ..LaunchOptions::default()
        };
        if opts.snapshot_save {
            // Without it the headless emulator skips the exit save (13c).
            self.avds().set_save_on_exit(&device.avd, true)?;
        }
        self.set_status(id, DeviceStatus::Starting, None);
        let progress = |stage: BootStage| {
            if stage == BootStage::Spawned {
                self.set_status(id, DeviceStatus::Booting, None);
            }
            self.emit(
                "device.boot",
                json!({ "device": id, "stage": stage, "progress": stage.progress() }),
            );
        };
        let info = match self.backend.start(&spec, &opts, &progress, cancel).await {
            Ok(info) => info,
            Err(e) => {
                self.set_status(id, DeviceStatus::Stopped, Some(None));
                self.emit("device.boot_failed", json!({ "device": id, "error": e }));
                return Err(e);
            }
        };
        lock(&self.instances).insert(id, info.clone());
        self.set_status(id, DeviceStatus::Running, Some(Some(info.runtime())));
        if device.launch.force_cold_boot || device.launch.resume_snapshot.is_some() {
            self.with_store(|s| {
                s.update(id, |d| {
                    d.launch.force_cold_boot = false;
                    d.launch.resume_snapshot = None;
                })
            })?;
        }
        self.watch_crash(id, info);
        self.touch(id);
        let device = self.with_store(|s| s.get(id).cloned())?;
        self.run_boot_hooks(&device).await;
        Ok(device)
    }

    async fn stop_device(&self, id: Uuid) -> JidrResult<Device> {
        let current = self.with_store(|s| s.get(id).cloned())?;
        if matches!(current.status, DeviceStatus::Hibernated) {
            // Already stopped, with its state saved for the next start.
            return Ok(current);
        }
        if let Some(w) = lock(&self.watchers).remove(&id) {
            w.abort();
        }
        let info = lock(&self.instances).remove(&id);
        if let Some(info) = info {
            self.set_status(id, DeviceStatus::Stopping, None);
            if let Err(e) = self.backend.stop(&info).await {
                // Put it back: we don't know its state.
                lock(&self.instances).insert(id, info.clone());
                self.watch_crash(id, info);
                return Err(e);
            }
        }
        self.finish_footprint(id);
        crate::state::lock(&self.activity).remove(&id);
        self.set_status(id, DeviceStatus::Stopped, Some(None));
        self.with_store(|s| s.get(id).cloned())
    }

    fn emit(&self, topic: &str, data: Value) {
        // No listeners is fine.
        let _ = self.events.send(Event {
            topic: topic.to_string(),
            data,
        });
    }
}
