//! The command provider contract. Every action Jidr can take is a command
//! registered by a provider (the daemon core, `jidr-widgets`, `jidr-root`);
//! the GUI, CLI and MCP all call the same commands.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{Cancel, ConnectInfo, Device, JidrError, JidrPaths, JidrResult};

/// One command row, as listed by `commands.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    /// Method name, e.g. `device.start`.
    pub id: String,
    /// One-line description for humans and agents.
    pub label: String,
    /// JSON Schema of the params object.
    pub params: Value,
    /// Loses data or can't be undone (MCP policy, D5).
    pub destructive: bool,
    /// Runs as a job: the call returns `{job, pending: true}` (4b).
    pub long_running: bool,
    /// Takes the per-device single-writer lock on `params.device`.
    pub locks_device: bool,
}

impl CommandSpec {
    /// Builds a spec whose params schema comes from `P`.
    pub fn new<P: JsonSchema>(id: &str, label: &str) -> Self {
        let mut params = serde_json::to_value(schemars::schema_for!(P)).unwrap_or(Value::Null);
        // Drop schema noise that agents don't need.
        if let Some(obj) = params.as_object_mut() {
            obj.remove("$schema");
            obj.remove("title");
        }
        Self {
            id: id.to_string(),
            label: label.to_string(),
            params,
            destructive: false,
            long_running: false,
            locks_device: false,
        }
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn long_running(mut self) -> Self {
        self.long_running = true;
        self
    }

    pub fn locks_device(mut self) -> Self {
        self.locks_device = true;
        self
    }
}

/// Params of commands that take nothing.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// Deserializes params; unknown keys and type errors become `usage` errors
/// that name the offending key. `null` counts as `{}`.
pub fn parse_params<T: DeserializeOwned>(params: Value) -> JidrResult<T> {
    let params = if params.is_null() {
        Value::Object(Default::default())
    } else {
        params
    };
    serde_json::from_value(params).map_err(|e| JidrError::usage(format!("invalid params: {e}")))
}

/// A change to a stored device (explicit `for<'a>` so async-trait keeps it
/// higher-ranked).
pub type DeviceUpdate = Box<dyn for<'a> FnOnce(&'a mut Device) + Send + 'static>;

/// Reports job progress: fraction (`None` = indeterminate) and a message.
pub type ProgressFn = Arc<dyn Fn(Option<f32>, &str) + Send + Sync>;

/// What a provider gets for each call.
#[derive(Clone)]
pub struct CallCtx {
    pub daemon: Arc<dyn DaemonContext>,
    pub progress: ProgressFn,
    pub cancel: Cancel,
}

impl std::fmt::Debug for CallCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallCtx")
            .field("cancel", &self.cancel)
            .finish_non_exhaustive()
    }
}

impl CallCtx {
    pub fn new(daemon: Arc<dyn DaemonContext>) -> Self {
        Self {
            daemon,
            progress: Arc::new(|_, _| {}),
            cancel: Cancel::new(),
        }
    }

    pub fn report(&self, fraction: Option<f32>, message: &str) {
        (self.progress)(fraction, message);
    }
}

/// Daemon services available to providers.
#[async_trait]
pub trait DaemonContext: Send + Sync {
    fn paths(&self) -> &JidrPaths;

    /// The `adb` binary of the SDK in use.
    fn adb_path(&self) -> JidrResult<PathBuf>;

    /// A device by name or id.
    async fn device(&self, selector: &str) -> JidrResult<Device>;

    async fn devices(&self) -> Vec<Device>;

    /// Connection details of a running device (`not_found` if stopped).
    async fn connect_info(&self, device: Uuid) -> JidrResult<ConnectInfo>;

    /// Applies `f` to the stored device and saves it.
    async fn update_device(&self, device: Uuid, f: DeviceUpdate) -> JidrResult<Device>;

    /// Starts a device and waits for boot. Callers already hold its lock.
    async fn start_device(
        &self,
        device: Uuid,
        cold_boot: bool,
        cancel: &Cancel,
    ) -> JidrResult<Device>;

    /// Stops a device. Callers already hold its lock.
    async fn stop_device(&self, device: Uuid) -> JidrResult<Device>;

    /// Publishes an event to `events.subscribe` listeners.
    fn emit(&self, topic: &str, data: Value);
}

/// A set of commands.
#[async_trait]
pub trait CommandProvider: Send + Sync {
    /// Short provider name for logs, e.g. `core`, `widgets`, `root`.
    fn name(&self) -> &str;

    fn commands(&self) -> Vec<CommandSpec>;

    /// Runs `method` (one of this provider's ids) with raw params.
    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value>;

    /// Called after every successful boot of a device, in registration order.
    async fn on_booted(&self, _ctx: &CallCtx, _device: &Device) -> JidrResult<()> {
        Ok(())
    }
}

/// `{ "device": "<name or id>" }`, the most common params shape.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceParams {
    /// Device name or id.
    pub device: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct P {
        /// Device name or id.
        device: String,
        #[serde(default)]
        cold_boot: bool,
    }

    #[test]
    fn spec_schema_lists_params() {
        let s = CommandSpec::new::<P>("device.start", "Start a device")
            .long_running()
            .locks_device();
        assert_eq!(s.params["properties"]["device"]["type"], "string");
        assert_eq!(s.params["additionalProperties"], false);
        assert_eq!(s.params["required"][0], "device");
        let wire = serde_json::to_value(&s).unwrap();
        assert_eq!(wire["longRunning"], true);
        assert_eq!(wire["locksDevice"], true);
        assert!(wire["params"].get("$schema").is_none());
    }

    #[test]
    fn unknown_keys_are_usage_errors_naming_the_key() {
        let err =
            parse_params::<P>(serde_json::json!({"device": "a", "colld_boot": true})).unwrap_err();
        assert_eq!(err.code, crate::ErrorCode::Usage);
        assert!(err.message.contains("colld_boot"), "{err}");
        let err = parse_params::<P>(serde_json::json!({})).unwrap_err();
        assert!(err.message.contains("device"), "{err}");
        assert!(parse_params::<NoParams>(Value::Null).is_ok());
    }
}
