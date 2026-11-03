//! Host RAM budget (D42, Phase 13a). Running devices may use a share of host
//! RAM (`Settings.budget_percent`, 75% by default). A device is expected to
//! cost its guest RAM plus ~2 GB for the emulator itself (host GPU, QEMU),
//! or what its last run actually peaked at once the daemon has measured it.

use jidr_core::JidrError;
use jidr_sdk::host::HostMemory;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

/// What the emulator costs on top of guest RAM (measured 2026-11-02).
pub const EMULATOR_OVERHEAD_MB: u64 = 2048;

/// How often the daemon samples running devices' footprints (`top` takes
/// seconds on a busy Mac).
pub const SAMPLE_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// The cost model: the measured peak of the device's last run when known,
/// otherwise `ram_mb + 2048`.
pub fn expected_mb(ram_mb: u32, last_footprint_mb: Option<u32>) -> u64 {
    match last_footprint_mb {
        Some(mb) if mb > 0 => u64::from(mb),
        _ => u64::from(ram_mb) + EMULATOR_OVERHEAD_MB,
    }
}

/// One device counted against the budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DeviceUse {
    pub id: Uuid,
    pub name: String,
    /// What the cost model expects it to use.
    pub expected_mb: u64,
    /// What it uses right now, when the daemon has measured it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_mb: Option<u64>,
}

impl DeviceUse {
    /// The larger of the expectation and the latest measurement.
    pub fn counted_mb(&self) -> u64 {
        self.expected_mb.max(self.measured_mb.unwrap_or(0))
    }
}

/// `host.budget`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Budget {
    pub total_mb: u64,
    /// What the OS could hand out now (for information; the budget is a
    /// share of total RAM).
    pub available_mb: u64,
    pub budget_percent: u8,
    pub budget_mb: u64,
    /// Sum over the running (and starting) devices.
    pub used_mb: u64,
    pub devices: Vec<DeviceUse>,
}

impl Budget {
    pub fn new(mem: HostMemory, budget_percent: u8, devices: Vec<DeviceUse>) -> Self {
        let budget_percent = budget_percent.clamp(10, 100);
        Self {
            total_mb: mem.total_mb,
            available_mb: mem.available_mb,
            budget_percent,
            budget_mb: mem.total_mb * u64::from(budget_percent) / 100,
            used_mb: devices.iter().map(DeviceUse::counted_mb).sum(),
            devices,
        }
    }

    pub fn fits(&self, need_mb: u64) -> bool {
        self.used_mb + need_mb <= self.budget_mb
    }

    /// `resource_exhausted` for starting `name` (needing `need_mb`), naming
    /// the devices that use the budget.
    pub fn refusal(&self, name: &str, need_mb: u64) -> JidrError {
        let users = if self.devices.is_empty() {
            "nothing else is running".to_string()
        } else {
            self.devices
                .iter()
                .map(|d| format!("{} ({:.1} GB)", d.name, d.counted_mb() as f64 / 1024.0))
                .collect::<Vec<_>>()
                .join(", ")
        };
        JidrError::resource_exhausted(format!(
            "starting {name} needs ~{:.1} GB but only {:.1} of {:.1} GB of the RAM budget is free (in use: {users}); hibernate or stop a device, use the agent-lean profile, or start with --force",
            need_mb as f64 / 1024.0,
            self.budget_mb.saturating_sub(self.used_mb) as f64 / 1024.0,
            self.budget_mb as f64 / 1024.0,
        ))
        .with_data(json!({
            "needed_mb": need_mb,
            "used_mb": self.used_mb,
            "budget_mb": self.budget_mb,
            "total_mb": self.total_mb,
            "devices": self.devices,
            "suggestions": ["hibernate", "stop", "lean_profile", "force"],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jidr_core::ErrorCode;

    fn mem(total_mb: u64) -> HostMemory {
        HostMemory {
            total_mb,
            available_mb: total_mb / 2,
        }
    }

    fn running(name: &str, expected_mb: u64, measured_mb: Option<u64>) -> DeviceUse {
        DeviceUse {
            id: Uuid::new_v4(),
            name: name.into(),
            expected_mb,
            measured_mb,
        }
    }

    #[test]
    fn cost_model() {
        assert_eq!(expected_mb(2048, None), 4096, "lean: ram + 2 GB");
        assert_eq!(expected_mb(4096, None), 6144);
        assert_eq!(expected_mb(4096, Some(7800)), 7800, "measured wins");
        assert_eq!(expected_mb(2048, Some(3900)), 3900, "and can be lower");
        assert_eq!(expected_mb(2048, Some(0)), 4096, "a zero sample is no data");
    }

    #[test]
    fn budget_decision() {
        // 16 GB host, 75%: 12 GB budget. Three lean devices fit, a fourth doesn't.
        let empty = Budget::new(mem(16384), 75, vec![]);
        assert_eq!(empty.budget_mb, 12288);
        assert!(empty.fits(4096));
        let two = Budget::new(
            mem(16384),
            75,
            vec![running("a", 4096, None), running("b", 4096, Some(3900))],
        );
        assert_eq!(two.used_mb, 8192, "the expectation counts until exceeded");
        assert!(two.fits(4096));
        let three = Budget::new(
            mem(16384),
            75,
            vec![
                running("a", 4096, None),
                running("b", 4096, Some(4300)),
                running("c", 4096, None),
            ],
        );
        assert_eq!(three.used_mb, 12492, "a measurement above it counts");
        assert!(!three.fits(4096));
        // Two default devices measured at ~7.6 GB don't fit on 16 GB.
        let one_default = Budget::new(mem(16384), 75, vec![running("t", 7800, None)]);
        assert!(!one_default.fits(7800));
        // The percentage is clamped.
        assert_eq!(Budget::new(mem(1000), 0, vec![]).budget_mb, 100);
        assert_eq!(Budget::new(mem(1000), 200, vec![]).budget_mb, 1000);
    }

    #[test]
    fn refusal_names_the_users() {
        let b = Budget::new(mem(16384), 75, vec![running("Tablet", 7800, Some(8100))]);
        let e = b.refusal("Phone", 6144);
        assert_eq!(e.code, ErrorCode::ResourceExhausted);
        assert!(e.message.contains("Tablet (7.9 GB)"), "{e}");
        assert!(e.message.contains("hibernate") && e.message.contains("--force"));
        let d = e.data.unwrap();
        assert_eq!(d["needed_mb"], 6144);
        assert_eq!(d["used_mb"], 8100);
        assert_eq!(d["devices"][0]["name"], "Tablet");
    }
}
