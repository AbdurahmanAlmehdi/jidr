//! The GUI's only link to the daemon: a `jidr-client` connection on its own
//! tokio runtime, with events, a health check and reconnect. The UI thread
//! never blocks on it; results come back through callbacks.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jidr_client::{Client, SpawnOptions};
use jidr_core::{Device, HardwareProfile, JidrError, JidrPaths, JidrResult, Job};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::i18n::tr;
use crate::model::{DeviceInfo, HostBudget, Lease};

/// How often to check that the daemon is still there.
const HEALTH_EVERY: Duration = Duration::from_secs(3);
/// Wait between reconnect attempts.
const RETRY_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conn {
    Connecting,
    Online,
    Reconnecting(String),
}

/// Daemon data the launcher shows, gathered after each change.
#[derive(Debug, Clone, Default)]
pub struct Data {
    /// People's devices, then pool clones, then bases (the sidebar's groups).
    pub devices: Vec<DeviceInfo>,
    pub jobs: Vec<Job>,
    pub sdk_ready: bool,
    /// `host.budget`; `None` from an older daemon.
    pub budget: Option<HostBudget>,
}

pub enum Update {
    Conn(Conn),
    Data(Data),
    /// A raw event, for views that follow one device (player, logcat).
    Event(jidr_core::rpc::Event),
}

type Sink = Arc<dyn Fn(Update) + Send + Sync>;

#[derive(Default)]
struct Cache {
    details: HashMap<Uuid, Value>,
    profiles: BTreeMap<String, HardwareProfile>,
    boot: HashMap<Uuid, f32>,
}

pub struct Bridge {
    rt: tokio::runtime::Runtime,
    client: Arc<tokio::sync::RwLock<Option<Arc<Client>>>>,
    /// Jobs this window started (others holding a device mean Busy).
    mine: Arc<Mutex<HashSet<Uuid>>>,
}

impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge").finish_non_exhaustive()
    }
}

impl Bridge {
    pub fn new() -> JidrResult<Arc<Self>> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("jidr-ui-io")
            .build()
            .map_err(|e| JidrError::unexpected(format!("tokio runtime: {e}")))?;
        Ok(Arc::new(Self {
            rt,
            client: Arc::default(),
            mine: Arc::default(),
        }))
    }

    pub fn handle(&self) -> tokio::runtime::Handle {
        self.rt.handle().clone()
    }

    /// Connects (starting the daemon if needed, D4) and keeps reconnecting
    /// for the life of the app.
    pub fn connect(
        self: &Arc<Self>,
        paths: JidrPaths,
        sink: impl Fn(Update) + Send + Sync + 'static,
    ) {
        let opts = SpawnOptions {
            restart_stale: true,
            ..Default::default()
        };
        self.connect_with(paths, opts, sink);
    }

    /// `connect` with explicit spawn options (tests point `program` at a
    /// binary that doesn't exist so nothing is spawned).
    pub fn connect_with(
        self: &Arc<Self>,
        paths: JidrPaths,
        opts: SpawnOptions,
        sink: impl Fn(Update) + Send + Sync + 'static,
    ) {
        let sink: Sink = Arc::new(sink);
        let me = self.clone();
        self.rt.spawn(async move {
            let mut cache = Cache::default();
            let mut first = true;
            loop {
                sink(Update::Conn(if first {
                    Conn::Connecting
                } else {
                    Conn::Reconnecting(tr!("Reconnecting to the daemon…"))
                }));
                first = false;
                match Client::connect_or_spawn(&paths, "gui", &opts).await {
                    Ok(c) => {
                        let c = Arc::new(c);
                        *me.client.write().await = Some(c.clone());
                        sink(Update::Conn(Conn::Online));
                        if let Err(e) = me.session(&c, &sink, &mut cache).await {
                            tracing::warn!("daemon session ended: {e}");
                        }
                        *me.client.write().await = None;
                    }
                    Err(e) => tracing::warn!("cannot reach the daemon: {e}"),
                }
                tokio::time::sleep(RETRY_EVERY).await;
            }
        });
    }

    /// One connected session: subscribe, load, then follow events until the
    /// health check fails.
    async fn session(&self, c: &Arc<Client>, sink: &Sink, cache: &mut Cache) -> JidrResult<()> {
        let mut events = c.subscribe(&["device.", "job.", "pool."]).await?;
        cache.details.clear();
        if let Ok(p) = c
            .call_as::<_, Vec<HardwareProfile>>("profile.list", json!({}))
            .await
        {
            cache.profiles = p.into_iter().map(|p| (p.id.clone(), p)).collect();
        }
        sink(Update::Data(self.load(c, cache).await?));
        let mut health = tokio::time::interval(HEALTH_EVERY);
        loop {
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(ev) => {
                        let reload = self.note(&ev, cache);
                        sink(Update::Event(ev));
                        if reload {
                            // Coalesce bursts (boot progress, job progress).
                            tokio::time::sleep(Duration::from_millis(80)).await;
                            while let Ok(ev) = events.try_recv() {
                                self.note(&ev, cache);
                                sink(Update::Event(ev));
                            }
                            sink(Update::Data(self.load(c, cache).await?));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        sink(Update::Data(self.load(c, cache).await?));
                    }
                    Err(e) => return Err(JidrError::unexpected(format!("events: {e}"))),
                },
                _ = health.tick() => {
                    c.call("daemon.status", json!({})).await?;
                }
            }
        }
    }

    /// Updates the cache from an event; true when the launcher should reload.
    fn note(&self, ev: &jidr_core::rpc::Event, cache: &mut Cache) -> bool {
        let device = ev
            .data
            .get("device")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok());
        match ev.topic.as_str() {
            "device.boot" => {
                if let (Some(d), Some(p)) =
                    (device, ev.data.get("progress").and_then(Value::as_f64))
                {
                    cache.boot.insert(d, p as f32);
                }
                true
            }
            "device.created" | "device.updated" | "device.deleted" => {
                if let Some(d) = device {
                    cache.details.remove(&d);
                }
                true
            }
            t => t.starts_with("device.") || t.starts_with("job.") || t.starts_with("pool."),
        }
    }

    async fn load(&self, c: &Client, cache: &mut Cache) -> JidrResult<Data> {
        let devices: Vec<Device> = c.call_as("device.list", json!({})).await?;
        let jobs: Vec<Job> = c.call_as("jobs.list", json!({})).await.unwrap_or_default();
        let sdk_ready = c
            .call("sdk.status", json!({}))
            .await
            .map(|v| sdk_has_tools(&v))
            .unwrap_or(false);
        let budget: Option<HostBudget> = c.call_as("host.budget", json!({})).await.ok();
        let leases = c
            .call("pool.list", json!({}))
            .await
            .map(|v| pool_leases(&v))
            .unwrap_or_default();
        let base_leases = c
            .call("base.list", json!({}))
            .await
            .map(|v| base_leases(&v))
            .unwrap_or_default();
        let names: HashMap<Uuid, String> = devices.iter().map(|d| (d.id, d.name.clone())).collect();
        let mine = self.mine.lock().map(|m| m.clone()).unwrap_or_default();
        let mut infos = Vec::with_capacity(devices.len());
        for d in devices {
            if !cache.details.contains_key(&d.id)
                && let Ok(v) = c.call("device.details", json!({ "device": d.id })).await
            {
                cache.details.insert(d.id, v);
            }
            let mut info = DeviceInfo::new(d.clone());
            if let Some(det) = cache.details.get(&d.id) {
                apply_details(&mut info, det);
            }
            if let Some(p) = cache.profiles.get(&d.profile) {
                info.profile_name = Some(p.name.clone());
                if info.width == 0 {
                    (info.width, info.height, info.density) = (p.width, p.height, p.density);
                }
                if info.ram_mb == 0 {
                    (info.ram_mb, info.cpus) = (p.ram_mb, p.cpus);
                }
            }
            if matches!(
                d.status,
                jidr_core::DeviceStatus::Booting | jidr_core::DeviceStatus::Starting
            ) {
                info.boot_progress = cache.boot.get(&d.id).copied();
            } else {
                cache.boot.remove(&d.id);
            }
            let holding: Vec<&Job> = jobs
                .iter()
                .filter(|j| j.device == Some(d.id) && !j.state.is_terminal())
                .collect();
            info.root_job = holding.iter().any(|j| j.kind.starts_with("root."));
            if holding
                .iter()
                .any(|j| !mine.contains(&j.id) && j.kind != "device.start")
            {
                info.busy_holder = Some("another client".into());
            }
            info.lease = leases.get(&d.id).cloned();
            info.base_name = d.pool_base.and_then(|b| names.get(&b).cloned());
            info.base_leases = base_leases.get(&d.id).copied().unwrap_or(0);
            infos.push(info);
        }
        infos.sort_by_key(crate::model::group_rank);
        Ok(Data {
            devices: infos,
            jobs,
            sdk_ready,
            budget,
        })
    }

    /// Runs a call off the UI thread; `done` gets the result (on the IO thread).
    pub fn call(
        &self,
        method: &str,
        params: Value,
        done: impl FnOnce(JidrResult<Value>) + Send + 'static,
    ) {
        let client = self.client.clone();
        let method = method.to_string();
        self.rt.spawn(async move {
            let c = client.read().await.clone();
            let r = match c {
                Some(c) => c.call(&method, params).await,
                None => Err(JidrError::unexpected("not connected to the daemon")),
            };
            done(r);
        });
    }

    /// Calls a long-running command and follows its job to the end.
    pub fn run(
        &self,
        method: &str,
        params: Value,
        progress: impl Fn(&Job) + Send + Sync + 'static,
        done: impl FnOnce(JidrResult<Value>) + Send + 'static,
    ) {
        let client = self.client.clone();
        let mine = self.mine.clone();
        let method = method.to_string();
        self.rt.spawn(async move {
            let Some(c) = client.read().await.clone() else {
                done(Err(JidrError::unexpected("not connected to the daemon")));
                return;
            };
            let first = match c.call(&method, params).await {
                Ok(v) => v,
                Err(e) => return done(Err(e)),
            };
            let Some(job) = jidr_client::pending_job(&first) else {
                return done(Ok(first));
            };
            if let Ok(mut m) = mine.lock() {
                m.insert(job);
            }
            let r = loop {
                match c.wait_job(job, Some(Duration::from_secs(1))).await {
                    Ok(j) => {
                        progress(&j);
                        match j.state {
                            jidr_core::JobState::Succeeded => {
                                break Ok(j.result.unwrap_or(Value::Null));
                            }
                            jidr_core::JobState::Failed => {
                                break Err(j.error.unwrap_or_else(|| {
                                    JidrError::unexpected(format!("{method} failed"))
                                }));
                            }
                            jidr_core::JobState::Cancelled => {
                                break Err(jidr_core::cancel::cancelled());
                            }
                            _ => {}
                        }
                    }
                    Err(e) => break Err(e),
                }
            };
            if let Ok(mut m) = mine.lock() {
                m.remove(&job);
            }
            done(r);
        });
    }

    /// The current client, for views that stream (logcat).
    pub fn client(&self) -> Option<Arc<Client>> {
        self.rt.block_on(async { self.client.read().await.clone() })
    }
}

/// `sdk.status` names the SDK whose emulator it would use; without one
/// (and without an image) Jidr still needs setting up (first run).
pub fn sdk_has_tools(v: &Value) -> bool {
    v.get("tools_sdk").is_some_and(|e| !e.is_null())
        && v.get("installed_images")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
}

/// Leases by pool device, from `pool.list` rows (`{lease: {device, holder,
/// expires_at}, …}`).
pub fn pool_leases(v: &Value) -> HashMap<Uuid, Lease> {
    let rows = v.as_array().map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .filter_map(|r| {
            let l = r.get("lease")?;
            let device = Uuid::parse_str(l.get("device")?.as_str()?).ok()?;
            let ttl = l.pointer("/holder/kind").and_then(Value::as_str) == Some("ttl");
            let expires_at = l
                .get("expires_at")
                .and_then(Value::as_str)
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&chrono::Utc));
            Some((device, Lease { ttl, expires_at }))
        })
        .collect()
}

/// Leased clones per base, from `base.list` rows (`{id, leases, …}`).
pub fn base_leases(v: &Value) -> HashMap<Uuid, usize> {
    let rows = v.as_array().map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .filter_map(|r| {
            let id = Uuid::parse_str(r.get("id")?.as_str()?).ok()?;
            let n = r.get("leases").and_then(Value::as_u64).unwrap_or(0);
            Some((id, usize::try_from(n).unwrap_or(0)))
        })
        .collect()
}

/// Screen and hardware from `device.details` (config.ini) and disk usage.
fn apply_details(info: &mut DeviceInfo, det: &Value) {
    let cfg = det.get("config");
    let num = |k: &str| -> u32 {
        cfg.and_then(|c| c.get(k))
            .and_then(Value::as_str)
            .and_then(|s| s.trim_end_matches(['M', 'B']).parse().ok())
            .unwrap_or(0)
    };
    info.width = num("hw.lcd.width");
    info.height = num("hw.lcd.height");
    info.density = num("hw.lcd.density");
    info.ram_mb = num("hw.ramSize");
    info.cpus = num("hw.cpu.ncore");
    info.disk_bytes = det.get("disk_usage").and_then(Value::as_u64);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_fill_screen_and_hardware() {
        let det = json!({
            "config": {"hw.lcd.width": "1080", "hw.lcd.height": "2400", "hw.lcd.density": "420",
                       "hw.ramSize": "4096M", "hw.cpu.ncore": "4"},
            "disk_usage": 6_800_000_000u64
        });
        let d: Device = serde_json::from_value(json!({
            "id": Uuid::nil(), "name": "p", "avd": "p", "profile": "phone-6.1-fhd",
            "image": {"api": 35, "tag": "google_apis", "abi": "arm64-v8a"},
            "sdk_root": "/sdk", "created_at": "2026-10-13T00:00:00Z"
        }))
        .unwrap();
        let mut info = DeviceInfo::new(d);
        apply_details(&mut info, &det);
        assert_eq!((info.width, info.height, info.density), (1080, 2400, 420));
        assert_eq!((info.ram_mb, info.cpus), (4096, 4));
        assert_eq!(info.disk_bytes, Some(6_800_000_000));
    }

    #[test]
    fn pool_and_base_lists_parse() {
        let dev = Uuid::from_u128(7);
        let base = Uuid::from_u128(8);
        let leases = pool_leases(&json!([
            {"lease": {"id": Uuid::from_u128(9), "base": base, "device": dev,
                       "holder": {"kind": "ttl"}, "expires_at": "2026-10-13T15:44:00Z",
                       "created_at": "2026-10-13T13:44:00Z"},
             "device": "Shop base 1", "base": "Shop base", "status": {"state": "running"}},
            {"lease": {"id": Uuid::from_u128(10), "base": base, "device": Uuid::from_u128(11),
                       "holder": {"kind": "connection", "id": 3}, "expires_at": null,
                       "created_at": "2026-10-13T14:00:00Z"}}
        ]));
        assert!(leases[&dev].ttl);
        assert!(leases[&dev].expires_at.is_some());
        assert!(!leases[&Uuid::from_u128(11)].ttl);
        assert!(leases[&Uuid::from_u128(11)].expires_at.is_none());
        let bases = base_leases(&json!([{"id": base, "name": "Shop base", "leases": 2}]));
        assert_eq!(bases[&base], 2);
        assert!(pool_leases(&json!({"not": "a list"})).is_empty());
    }
}
