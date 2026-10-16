//! The Jidr domain model, shared by the daemon, the clients and the providers.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A hardware profile: what the device looks like (D24: generic names).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HardwareProfile {
    /// Stable id, e.g. `phone-6.1-fhd`.
    pub id: String,
    /// Display name, e.g. `Phone 6.1″ FHD+`.
    pub name: String,
    #[serde(default)]
    pub form_factor: FormFactor,
    pub width: u32,
    pub height: u32,
    pub density: u32,
    pub ram_mb: u32,
    pub cpus: u32,
    /// Data partition size in MB.
    #[serde(default = "default_data_mb")]
    pub data_mb: u32,
    /// Extra `config.ini` keys applied at creation.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    /// Requested build props (🔎 research: not applied in the MVP).
    #[serde(default)]
    pub build_props: BTreeMap<String, String>,
}

fn default_data_mb() -> u32 {
    6144
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FormFactor {
    #[default]
    Phone,
    Tablet,
    Foldable,
}

/// Identifies a system image: `system-images;android-35;google_apis;arm64-v8a`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct ImageRef {
    pub api: u32,
    /// `default`, `google_apis`, `google_apis_playstore`, `aosp_atd`, …
    pub tag: String,
    pub abi: String,
}

impl ImageRef {
    /// The SDK package path.
    pub fn package(&self) -> String {
        format!(
            "system-images;android-{};{};{}",
            self.api, self.tag, self.abi
        )
    }

    /// Directory relative to the SDK root, as `image.sysdir.1` wants it.
    pub fn sysdir(&self) -> String {
        format!(
            "system-images/android-{}/{}/{}/",
            self.api, self.tag, self.abi
        )
    }

    /// Parses `system-images;android-35;google_apis;arm64-v8a`.
    pub fn parse_package(package: &str) -> Option<Self> {
        let mut parts = package.split(';');
        if parts.next()? != "system-images" {
            return None;
        }
        let api = parts.next()?.strip_prefix("android-")?.parse().ok()?;
        let tag = parts.next()?.to_string();
        let abi = parts.next()?.to_string();
        if parts.next().is_some() || tag.is_empty() || abi.is_empty() {
            return None;
        }
        Some(Self { api, tag, abi })
    }

    /// Images where `adb root` works (plan: default, google_apis, atd).
    pub fn allows_adb_root(&self) -> bool {
        !self.tag.contains("playstore")
    }
}

/// How well an API level is supported (D23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Supported,
    BestEffort,
    Unsupported,
}

impl Support {
    pub fn for_api(api: u32) -> Self {
        match api {
            29..=36 => Self::Supported,
            28 => Self::BestEffort,
            _ => Self::Unsupported,
        }
    }
}

/// A system image in the catalog, installed or not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemImage {
    #[serde(flatten)]
    pub image: ImageRef,
    pub display: String,
    pub revision: String,
    pub support: Support,
    /// Download size in bytes (0 if unknown).
    pub size: u64,
    /// The license id the user must accept before install.
    pub license: Option<String>,
    /// Where it is installed, if it is.
    pub installed_in: Option<PathBuf>,
    /// SHA-256 of the pristine `ramdisk.img` (filled by the root engine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pristine_ramdisk_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RootMode {
    #[default]
    None,
    Adb,
    Magisk,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RootHealth {
    #[default]
    Unverified,
    Ok,
    Failed,
}

/// A device's root state (plan, "Root engine").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RootState {
    pub mode: RootMode,
    #[serde(default)]
    pub writable_system: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magisk_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patched_ramdisk_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub health: RootHealth,
    /// D22: opt-in "auto-grant all" for apps on Magisk devices.
    #[serde(default)]
    pub auto_grant_all: bool,
}

/// Lifecycle status of a device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum DeviceStatus {
    #[default]
    Stopped,
    Starting,
    Booting,
    Running,
    Stopping,
    /// The emulator exited or stopped answering (D9: no auto-restart).
    Crashed {
        reason: String,
        log: Option<PathBuf>,
    },
}

impl DeviceStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Booting | Self::Running | Self::Stopping
        )
    }
}

/// Live connection details of a running device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceRuntime {
    pub pid: u32,
    pub console_port: u16,
    pub adb_serial: String,
    pub grpc_port: u16,
    pub started_at: DateTime<Utc>,
}

/// A Jidr device: an AVD plus Jidr's metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Device {
    pub id: Uuid,
    /// Unique, user-facing name.
    pub name: String,
    /// The AVD id in Jidr's AVD home.
    pub avd: String,
    pub profile: String,
    pub image: ImageRef,
    /// SDK whose image this device boots (the private SDK, or the Studio SDK read-only).
    pub sdk_root: PathBuf,
    #[serde(default)]
    pub root: RootState,
    /// D5: an agent may run destructive commands on it without `--allow-destructive`.
    #[serde(default)]
    pub agent_owned: bool,
    pub created_at: DateTime<Utc>,
    /// Persistent launch settings applied on every start.
    #[serde(default)]
    pub launch: LaunchPrefs,
    #[serde(default)]
    pub status: DeviceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<DeviceRuntime>,
}

/// Per-device launch settings. The root engine sets `ramdisk` (Magisk) and
/// `force_cold_boot` (root changes force a cold boot, D29).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchPrefs {
    /// `-ramdisk <path>`: a patched per-device ramdisk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramdisk: Option<PathBuf>,
    /// Extra emulator arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
    /// `-gpu` override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// The next start ignores the quick-boot snapshot; cleared after a boot.
    #[serde(default)]
    pub force_cold_boot: bool,
    /// `-camera-back`: `emulated`, `virtualscene`, `none` or `webcamN` (R7, D36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_back: Option<String>,
    /// `-camera-front`: same values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_front: Option<String>,
}

/// How to reach a running device directly (the player streams frames from
/// the emulator itself, not through the daemon).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectInfo {
    pub device: Uuid,
    /// `127.0.0.1:<port>`.
    pub grpc_endpoint: String,
    pub grpc_port: u16,
    /// Bearer token for `authorization: Bearer <token>`.
    pub grpc_token: String,
    pub adb_serial: String,
    pub adb_path: PathBuf,
    pub console_port: u16,
}

/// An emulator snapshot, tagged with the root mode it was taken under (D29).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub name: String,
    pub root_mode: RootMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magisk_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Bytes done and expected of a job's current download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobBytes {
    pub done: u64,
    pub total: u64,
}

/// Daemon-wide settings (gap B4), kept in `settings.json` so the CLI, the GUI
/// and agents agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Settings {
    /// Root mode for `device.create` without `root_mode` (D7: Shell where the
    /// image allows adb root, otherwise None).
    pub default_root_mode: RootMode,
    /// Save a quick-boot snapshot when a device stops (D29). Applies from the
    /// device's next start.
    pub quick_boot: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_root_mode: RootMode::Adb,
            quick_boot: true,
        }
    }
}

/// A long-running operation (image download, boot, Magisk patch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Job {
    pub id: Uuid,
    /// The command that started it, e.g. `device.start`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Uuid>,
    /// 0.0 ..= 1.0, or `None` when indeterminate.
    pub progress: Option<f32>,
    /// Human-readable current step.
    #[serde(default)]
    pub message: String,
    /// Bytes done of the current download, while there is one (gap B5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<JobBytes>,
    pub state: JobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::JidrError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_ref_package_round_trip() {
        let r = ImageRef::parse_package("system-images;android-35;google_apis;arm64-v8a").unwrap();
        assert_eq!(r.api, 35);
        assert_eq!(
            r.package(),
            "system-images;android-35;google_apis;arm64-v8a"
        );
        assert_eq!(
            r.sysdir(),
            "system-images/android-35/google_apis/arm64-v8a/"
        );
        assert!(ImageRef::parse_package("platforms;android-35").is_none());
        assert!(ImageRef::parse_package("system-images;android-x;a;b").is_none());
        assert!(ImageRef::parse_package("system-images;android-35;a;b;c").is_none());
    }

    #[test]
    fn support_levels_follow_d23() {
        assert_eq!(Support::for_api(29), Support::Supported);
        assert_eq!(Support::for_api(36), Support::Supported);
        assert_eq!(Support::for_api(28), Support::BestEffort);
        assert_eq!(Support::for_api(27), Support::Unsupported);
    }

    #[test]
    fn device_status_tagged_json() {
        let s = DeviceStatus::Crashed {
            reason: "exit 1".into(),
            log: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["state"], "crashed");
        assert_eq!(
            serde_json::to_value(DeviceStatus::Running).unwrap()["state"],
            "running"
        );
    }

    #[test]
    fn playstore_disallows_adb_root() {
        let ps = ImageRef {
            api: 35,
            tag: "google_apis_playstore".into(),
            abi: "arm64-v8a".into(),
        };
        let ga = ImageRef {
            api: 35,
            tag: "google_apis".into(),
            abi: "arm64-v8a".into(),
        };
        assert!(!ps.allows_adb_root());
        assert!(ga.allows_adb_root());
    }
}
