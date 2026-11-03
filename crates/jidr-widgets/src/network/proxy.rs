//! The device's global HTTP proxy, for Burp or mitmproxy on the Mac.
//!
//! `settings put global http_proxy host:port` is what Android's own
//! `ConnectivityManager` reads; it takes effect at once on API 29–36, holds
//! for every network (Wi-Fi and mobile) and survives reboots and snapshots.
//! The emulator's `-http-proxy` is launch-only, and neither the console nor
//! gRPC can change a proxy on a running device.
//!
//! Clearing writes `:0` instead of deleting the setting: Android only
//! notices a deleted `http_proxy` after a reboot.

use jidr_adb::Adb;
use jidr_core::{JidrError, JidrResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The host's loopback as the emulator sees it.
pub const HOST_LOOPBACK: &str = "10.0.2.2";

/// The settings Android's own proxy screens write next to `http_proxy`;
/// cleared too so nothing brings an old proxy back.
const UI_KEYS: [&str; 3] = [
    "global_http_proxy_host",
    "global_http_proxy_port",
    "global_http_proxy_exclusion_list",
];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    pub device: String,
    /// Hostname or IPv4 address as the device sees it: `10.0.2.2` is this
    /// Mac, `127.0.0.1` the device itself (with `adb reverse`).
    pub host: String,
    pub port: u16,
}

/// `host:port` from `settings get global http_proxy`; `None` for no proxy
/// (`null`, empty, or the `:0` that clearing writes).
pub fn parse(setting: &str) -> Option<(String, u16)> {
    let (host, port) = setting.trim().rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    (!host.is_empty() && port != 0).then(|| (host.to_string(), port))
}

/// A host safe to put in `settings put` (it goes through `adb shell`):
/// letters, digits, dots and dashes, as in a hostname or an IPv4 address.
pub fn check_host(host: &str) -> JidrResult<&str> {
    let host = host.trim();
    let valid = !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['-', '.'])
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if valid {
        Ok(host)
    } else {
        Err(JidrError::usage(format!(
            "host: {host:?} isn't a hostname or IPv4 address (use {HOST_LOOPBACK} for this Mac)"
        ))
        .with_data(json!({ "field": "host" })))
    }
}

fn result(proxy: Option<(String, u16)>) -> Value {
    match proxy {
        Some((host, port)) => {
            json!({ "proxy": format!("{host}:{port}"), "host": host, "port": port })
        }
        None => json!({ "proxy": null, "host": null, "port": null }),
    }
}

async fn read(adb: &Adb) -> JidrResult<Option<(String, u16)>> {
    Ok(parse(&adb.shell("settings get global http_proxy").await?))
}

pub async fn get(adb: &Adb) -> JidrResult<Value> {
    Ok(result(read(adb).await?))
}

/// Checks a proxy before it is set; returns the trimmed host.
pub fn check(host: &str, port: u16) -> JidrResult<&str> {
    let host = check_host(host)?;
    if port == 0 {
        return Err(JidrError::usage("port: must be 1–65535").with_data(json!({ "field": "port" })));
    }
    Ok(host)
}

pub async fn set(adb: &Adb, host: &str, port: u16) -> JidrResult<Value> {
    let host = check(host, port)?;
    adb.shell(&format!("settings put global http_proxy {host}:{port}"))
        .await?;
    let now = read(adb).await?;
    if now.as_ref() != Some(&(host.to_string(), port)) {
        return Err(JidrError::unexpected(format!(
            "the device didn't take the proxy {host}:{port}: http_proxy is {now:?}"
        )));
    }
    Ok(result(now))
}

pub async fn clear(adb: &Adb) -> JidrResult<Value> {
    let mut cmd = "settings put global http_proxy :0".to_string();
    for key in UI_KEYS {
        cmd.push_str(&format!("; settings delete global {key} >/dev/null"));
    }
    adb.shell(&cmd).await?;
    let now = read(adb).await?;
    if let Some((host, port)) = now {
        return Err(JidrError::unexpected(format!(
            "the device still has the proxy {host}:{port}"
        )));
    }
    Ok(result(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_setting() {
        assert_eq!(parse("10.0.2.2:8080\n"), Some(("10.0.2.2".into(), 8080)));
        assert_eq!(parse("proxy.lan:3128"), Some(("proxy.lan".into(), 3128)));
        assert_eq!(parse(":0"), None, "what clear writes");
        assert_eq!(parse("null"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("host:"), None);
        assert_eq!(parse("host:0"), None);
        assert_eq!(parse("host:99999"), None);
    }

    #[test]
    fn hosts_are_safe_for_the_shell() {
        assert_eq!(check_host(" 10.0.2.2 ").unwrap(), "10.0.2.2");
        assert!(check_host("burp-box.local").is_ok());
        for bad in ["", "a b", "x;reboot", "$(id)", "h:80", "-x", "::1", "a'b"] {
            let e = check_host(bad).unwrap_err();
            assert_eq!(e.data.unwrap()["field"], "host", "{bad}");
        }
    }

    #[test]
    fn port_zero_is_refused() {
        assert_eq!(
            check("10.0.2.2", 0).unwrap_err().data.unwrap()["field"],
            "port"
        );
        assert_eq!(check("10.0.2.2", 8080).unwrap(), "10.0.2.2");
    }

    #[test]
    fn results_name_host_and_port() {
        let v = result(Some(("10.0.2.2".into(), 8080)));
        assert_eq!(v["proxy"], "10.0.2.2:8080");
        assert_eq!(v["port"], 8080);
        assert!(result(None)["proxy"].is_null());
    }
}
