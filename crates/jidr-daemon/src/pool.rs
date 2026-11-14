//! Bases and the pool (D42, Phase 13d): an agent gets a throwaway
//! copy-on-write clone of a snapshotted base with `pool.get`, uses it, and
//! gives it back with `pool.release`. Leases end with the client connection
//! (plus grace) or a TTL; a reaper stops and deletes expired clones. How many
//! clones run at once comes from the host RAM budget (13a): a full pool makes
//! `pool.get` wait, then fail with `resource_exhausted`. Persistent agent
//! devices stay on `device.create`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use jidr_adb::Adb;
use jidr_core::provider::{CallCtx, CommandProvider, CommandSpec, DaemonContext, parse_params};
use jidr_core::{Device, ErrorCode, JidrError, JidrResult, LaunchPrefs, RootMode};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::core_provider::move_device_files;
use crate::leases::{Holder, Lease};
use crate::state::{State, lock};

/// The snapshot a base keeps and its clones resume.
pub const BASE_SNAPSHOT: &str = "jidr-base";
/// A TTL lease's default lifetime (the CLI's).
pub const DEFAULT_TTL: Duration = Duration::from_secs(2 * 60 * 60);
/// How long `pool.get` waits for room in the budget by default.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaseCreateParams {
    /// A running device to make the base from (any root mode; a Magisk
    /// source must pass `su -c id`). It is stopped; the base is a copy of it.
    pub from_device: String,
    /// The base's name.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaseParams {
    /// Base name or id.
    pub base: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PoolGetParams {
    /// Base name or id.
    pub base: String,
    /// Lease lifetime in seconds. Without it the lease lasts as long as this
    /// client connection, plus 2 minutes.
    #[serde(default)]
    pub ttl_s: Option<u64>,
    /// How long to wait for room in the RAM budget (default 120 s).
    #[serde(default)]
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PoolReleaseParams {
    /// The lease id from `pool.get`.
    #[serde(default)]
    pub lease: Option<Uuid>,
    /// Or the pool device's name or id.
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PoolRenewParams {
    pub lease: Uuid,
    /// New lifetime from now, in seconds (default 2 h).
    #[serde(default)]
    pub ttl_s: Option<u64>,
}

pub struct PoolProvider {
    state: Arc<State>,
}

impl PoolProvider {
    pub fn new(state: Arc<State>) -> Self {
        Self { state }
    }
}

fn json<T: serde::Serialize>(v: T) -> JidrResult<Value> {
    serde_json::to_value(v).map_err(|e| JidrError::unexpected(format!("encoding result: {e}")))
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..6].to_string()
}

impl State {
    pub fn lease_count(&self) -> usize {
        lock(&self.leases).len()
    }

    /// A client connection closed: its leases expire after the grace period.
    pub fn connection_closed(&self, conn: u64) {
        lock(&self.leases).disconnected(conn, Utc::now());
    }

    fn base(&self, selector: &str) -> JidrResult<Device> {
        let d = self.with_store(|s| s.find(selector).cloned())?;
        if !d.base {
            return Err(JidrError::usage(format!(
                "{} is not a base; make one with base.create",
                d.name
            )));
        }
        Ok(d)
    }

    /// Stops (without saving) and deletes a device: a pool clone or a base.
    async fn remove_device(&self, id: Uuid) -> JidrResult<Device> {
        let d = self.with_store(|s| s.get(id).cloned())?;
        if d.status.is_active() {
            self.stop_device(id).await?;
        }
        let avds = self.avds();
        if avds.exists(&d.avd) {
            avds.delete(&d.avd)?;
        }
        let d = self.with_store(|s| s.remove(id))?;
        self.emit("device.deleted", json!({ "device": d.id, "name": d.name }));
        Ok(d)
    }

    /// Ends a lease: its clone is stopped and deleted.
    pub async fn release(&self, lease: Uuid) -> JidrResult<Value> {
        let l = lock(&self.leases)
            .get(lease)
            .cloned()
            .ok_or_else(|| JidrError::not_found(format!("no lease {lease}")))?;
        let name = self
            .with_store(|s| s.get(l.device).map(|d| d.name.clone()))
            .unwrap_or_default();
        let _guard = self.locks.try_lock(l.device, &name, "pool.release", None)?;
        if self.with_store(|s| s.get(l.device).is_ok()) {
            self.remove_device(l.device).await?;
        }
        lock(&self.leases).remove(lease);
        self.emit(
            "pool.released",
            json!({ "lease": lease, "device": l.device }),
        );
        Ok(json!({ "released": lease, "device": l.device, "name": name }))
    }

    /// One reaper pass as of `now`: releases every expired lease. Returns
    /// the leases it released (busy ones wait for the next pass).
    pub async fn reap(&self, now: chrono::DateTime<Utc>) -> Vec<Uuid> {
        let due = lock(&self.leases).expired(now);
        let mut done = Vec::new();
        for l in due {
            match self.release(l.id).await {
                Ok(_) => done.push(l.id),
                Err(e) => tracing::warn!(lease = %l.id, "reaping failed: {e}"),
            }
        }
        done
    }

    /// Pool clones left from before a daemon restart have no connection any
    /// more: they get the grace period, then the reaper takes them.
    pub(crate) fn adopt_pool_clones(&self) {
        let now = Utc::now();
        let clones: Vec<Device> = self.with_store(|s| {
            s.all()
                .iter()
                .filter(|d| d.pool_base.is_some())
                .cloned()
                .collect()
        });
        let mut leases = lock(&self.leases);
        for d in clones {
            if leases.for_device(d.id).is_none() {
                leases.insert(Lease {
                    id: Uuid::new_v4(),
                    base: d.pool_base.unwrap_or_default(),
                    device: d.id,
                    holder: Holder::Ttl,
                    expires_at: Some(
                        now + chrono::Duration::from_std(crate::leases::GRACE).unwrap_or_default(),
                    ),
                    created_at: now,
                });
            }
        }
    }
}

/// A Magisk base needs its patched ramdisk and a shell `su` that works
/// without a prompt, because every clone resumes that state. On a Play Store
/// image that is the grant the person gave in Magisk's prompt (ADR-0002);
/// Jidr never grants it for them.
async fn check_magisk_source(src: &Device, adb: &Adb) -> JidrResult<()> {
    if !src.launch.ramdisk.as_deref().is_some_and(Path::is_file) {
        return Err(JidrError::usage(format!(
            "{} has no patched ramdisk; run `root set {} magisk` again first",
            src.name, src.name
        )));
    }
    let out = adb
        .clone()
        .with_timeout(Duration::from_secs(20))
        .shell_status("su -c id")
        .await
        .map(|(_, out)| out)
        .unwrap_or_default();
    if out.contains("uid=0(") {
        return Ok(());
    }
    let hint = if src.image.allows_adb_root() {
        "check it with `root verify`"
    } else {
        "on a Play Store image, tap Grant in Magisk's prompt and choose Forever"
    };
    Err(JidrError::root_failed(format!(
        "{}'s shell `su -c id` isn't root, so its clones wouldn't be either; {hint}",
        src.name
    )))
}

impl PoolProvider {
    async fn base_create(&self, ctx: &CallCtx, p: BaseCreateParams) -> JidrResult<Value> {
        let st = &self.state;
        let src = st.device(&p.from_device).await?;
        if src.base || src.pool_base.is_some() {
            return Err(JidrError::usage(format!(
                "{} is already a base or a pool device",
                src.name
            )));
        }
        if src.root.writable_system {
            // #25: the writable system copy isn't part of the base snapshot.
            return Err(JidrError::usage(format!(
                "{} has a writable /system; turn it off first (root.writable_system false)",
                src.name
            )));
        }
        let info = st.connect_info(src.id).await.map_err(|_| {
            JidrError::not_found(format!(
                "start {} first: a base is saved from a running device",
                src.name
            ))
        })?;
        let name = p.name.trim().to_string();
        crate::core_provider::validate_device_name(&name)?;
        if st.with_store(|s| s.name_taken(&name, None)) {
            return Err(JidrError::usage(format!(
                "a device named {name:?} already exists"
            )));
        }
        let _guard = st.locks.try_lock(src.id, &src.name, "base.create", None)?;

        let adb = Adb::new(info.adb_path.clone()).device(info.adb_serial.clone());
        if src.root.mode == RootMode::Magisk {
            ctx.report(Some(0.05), "Checking Magisk root");
            check_magisk_source(&src, &adb).await?;
        }
        // Agents' clones shouldn't wait on animations.
        ctx.report(Some(0.1), "Turning animations off");
        for key in [
            "window_animation_scale",
            "transition_animation_scale",
            "animator_duration_scale",
        ] {
            adb.shell(&format!("settings put global {key} 0")).await?;
        }
        ctx.report(Some(0.3), "Saving the base snapshot");
        let inst = st
            .instance(src.id)
            .ok_or_else(|| JidrError::not_found(format!("{} is not running", src.name)))?;
        let src_dir = st.avds().dir(&src.avd);
        st.backend
            .save_snapshot(&inst, BASE_SNAPSHOT, &src_dir)
            .await?;
        jidr_sdk::avd::check_snapshot(&src_dir, BASE_SNAPSHOT)?;
        ctx.report(Some(0.6), "Stopping the source");
        st.stop_device(src.id).await?;

        ctx.report(Some(0.8), "Copying");
        let avds = st.avds();
        let avd = st.with_store(|s| s.avd_id_for(&name, |id| avds.exists(id)));
        avds.clone_with_snapshot(&src.avd, &avd, &name, BASE_SNAPSHOT)?;
        // The source keeps running as an ordinary device; its copy of the
        // snapshot isn't needed.
        let _ = std::fs::remove_dir_all(src_dir.join("snapshots").join(BASE_SNAPSHOT));
        // A Magisk base keeps its own copy of the patched ramdisk (13f).
        let mut launch = LaunchPrefs {
            force_cold_boot: false,
            resume_snapshot: None,
            ..src.launch.clone()
        };
        let mut root = src.root.clone();
        if root.mode == RootMode::Magisk {
            move_device_files(&mut launch, &mut root, &src_dir, &avds.dir(&avd));
        } else {
            launch.ramdisk = None;
        }
        let base = Device {
            id: Uuid::new_v4(),
            name,
            avd: avd.clone(),
            created_at: Utc::now(),
            launch,
            root,
            status: Default::default(),
            runtime: None,
            agent_owned: false,
            base: true,
            pool_base: None,
            network_shaping: Default::default(),
            auto_hibernate_minutes: None,
            ..src.clone()
        };
        if let Err(e) = st.with_store(|s| s.insert(base.clone())) {
            let _ = avds.delete(&avd);
            return Err(e);
        }
        st.emit(
            "device.created",
            json!({ "device": base.id, "name": base.name }),
        );
        json(base)
    }

    async fn base_list(&self) -> JidrResult<Value> {
        let leases = lock(&self.state.leases).all();
        let bases: Vec<Value> = self
            .state
            .devices()
            .await
            .into_iter()
            .filter(|d| d.base)
            .map(|d| {
                let n = leases.iter().filter(|l| l.base == d.id).count();
                json!({ "id": d.id, "name": d.name, "image": d.image, "profile": d.profile, "root": d.root.mode, "leases": n, "created_at": d.created_at })
            })
            .collect();
        Ok(Value::Array(bases))
    }

    async fn base_delete(&self, p: BaseParams) -> JidrResult<Value> {
        let b = self.state.base(&p.base)?;
        let n = lock(&self.state.leases).of_base(b.id);
        if n > 0 {
            return Err(JidrError::busy(format!(
                "{} has {n} leased clone(s); release them first",
                b.name
            ))
            .with_data(json!({ "leases": n })));
        }
        let _guard = self
            .state
            .locks
            .try_lock(b.id, &b.name, "base.delete", None)?;
        let d = self.state.remove_device(b.id).await?;
        Ok(json!({ "deleted": d.id, "name": d.name }))
    }

    async fn pool_get(&self, ctx: &CallCtx, p: PoolGetParams) -> JidrResult<Value> {
        let st = &self.state;
        let base = st.base(&p.base)?;
        let now = Utc::now();
        let holder = match (p.ttl_s, ctx.connection) {
            (None, Some(id)) => Holder::Connection { id },
            _ => Holder::Ttl,
        };
        let ttl = p.ttl_s.map(Duration::from_secs).unwrap_or(DEFAULT_TTL);
        let expires_at = (holder == Holder::Ttl)
            .then(|| now + chrono::Duration::from_std(ttl).unwrap_or_default());

        ctx.report(Some(0.05), "Cloning the base");
        let name = format!("{}-{}", base.name, short_id());
        let avds = st.avds();
        let avd = st.with_store(|s| s.avd_id_for(&name, |id| avds.exists(id)));
        avds.clone_with_snapshot(&base.avd, &avd, &name, BASE_SNAPSHOT)?;
        let mut launch = LaunchPrefs {
            resume_snapshot: Some(BASE_SNAPSHOT.into()),
            force_cold_boot: false,
            ..base.launch.clone()
        };
        // Each clone of a Magisk base boots its own copy of the patched
        // ramdisk; the root tag and Magisk version carry over (13f).
        let mut root = base.root.clone();
        move_device_files(
            &mut launch,
            &mut root,
            &avds.dir(&base.avd),
            &avds.dir(&avd),
        );
        if let Some(r) = launch.ramdisk.as_deref().filter(|r| !r.is_file()) {
            let _ = avds.delete(&avd);
            return Err(JidrError::not_found(format!(
                "{} lost its patched ramdisk ({}); make the base again",
                base.name,
                r.display()
            )));
        }
        let device = Device {
            id: Uuid::new_v4(),
            name,
            avd: avd.clone(),
            created_at: now,
            launch,
            root,
            status: Default::default(),
            runtime: None,
            agent_owned: true,
            base: false,
            pool_base: Some(base.id),
            network_shaping: Default::default(),
            // Throwaway: never hibernated, never resumed from its own state.
            auto_hibernate_minutes: Some(0),
            ..base.clone()
        };
        if let Err(e) = st.with_store(|s| s.insert(device.clone())) {
            let _ = avds.delete(&avd);
            return Err(e);
        }
        let lease = Lease {
            id: Uuid::new_v4(),
            base: base.id,
            device: device.id,
            holder,
            expires_at,
            created_at: now,
        };
        // Registered before the boot, so a client that disconnects mid-boot
        // still gets its clone reaped.
        lock(&st.leases).insert(lease.clone());

        let result = self.boot_clone(ctx, &device, &p).await;
        match result {
            Ok(d) => {
                let info = st.connect_info(d.id).await?;
                st.emit(
                    "pool.leased",
                    json!({ "lease": lease.id, "device": d.id, "base": base.id }),
                );
                Ok(json!({
                    "lease": lease,
                    "device": d,
                    "serial": info.adb_serial,
                    "connect": info,
                }))
            }
            Err(e) => {
                lock(&st.leases).remove(lease.id);
                let _ = st.remove_device(device.id).await;
                Err(e)
            }
        }
    }

    /// Waits for room in the budget, then resumes the clone.
    async fn boot_clone(&self, ctx: &CallCtx, d: &Device, p: &PoolGetParams) -> JidrResult<Device> {
        let st = &self.state;
        let wait = p.timeout_s.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT);
        let deadline = tokio::time::Instant::now() + wait;
        let mut waiting = false;
        loop {
            ctx.cancel.check()?;
            match st.admit(d.id, false) {
                Ok(()) => break,
                Err(e) if e.code == ErrorCode::ResourceExhausted => {
                    if tokio::time::Instant::now() >= deadline {
                        let mut e = e;
                        e.message = format!(
                            "the pool is full: no room in the RAM budget after {} s; {}",
                            wait.as_secs(),
                            e.message
                        );
                        return Err(e);
                    }
                    if !waiting {
                        ctx.report(Some(0.1), "Waiting for room in the RAM budget");
                        waiting = true;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(e) => return Err(e),
            }
        }
        ctx.report(Some(0.2), "Resuming the base snapshot");
        st.start_device(d.id, false, &ctx.cancel).await
    }
}

#[async_trait]
impl CommandProvider for PoolProvider {
    fn name(&self) -> &str {
        "pool"
    }

    fn commands(&self) -> Vec<CommandSpec> {
        vec![
            CommandSpec::new::<BaseCreateParams>(
                "base.create",
                "Save a running device as a read-only base for pool clones (snapshot jidr-base, animations off)",
            )
            .long_running(),
            CommandSpec::new::<jidr_core::provider::NoParams>("base.list", "Bases and how many clones each has leased"),
            CommandSpec::new::<BaseParams>("base.delete", "Delete a base (none of its clones may be leased)")
                .destructive(),
            CommandSpec::new::<PoolGetParams>(
                "pool.get",
                "Lease a throwaway clone of a base, booted from its snapshot; returns its serial. Release it when done",
            )
            .long_running(),
            CommandSpec::new::<PoolReleaseParams>("pool.release", "Give a leased clone back: it is stopped and deleted"),
            CommandSpec::new::<PoolRenewParams>("pool.renew", "Extend a lease (TTL leases, or one in its grace period)"),
            CommandSpec::new::<jidr_core::provider::NoParams>("pool.list", "Current leases"),
        ]
    }

    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
        match method {
            "base.create" => self.base_create(ctx, parse_params(params)?).await,
            "base.list" => {
                parse_params::<jidr_core::provider::NoParams>(params)?;
                self.base_list().await
            }
            "base.delete" => self.base_delete(parse_params(params)?).await,
            "pool.get" => self.pool_get(ctx, parse_params(params)?).await,
            "pool.release" => {
                let p: PoolReleaseParams = parse_params(params)?;
                let lease = match (p.lease, p.device) {
                    (Some(l), None) => l,
                    (None, Some(sel)) => {
                        let d = self.state.device(&sel).await?;
                        lock(&self.state.leases)
                            .for_device(d.id)
                            .map(|l| l.id)
                            .ok_or_else(|| {
                                JidrError::not_found(format!(
                                    "{} is not a leased pool device",
                                    d.name
                                ))
                            })?
                    }
                    _ => return Err(JidrError::usage("give either lease or device")),
                };
                self.state.release(lease).await
            }
            "pool.renew" => {
                let p: PoolRenewParams = parse_params(params)?;
                let ttl = p.ttl_s.map(Duration::from_secs).unwrap_or(DEFAULT_TTL);
                let l = lock(&self.state.leases)
                    .renew(p.lease, ttl, Utc::now())
                    .ok_or_else(|| JidrError::not_found(format!("no lease {}", p.lease)))?;
                json(l)
            }
            "pool.list" => {
                parse_params::<jidr_core::provider::NoParams>(params)?;
                let leases = lock(&self.state.leases).all();
                let devices = self.state.devices().await;
                let rows: Vec<Value> = leases
                    .into_iter()
                    .map(|l| {
                        let d = devices.iter().find(|d| d.id == l.device);
                        let base = devices.iter().find(|d| d.id == l.base);
                        json!({
                            "lease": l,
                            "device": d.map(|d| d.name.clone()),
                            "base": base.map(|d| d.name.clone()),
                            "status": d.map(|d| d.status.clone()),
                            "serial": d.and_then(|d| d.runtime.as_ref().map(|r| r.adb_serial.clone())),
                        })
                    })
                    .collect();
                Ok(Value::Array(rows))
            }
            other => Err(JidrError::usage(format!(
                "pool provider has no command {other}"
            ))),
        }
    }
}
