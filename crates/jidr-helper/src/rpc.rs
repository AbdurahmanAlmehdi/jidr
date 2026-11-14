//! The wire protocol between the daemon and the helper (ADR-0003).
//!
//! One JSON object per line (NDJSON) each way, over the abstract Unix socket
//! [`SOCKET`] on the guest, which the daemon reaches through `adb forward`:
//!
//! ```text
//! → {"id":1,"method":"net.shaping.set","params":{"loss_pct":10}}
//! ← {"id":1,"result":{"interfaces":[{"name":"wlan0","qdisc":"netem","loss_pct":10.0}]}}
//! ← {"id":2,"error":{"code":"unsupported","message":"…"}}
//! ```
//!
//! `ping` and `shutdown` never change shape, so a daemon can always tell an
//! old helper apart and replace it. Everything else is versioned by
//! [`PROTOCOL`].

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped on any breaking change to a method's params or result. The daemon
/// replaces a helper whose `ping` reports another protocol.
pub const PROTOCOL: u32 = 1;

/// The helper's abstract socket name (`@jidr-helper`; no file, no port).
pub const SOCKET: &str = "jidr-helper";

/// Where the daemon pushes the binary on the guest.
pub const DEVICE_PATH: &str = "/data/local/tmp/jidr-helper";

/// The uids allowed to connect: root, and shell (adbd, which carries every
/// `adb forward` connection). Apps (uid ≥ 10000) are refused.
pub const ALLOWED_UIDS: [u32; 2] = [0, 2000];

/// A request line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

/// A response line: exactly one of `result` and `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, error: Error) -> Self {
        Self {
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// What went wrong, for the daemon to map onto its own error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Bad params, or a method this helper doesn't know.
    Usage,
    /// The guest can't do it (a kernel without netem, a missing tool).
    Unsupported,
    /// A tool the helper ran failed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Usage, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Failed, message)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = serde_json::to_value(self.code)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        write!(f, "{code}: {}", self.message)
    }
}

/// A typed call: its method name, params and result type.
pub trait Method: Serialize {
    const NAME: &'static str;
    type Output: DeserializeOwned + Serialize;
}

/// `ping`: who and what the helper is. Frozen shape (see the module docs).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ping {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pong {
    /// The helper's crate version (the Jidr version it was built with).
    pub version: String,
    /// Its [`PROTOCOL`].
    pub protocol: u32,
    /// SHA-256 of its own executable, so the daemon knows it runs the exact
    /// build it ships.
    pub sha256: String,
    pub pid: u32,
    /// Its uid: 0 when it was started as root.
    pub uid: u32,
    /// Its SELinux context (`u:r:su:s0` under `adb root`, `u:r:magisk:s0`
    /// under Magisk), when the guest reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selinux: Option<String>,
    /// The CPU architecture it was built for (`aarch64`, `x86_64`).
    pub arch: String,
}

impl Method for Ping {
    const NAME: &'static str = "ping";
    type Output = Pong;
}

/// `shutdown`: the helper answers, then exits. Frozen shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Shutdown {}

impl Method for Shutdown {
    const NAME: &'static str = "shutdown";
    type Output = Value;
}

/// `net.shaping.get`: the shaping on each interface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapingGet {
    /// Interfaces to read; default: every interface with a default route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interfaces: Option<Vec<String>>,
}

impl Method for ShapingGet {
    const NAME: &'static str = "net.shaping.get";
    type Output = Shaping;
}

/// `net.shaping.set`: drop `loss_pct` % of outgoing packets (netem); 0 clears
/// it. Later shaping fields (#17) are added here, defaulting to off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapingSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interfaces: Option<Vec<String>>,
    pub loss_pct: f64,
}

impl Method for ShapingSet {
    const NAME: &'static str = "net.shaping.set";
    type Output = Shaping;
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Shaping {
    pub interfaces: Vec<InterfaceShaping>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterfaceShaping {
    pub name: String,
    /// The root qdisc's kind (`netem`, `mq`, `pfifo_fast`…), when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qdisc: Option<String>,
    /// Packet loss in percent; 0 without netem.
    pub loss_pct: f64,
}

/// `identity.android_id.set`: the Android ID as every installed app reads
/// it (its SSAID). Restarts the Android framework.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AndroidIdSet {
    /// 16 hexadecimal digits.
    pub android_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AndroidIdSetResult {
    /// The ID as set (lower case).
    pub android_id: String,
    /// How many app uids now read it.
    pub apps: u32,
}

impl Method for AndroidIdSet {
    const NAME: &'static str = "identity.android_id.set";
    type Output = AndroidIdSetResult;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lines_have_the_documented_shape() {
        let req = Request {
            id: 1,
            method: ShapingSet::NAME.into(),
            params: serde_json::to_value(ShapingSet {
                interfaces: None,
                loss_pct: 10.0,
            })
            .unwrap(),
        };
        assert_eq!(
            serde_json::to_value(&req).unwrap(),
            json!({"id": 1, "method": "net.shaping.set", "params": {"loss_pct": 10.0}})
        );
        let ping: Request = serde_json::from_str(r#"{"id":7,"method":"ping"}"#).unwrap();
        assert_eq!(ping.params, Value::Null);
        let err = Response::err(2, Error::unsupported("no netem"));
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({"id": 2, "error": {"code": "unsupported", "message": "no netem"}})
        );
        assert_eq!(err.error.unwrap().to_string(), "unsupported: no netem");
    }
}
