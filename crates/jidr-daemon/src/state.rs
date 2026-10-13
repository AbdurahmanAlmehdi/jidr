//! Daemon state: devices, running instances, crash watchers. Implements the
//! `DaemonContext` providers use.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use async_trait::async_trait;
use jidr_backend::{Backend, BootStage, InstanceInfo, LaunchOptions, LaunchSpec};
use jidr_core::provider::{CallCtx, CommandProvider, DaemonContext, DeviceUpdate};
use jidr_core::{Cancel, ConnectInfo, Device, DeviceStatus, JidrError, JidrPaths, JidrResult};
use jidr_sdk::{AvdManager, Sdk, SdkSet};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::locks::DeviceLocks;
use crate::store::DeviceStore;

/// An event published to `events.subscribe` listeners.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub topic: String,
    pub data: Value,
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub paths: JidrPaths,
    /// Android Studio's SDK (read-only). `None` = auto-detect.
    pub studio_sdk: Option<PathBuf>,
}

pub struct State {
    pub(crate) paths: JidrPaths,
    studio_sdk: Option<PathBuf>,
    pub(crate) backend: Arc<dyn Backend>,
    store: Mutex<DeviceStore>,
    instances: Mutex<HashMap<Uuid, InstanceInfo>>,
    watchers: Mutex<HashMap<Uuid, JoinHandle<()>>>,
    pub(crate) locks: DeviceLocks,
    providers: OnceLock<Vec<Arc<dyn CommandProvider>>>,
    events: broadcast::Sender<Event>,
    me: Weak<State>,
    pub(crate) started_at: std::time::Instant,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl State {
    pub fn new(config: DaemonConfig, backend: Arc<dyn Backend>) -> JidrResult<Arc<Self>> {
        let store = DeviceStore::load(config.paths.devices_file())?;
        let (events, _) = broadcast::channel(1024);
        Ok(Arc::new_cyclic(|me| Self {
            paths: config.paths,
            studio_sdk: config.studio_sdk,
            backend,
            store: Mutex::new(store),
            instances: Mutex::new(HashMap::new()),
            watchers: Mutex::new(HashMap::new()),
            locks: DeviceLocks::default(),
            providers: OnceLock::new(),
            events,
            me: me.clone(),
            started_at: std::time::Instant::now(),
        }))
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

    fn set_status(
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
        }
    }

    fn watch_crash(&self, id: Uuid, info: InstanceInfo) {
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
        let opts = LaunchOptions {
            ramdisk: device.launch.ramdisk.clone(),
            extra_args: device.launch.extra_args.clone(),
            gpu: device.launch.gpu.clone(),
            cold_boot: cold_boot || device.launch.force_cold_boot,
            ..LaunchOptions::default()
        };
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
        if device.launch.force_cold_boot {
            self.with_store(|s| s.update(id, |d| d.launch.force_cold_boot = false))?;
        }
        self.watch_crash(id, info);
        let device = self.with_store(|s| s.get(id).cloned())?;
        self.run_boot_hooks(&device).await;
        Ok(device)
    }

    async fn stop_device(&self, id: Uuid) -> JidrResult<Device> {
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
