//! Error codes shared by the protocol, the CLI (exit codes) and MCP.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The eight error codes. Each one is also the CLI exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// A bug or an error we didn't classify. Exit code 1.
    Unexpected,
    /// Bad arguments or parameters. Exit code 2.
    Usage,
    /// The device, image, profile, job or snapshot doesn't exist. Exit code 3.
    NotFound,
    /// Another operation holds the device lock. Exit code 4.
    Busy,
    /// The device didn't finish booting in time. Exit code 5.
    BootTimeout,
    /// SDK, emulator, download or license problem. Exit code 6.
    Sdk,
    /// Rooting, verifying or reverting root failed. Exit code 7.
    RootFailed,
    /// The client and the running daemon are different versions. Exit code 8.
    DaemonStale,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 8] = [
        Self::Unexpected,
        Self::Usage,
        Self::NotFound,
        Self::Busy,
        Self::BootTimeout,
        Self::Sdk,
        Self::RootFailed,
        Self::DaemonStale,
    ];

    /// The CLI exit code (0 is success and has no `ErrorCode`).
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Unexpected => 1,
            Self::Usage => 2,
            Self::NotFound => 3,
            Self::Busy => 4,
            Self::BootTimeout => 5,
            Self::Sdk => 6,
            Self::RootFailed => 7,
            Self::DaemonStale => 8,
        }
    }

    /// The JSON-RPC error code. Usage maps to "invalid params"; the others
    /// live in the implementation-defined server-error range.
    pub const fn rpc_code(self) -> i64 {
        match self {
            Self::Usage => -32602,
            other => -32000 - other.exit_code() as i64,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unexpected => "unexpected",
            Self::Usage => "usage",
            Self::NotFound => "not_found",
            Self::Busy => "busy",
            Self::BootTimeout => "boot_timeout",
            Self::Sdk => "sdk",
            Self::RootFailed => "root_failed",
            Self::DaemonStale => "daemon_stale",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error type every Jidr layer returns across crate boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{code}: {message}")]
pub struct JidrError {
    pub code: ErrorCode,
    pub message: String,
    /// Structured details, e.g. `{"holder": "...", "op": "..."}` for `busy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

pub type JidrResult<T> = Result<T, JidrError>;

impl JidrError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn unexpected(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unexpected, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Usage, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Busy, message)
    }

    pub fn sdk(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Sdk, message)
    }

    pub fn root_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::RootFailed, message)
    }

    pub fn boot_timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BootTimeout, message)
    }
}

/// Adds context to foreign errors: `.ctx(ErrorCode::Sdk, "reading config.ini")`.
pub trait ResultExt<T> {
    fn ctx(self, code: ErrorCode, what: impl std::fmt::Display) -> JidrResult<T>;
}

impl<T, E: std::fmt::Display> ResultExt<T> for Result<T, E> {
    fn ctx(self, code: ErrorCode, what: impl std::fmt::Display) -> JidrResult<T> {
        self.map_err(|e| JidrError::new(code, format!("{what}: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_plan() {
        let codes: Vec<u8> = ErrorCode::ALL.iter().map(|c| c.exit_code()).collect();
        assert_eq!(codes, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn serializes_snake_case() {
        let e = JidrError::busy("held").with_data(serde_json::json!({"holder": "x"}));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "busy");
        assert_eq!(v["data"]["holder"], "x");
        let back: JidrError = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn rpc_codes_are_distinct() {
        let mut codes: Vec<i64> = ErrorCode::ALL.iter().map(|c| c.rpc_code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 8);
        assert_eq!(ErrorCode::NotFound.rpc_code(), -32003);
    }
}
