//! Backend trait and the AOSP emulator backend (spawn, discovery, gRPC/console
//! clients, ports, boot wait, crash detection).

pub mod aosp;
pub mod discovery;
pub mod grpc;
pub mod ports;

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use jidr_core::{Cancel, DeviceRuntime, JidrResult};
use serde::{Deserialize, Serialize};

pub use aosp::AospBackend;
pub use ports::{PortAllocator, Ports};

/// What to launch.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// AVD id.
    pub avd: String,
    /// `ANDROID_AVD_HOME` (Jidr's own AVD home).
    pub avd_home: PathBuf,
    /// `ANDROID_SDK_ROOT`: the SDK holding the system image.
    pub sdk_root: PathBuf,
    /// The `emulator` binary.
    pub emulator: PathBuf,
    /// The `adb` binary.
    pub adb: PathBuf,
    /// Directory for per-boot emulator logs.
    pub log_dir: PathBuf,
}

/// How to launch. `ramdisk` is the hook the root engine uses for Magisk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptions {
    /// Extra emulator arguments, appended last.
    pub extra_args: Vec<String>,
    /// `-ramdisk <path>`: boot a patched per-device ramdisk.
    pub ramdisk: Option<PathBuf>,
    /// `-no-snapshot-load`: ignore the quick-boot snapshot.
    pub cold_boot: bool,
    /// `-wipe-data`.
    pub wipe: bool,
    /// `-gpu` mode; `None` = `host` on macOS (ADR-0001), `swiftshader_indirect` elsewhere.
    pub gpu: Option<String>,
    /// Guest audio to the host; `false` adds `-no-audio`.
    pub audio: bool,
    /// Save the quick-boot snapshot when stopped (D29); `false` adds `-no-snapshot-save`.
    pub snapshot_save: bool,
    pub boot_timeout: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            extra_args: Vec::new(),
            ramdisk: None,
            cold_boot: false,
            wipe: false,
            gpu: None,
            audio: true,
            snapshot_save: true,
            boot_timeout: Duration::from_secs(300),
        }
    }
}

pub fn default_gpu() -> &'static str {
    if cfg!(target_os = "macos") {
        "host"
    } else {
        "swiftshader_indirect"
    }
}

/// Boot progress, reported as it happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BootStage {
    Spawned,
    GrpcReady,
    AdbOnline,
    Booted,
}

impl BootStage {
    /// Rough progress for job reporting.
    pub fn progress(self) -> f32 {
        match self {
            Self::Spawned => 0.1,
            Self::GrpcReady => 0.3,
            Self::AdbOnline => 0.5,
            Self::Booted => 1.0,
        }
    }
}

/// A running emulator instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InstanceInfo {
    pub avd: String,
    pub pid: u32,
    pub console_port: u16,
    pub adb_port: u16,
    pub grpc_port: u16,
    pub adb_serial: String,
    /// Bearer token for gRPC (`-grpc-use-token`). Never logged.
    pub grpc_token: String,
    pub started_at: DateTime<Utc>,
    /// The emulator's stdout/stderr log for this boot (`None` when adopted).
    pub log_path: Option<PathBuf>,
}

impl InstanceInfo {
    pub fn ports(&self) -> Ports {
        Ports {
            console: self.console_port,
            adb: self.adb_port,
            grpc: self.grpc_port,
        }
    }

    pub fn runtime(&self) -> DeviceRuntime {
        DeviceRuntime {
            pid: self.pid,
            console_port: self.console_port,
            adb_serial: self.adb_serial.clone(),
            grpc_port: self.grpc_port,
            started_at: self.started_at,
        }
    }
}

pub type BootProgress<'a> = &'a (dyn Fn(BootStage) + Send + Sync);

/// A device backend. Object-safe so the daemon can run against a fake.
#[async_trait]
pub trait Backend: Send + Sync + std::fmt::Debug {
    /// Launches and waits until the device has booted.
    async fn start(
        &self,
        spec: &LaunchSpec,
        opts: &LaunchOptions,
        progress: BootProgress<'_>,
        cancel: &Cancel,
    ) -> JidrResult<InstanceInfo>;

    /// Shuts the device down gracefully (saving the quick-boot snapshot when
    /// it was launched with `snapshot_save`), killing it if it hangs.
    async fn stop(&self, instance: &InstanceInfo) -> JidrResult<()>;

    /// True if the instance's process is still running.
    async fn is_alive(&self, instance: &InstanceInfo) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_progress_monotonically() {
        let s = [
            BootStage::Spawned,
            BootStage::GrpcReady,
            BootStage::AdbOnline,
            BootStage::Booted,
        ];
        assert!(s.windows(2).all(|w| w[0].progress() < w[1].progress()));
    }
}
