//! Physical Android phones that `adb` sees (USB debugging on, #92): what
//! `physical.list` and `physical.details` return. They aren't Jidr devices:
//! no AVD, no id, no root modes or snapshots. A phone is named by its adb
//! serial.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A phone's adb state, as the user sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhoneState {
    /// Authorized and answering (`adb devices` says `device`).
    Ready,
    /// The phone hasn't accepted this Mac's RSA key yet: unlock it and tap
    /// "Allow" on the USB debugging prompt.
    Unauthorized,
    /// adb sees it but it doesn't answer (just plugged in, or a stale link).
    Offline,
    /// The host can't open the USB device (Linux udev rules).
    NoPermissions,
    /// Being connected or authorized (`connecting`, `authorizing`).
    Connecting,
    /// Booted into recovery, sideload, bootloader or rescue mode.
    #[default]
    Other,
}

impl PhoneState {
    /// Maps adb's state word(s) (`device`, `unauthorized`, `no permissions (…)`).
    pub fn from_adb(state: &str) -> Self {
        match state.split_whitespace().next().unwrap_or_default() {
            "device" => Self::Ready,
            "unauthorized" => Self::Unauthorized,
            "offline" => Self::Offline,
            "no" if state.starts_with("no permissions") => Self::NoPermissions,
            "connecting" | "authorizing" => Self::Connecting,
            _ => Self::Other,
        }
    }

    /// What to do about a phone that isn't ready, for every client to show.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Unauthorized => Some(
                "Unlock the phone and tap Allow on the \"Allow USB debugging?\" prompt. No prompt? Unplug it and plug it in again.",
            ),
            Self::Offline => Some("The phone doesn't answer adb. Unplug it and plug it in again."),
            Self::NoPermissions => Some(
                "This computer can't open the phone's USB device. On Linux, add a udev rule for it.",
            ),
            Self::Connecting => Some("Connecting to the phone…"),
            Self::Other => Some("The phone is in recovery, bootloader or sideload mode."),
        }
    }
}

/// How the phone is attached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhoneConnection {
    #[default]
    Usb,
    /// Wireless debugging (`ip:port` or an mDNS service name).
    Wifi,
}

/// `dumpsys battery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PhoneBattery {
    /// 0–100.
    pub level: u8,
    pub charging: bool,
}

/// `wm size` and `wm density`, override applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PhoneScreen {
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<u32>,
}

/// A physical phone adb sees. Fields other than `serial`, `state` and
/// `connection` are filled only when the phone is `ready` (adb can't ask an
/// unauthorized phone anything), except `model` and `product`, which
/// `adb devices -l` may report before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PhysicalDevice {
    /// The adb serial: what every `physical.*` command takes as `device`.
    pub serial: String,
    pub state: PhoneState,
    pub connection: PhoneConnection,
    /// e.g. `Pixel 8` (`ro.product.model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// e.g. `Google` (`ro.product.manufacturer`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// Product name, e.g. `shiba`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// Android release, e.g. `14` (`ro.build.version.release`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android: Option<String>,
    /// API level (`ro.build.version.sdk`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<u32>,
    /// CPU ABI, e.g. `arm64-v8a`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<PhoneBattery>,
    /// Only from `physical.details`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<PhoneScreen>,
    /// The USB port adb reports, e.g. `1-1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb: Option<String>,
    /// What to do when the phone isn't `ready`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl PhysicalDevice {
    /// The name people know the phone by: its model, else its serial.
    pub fn display_name(&self) -> &str {
        self.model.as_deref().unwrap_or(&self.serial)
    }
}

/// `physical.connect_info`: how a client reaches a ready phone directly
/// (the GUI's screen view and the MCP tools run `adb` themselves, as the
/// emulator player streams from the emulator, ADR-0001).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PhoneConnectInfo {
    pub serial: String,
    pub adb_path: std::path::PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adb_states_map_to_phone_states() {
        assert_eq!(PhoneState::from_adb("device"), PhoneState::Ready);
        assert_eq!(
            PhoneState::from_adb("unauthorized"),
            PhoneState::Unauthorized
        );
        assert_eq!(PhoneState::from_adb("offline"), PhoneState::Offline);
        assert_eq!(
            PhoneState::from_adb("no permissions (missing udev rules?); see [http://x]"),
            PhoneState::NoPermissions
        );
        assert_eq!(PhoneState::from_adb("authorizing"), PhoneState::Connecting);
        assert_eq!(PhoneState::from_adb("recovery"), PhoneState::Other);
        assert!(PhoneState::Ready.hint().is_none());
        assert!(
            PhoneState::Unauthorized
                .hint()
                .is_some_and(|h| h.contains("Allow"))
        );
    }

    #[test]
    fn wire_names_are_snake_case() {
        assert_eq!(
            serde_json::to_value(PhoneState::NoPermissions).unwrap(),
            "no_permissions"
        );
        let p = PhysicalDevice {
            serial: "R5CT1".into(),
            state: PhoneState::Unauthorized,
            connection: PhoneConnection::Usb,
            model: None,
            manufacturer: None,
            product: None,
            android: None,
            api: None,
            abi: None,
            battery: None,
            screen: None,
            usb: Some("1-1".into()),
            hint: None,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["state"], "unauthorized");
        assert!(v.get("model").is_none());
        assert_eq!(p.display_name(), "R5CT1");
    }
}
