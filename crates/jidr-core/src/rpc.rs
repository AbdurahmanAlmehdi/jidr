//! JSON-RPC 2.0 message types (newline-delimited JSON over the socket).

use crate::{ErrorCode, JidrError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// Protocol version. Bumped on any breaking change; see docs/control-protocol.md.
pub const PROTOCOL_VERSION: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// Absent for notifications (which we don't accept from clients).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// A Jidr error: the JSON-RPC code from its `ErrorCode`, and
    /// `data.code` = the snake_case name (`busy`, `not_found`, …).
    pub fn from_jidr(e: &JidrError) -> Self {
        let mut data = json!({ "code": e.code.as_str(), "exit_code": e.code.exit_code() });
        if let (Some(extra), Some(obj)) = (&e.data, data.as_object_mut()) {
            obj.insert("details".into(), extra.clone());
        }
        Self {
            code: e.code.rpc_code(),
            message: e.message.clone(),
            data: Some(data),
        }
    }

    /// Back to a `JidrError` on the client side.
    pub fn to_jidr(&self) -> JidrError {
        let code = self
            .data
            .as_ref()
            .and_then(|d| d.get("code"))
            .and_then(|c| serde_json::from_value::<ErrorCode>(c.clone()).ok())
            .unwrap_or(match self.code {
                PARSE_ERROR | INVALID_REQUEST | METHOD_NOT_FOUND | INVALID_PARAMS => {
                    ErrorCode::Usage
                }
                _ => ErrorCode::Unexpected,
            });
        let mut err = JidrError::new(code, self.message.clone());
        err.data = self.data.as_ref().and_then(|d| d.get("details")).cloned();
        err
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A server → client notification (events).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

/// An event published to `events.subscribe` listeners, sent as an `event`
/// notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// e.g. `device.status`, `device.boot`, `job.progress`, `logcat`.
    pub topic: String,
    pub data: Value,
}

/// The notification method carrying events.
pub const EVENT_METHOD: &str = "event";

/// `hello` params and result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    /// The client's Jidr version; must equal the daemon's.
    pub version: String,
    /// `cli`, `gui`, `mcp`, …
    #[serde(default)]
    pub client: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HelloResult {
    pub version: String,
    pub protocol: String,
    pub pid: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jidr_errors_round_trip() {
        let e = JidrError::busy("held").with_data(json!({"holder": "device.start", "op": "start"}));
        let r = RpcError::from_jidr(&e);
        assert_eq!(r.code, -32004);
        assert_eq!(r.data.as_ref().unwrap()["code"], "busy");
        assert_eq!(r.data.as_ref().unwrap()["exit_code"], 4);
        let back = r.to_jidr();
        assert_eq!(back, e);
        assert_eq!(
            RpcError::new(PARSE_ERROR, "bad").to_jidr().code,
            ErrorCode::Usage
        );
    }
}
