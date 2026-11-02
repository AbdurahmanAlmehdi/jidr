//! Hibernate (D42, Phase 13c): save a running device's whole state under a
//! name, check it really holds memory, then end the emulator without the
//! quick-boot save. The device uses no host RAM until `device.start` resumes
//! it from that snapshot, in seconds. Pausing alone would only free CPU.

use chrono::Utc;
use jidr_core::provider::DaemonContext;
use jidr_core::{Device, DeviceStatus, JidrError, JidrResult};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::state::{State, lock};

/// The snapshot a hibernated device resumes from.
pub const SNAPSHOT: &str = "jidr-hibernate";

impl State {
    /// `device.hibernate`. Callers hold the device lock. When the save fails
    /// or leaves an empty `ram.bin`, the device keeps running.
    pub async fn hibernate(&self, id: Uuid) -> JidrResult<Device> {
        let d = self.with_store(|s| s.get(id).cloned())?;
        let info = self
            .instance(id)
            .ok_or_else(|| JidrError::not_found(format!("{} is not running", d.name)))?;
        let avd_dir = self.avds().dir(&d.avd);
        self.backend
            .save_snapshot(&info, SNAPSHOT, &avd_dir)
            .await?;
        jidr_sdk::avd::check_snapshot(&avd_dir, SNAPSHOT)
            .map_err(|e| JidrError::sdk(format!("hibernating {}: {}", d.name, e.message)))?;
        tag_snapshot(&avd_dir, &d)?;

        if let Some(w) = lock(&self.watchers).remove(&id) {
            w.abort();
        }
        lock(&self.instances).remove(&id);
        self.set_status(id, DeviceStatus::Stopping, None);
        if let Err(e) = self.backend.stop_without_save(&info).await {
            lock(&self.instances).insert(id, info.clone());
            self.set_status(id, DeviceStatus::Running, None);
            self.watch_crash(id, info);
            return Err(e);
        }
        self.finish_footprint(id);
        lock(&self.activity).remove(&id);
        self.with_store(|s| s.update(id, |d| d.launch.resume_snapshot = Some(SNAPSHOT.into())))?;
        self.set_status(id, DeviceStatus::Hibernated, Some(None));
        self.emit("device.hibernated", json!({ "device": id, "name": d.name }));
        self.with_store(|s| s.get(id).cloned())
    }
}

/// Records the device's root mode on the snapshot, in the snapshot
/// manager's `jidr/snapshots.json`, so the root tag travels with it (D29).
fn tag_snapshot(avd_dir: &std::path::Path, d: &Device) -> JidrResult<()> {
    let path = avd_dir.join("jidr").join("snapshots.json");
    let mut tags: serde_json::Map<String, Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut tag = json!({
        "label": "Hibernated",
        "root_mode": d.root.mode,
        "created_at": Utc::now(),
    });
    if let Some(v) = &d.root.magisk_version {
        tag["magisk_version"] = json!(v);
    }
    tags.insert(SNAPSHOT.into(), tag);
    let text =
        serde_json::to_string_pretty(&tags).map_err(|e| JidrError::unexpected(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| JidrError::unexpected(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&path, text)
        .map_err(|e| JidrError::unexpected(format!("{}: {e}", path.display())))
}
