//! Per-device single-writer locks. A conflicting call fails fast with `busy`
//! naming the holder, instead of queueing behind a long boot.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use jidr_core::{JidrError, JidrResult};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct Holder {
    /// The command holding the lock, e.g. `device.start`.
    op: String,
    /// The job id, when the holder is a job.
    job: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceLocks {
    held: Arc<Mutex<HashMap<Uuid, Holder>>>,
}

/// Releases the lock when dropped.
#[derive(Debug)]
pub struct LockGuard {
    locks: DeviceLocks,
    device: Uuid,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.locks
            .held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.device);
    }
}

impl DeviceLocks {
    pub fn try_lock(
        &self,
        device: Uuid,
        device_name: &str,
        op: &str,
        job: Option<Uuid>,
    ) -> JidrResult<LockGuard> {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = held.get(&device) {
            return Err(JidrError::busy(format!("{device_name} is busy with {}", h.op))
                .with_data(json!({ "holder": h.job.map(|j| j.to_string()).unwrap_or_else(|| h.op.clone()), "op": h.op, "device": device })));
        }
        held.insert(
            device,
            Holder {
                op: op.to_string(),
                job,
            },
        );
        Ok(LockGuard {
            locks: self.clone(),
            device,
        })
    }

    /// Records the job id once the holder became a job.
    pub fn set_job(&self, device: Uuid, job: Uuid) {
        if let Some(h) = self
            .held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&device)
        {
            h.job = Some(job);
        }
    }

    pub fn is_locked(&self, device: Uuid) -> bool {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jidr_core::ErrorCode;

    #[test]
    fn single_writer() {
        let locks = DeviceLocks::default();
        let d = Uuid::new_v4();
        let g = locks.try_lock(d, "phone", "device.start", None).unwrap();
        let err = locks.try_lock(d, "phone", "device.stop", None).unwrap_err();
        assert_eq!(err.code, ErrorCode::Busy);
        assert_eq!(err.data.as_ref().unwrap()["op"], "device.start");
        let job = Uuid::new_v4();
        locks.set_job(d, job);
        let err = locks.try_lock(d, "phone", "x", None).unwrap_err();
        assert_eq!(err.data.unwrap()["holder"], job.to_string());
        // Other devices are independent.
        let _other = locks
            .try_lock(Uuid::new_v4(), "tablet", "device.stop", None)
            .unwrap();
        drop(g);
        assert!(!locks.is_locked(d));
        locks.try_lock(d, "phone", "device.stop", None).unwrap();
    }
}
