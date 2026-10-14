//! Network widget: Wi-Fi on/off (`svc wifi`), mobile data on/off (console
//! `gsm data`), speed and delay profiles (console `network speed/delay`).
//!
//! Mobile data is switched at the emulated radio, not with Android's own
//! switch: `svc data` / `cmd phone data` exit 0 and change nothing on the
//! API 35 images (CI run 38625267538), while the console works on every image.
//! Android's Quick Settings switch stays as it was; the device just has no
//! mobile data network, as when it loses coverage.

use jidr_backend::console::GsmLine;
use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{JidrError, JidrResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::device::target;

/// Named profiles (speed + latency pairs from the console's own tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// No throttling.
    Full,
    /// LTE speed, no added delay.
    Lte,
    /// HSDPA (3.5G).
    Hsdpa,
    /// UMTS (3G) speed and delay.
    Umts,
    /// EDGE (2.5G) speed and delay.
    Edge,
    /// GPRS speed, GSM delay.
    Gprs,
    /// GSM (2G) speed and delay.
    Gsm,
}

impl Profile {
    /// (console `network speed`, console `network delay`).
    pub fn console(self) -> (&'static str, &'static str) {
        match self {
            Profile::Full => ("full", "none"),
            Profile::Lte => ("lte", "none"),
            Profile::Hsdpa => ("hsdpa", "umts"),
            Profile::Umts => ("umts", "umts"),
            Profile::Edge => ("edge", "edge"),
            Profile::Gprs => ("gprs", "gsm"),
            Profile::Gsm => ("gsm", "gsm"),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    pub device: String,
    #[serde(default)]
    pub wifi: Option<bool>,
    /// Mobile data.
    #[serde(default)]
    pub mobile: Option<bool>,
    #[serde(default)]
    pub profile: Option<Profile>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<DeviceParams>("network.get", "Wi-Fi, mobile data and speed profile"),
        CommandSpec::new::<SetParams>(
            "network.set",
            "Turn Wi-Fi or mobile data on/off and set the speed/delay profile",
        ),
    ]
}

fn on_off(v: bool) -> &'static str {
    if v { "enable" } else { "disable" }
}

/// Whether the emulated radio has mobile data, from the console's
/// `gsm status` reply (`gsm data state:  home`): registered at home or roaming.
fn mobile_data_on(gsm_status: &[String]) -> Option<bool> {
    let state = gsm_status
        .iter()
        .find_map(|l| l.trim().strip_prefix("gsm data state:"))?
        .trim();
    Some(matches!(state, "home" | "roaming"))
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "network.get" => {
            let p: DeviceParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let adb = t.adb();
            let wifi = adb.shell("settings get global wifi_on").await?.trim() == "1";
            let gsm = t.console_cmd("gsm status").await?;
            let mobile = mobile_data_on(&gsm).ok_or_else(|| {
                JidrError::unexpected(format!("no data state in `gsm status`: {gsm:?}"))
            })?;
            let status = t.console_cmd("network status").await.unwrap_or_default();
            Ok(json!({ "wifi": wifi, "mobile": mobile, "status": status }))
        }
        "network.set" => {
            let p: SetParams = parse_params(params)?;
            if p.wifi.is_none() && p.mobile.is_none() && p.profile.is_none() {
                return Err(JidrError::usage("give wifi, mobile or profile"));
            }
            let t = target(ctx, &p.device).await?;
            let adb = t.adb();
            if let Some(w) = p.wifi {
                adb.shell(&format!("svc wifi {}", on_off(w))).await?;
            }
            if p.mobile.is_some() || p.profile.is_some() {
                let mut c = t.console().await?;
                if let Some(m) = p.mobile {
                    c.gsm_state(GsmLine::Data, if m { "home" } else { "off" })
                        .await?;
                }
                if let Some(pr) = p.profile {
                    let (speed, delay) = pr.console();
                    c.network_speed(speed).await?;
                    c.network_delay(delay).await?;
                }
            }
            Ok(json!({ "wifi": p.wifi, "mobile": p.mobile, "profile": p.profile }))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_map_to_console_tables() {
        assert_eq!(Profile::Edge.console(), ("edge", "edge"));
        assert_eq!(Profile::Full.console(), ("full", "none"));
        let p: SetParams = parse_params(json!({"device": "d", "profile": "umts"})).unwrap();
        assert_eq!(p.profile, Some(Profile::Umts));
    }

    #[test]
    fn mobile_data_follows_the_radio_data_state() {
        let status = |data: &str| {
            vec![
                "gsm voice state: home".to_string(),
                format!("gsm data state:  {data}"),
            ]
        };
        assert_eq!(mobile_data_on(&status("home")), Some(true));
        assert_eq!(mobile_data_on(&status("roaming")), Some(true));
        assert_eq!(mobile_data_on(&status("off")), Some(false));
        assert_eq!(mobile_data_on(&status("unregistered")), Some(false));
        assert_eq!(mobile_data_on(&["OK".to_string()]), None);
    }
}
