//! Auto-hibernate (D42, Phase 13c): a running device with no Jidr command
//! for it (CLI, GUI, MCP all go through the daemon) and no player attached
//! for N minutes is hibernated. Off by default; set globally
//! (`auto_hibernate_minutes`) or per device. adb use that bypasses the
//! daemon isn't seen.

use std::time::{Duration, Instant};

use jidr_core::DeviceStatus;
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

use crate::state::{State, lock};

/// How often the daemon checks for idle devices.
pub const CHECK_EVERY: Duration = Duration::from_secs(60);

/// A running device and how long it has been idle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct IdleDevice {
    pub id: Uuid,
    pub name: String,
    pub idle_s: u64,
}

impl State {
    /// Marks activity on a device (any command naming it).
    pub fn touch(&self, id: Uuid) {
        if self.instance(id).is_some() {
            lock(&self.activity).insert(id, Instant::now());
        }
    }

    fn idle_for(&self, id: Uuid, now: Instant) -> Duration {
        lock(&self.activity)
            .get(&id)
            .map(|t| now.saturating_duration_since(*t))
            .unwrap_or_default()
    }

    /// Running devices, the longest-idle first.
    pub fn idle_devices(&self) -> Vec<IdleDevice> {
        let now = Instant::now();
        let mut v: Vec<IdleDevice> = self
            .with_store(|s| s.all().to_vec())
            .into_iter()
            .filter(|d| matches!(d.status, DeviceStatus::Running))
            .map(|d| IdleDevice {
                idle_s: self.idle_for(d.id, now).as_secs(),
                id: d.id,
                name: d.name,
            })
            .collect();
        v.sort_by_key(|d| std::cmp::Reverse(d.idle_s));
        v
    }

    /// One auto-hibernate pass as of `now`: hibernates every running device
    /// idle past its limit with no player attached, skipping busy ones.
    /// Returns the ones it hibernated.
    pub async fn auto_hibernate(&self, now: Instant) -> Vec<Uuid> {
        let global = self.settings().auto_hibernate_minutes;
        let devices = self.with_store(|s| s.all().to_vec());
        let mut done = Vec::new();
        for d in devices {
            let minutes = d.auto_hibernate_minutes.unwrap_or(global);
            let Some(info) = self.instance(d.id) else {
                continue;
            };
            if minutes == 0
                || !matches!(d.status, DeviceStatus::Running)
                || self.idle_for(d.id, now) < Duration::from_secs(u64::from(minutes) * 60)
            {
                continue;
            }
            if self.backend.has_clients(&info).await {
                // A player is watching: that's activity.
                lock(&self.activity).insert(d.id, now);
                continue;
            }
            let Ok(_guard) = self.locks.try_lock(d.id, &d.name, "device.hibernate", None) else {
                continue;
            };
            match self.hibernate(d.id).await {
                Ok(_) => done.push(d.id),
                Err(e) => {
                    tracing::warn!(device = %d.name, "auto-hibernate failed: {e}");
                    // Don't retry every minute.
                    lock(&self.activity).insert(d.id, now);
                }
            }
        }
        done
    }
}
