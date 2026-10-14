//! `CommandProvider` for sensors, phone, network, dev tools, screenshots,
//! recording and snapshots (plan Phases 8a, 8b, 10a, 10b). Registered in
//! the daemon by Stream A through [`provider`] (gap A1). Apache-2.0, no Slint.

use std::sync::Arc;

use async_trait::async_trait;
use jidr_core::provider::{CallCtx, CommandProvider, CommandSpec};
use jidr_core::{JidrError, JidrResult};
use serde_json::Value;

pub mod apk;
pub mod apps;
pub mod device;

/// The widgets provider, ready to register with the daemon.
pub fn provider() -> Arc<dyn CommandProvider> {
    Arc::new(WidgetsProvider::default())
}

#[derive(Debug, Default)]
pub struct WidgetsProvider {}

#[async_trait]
impl CommandProvider for WidgetsProvider {
    fn name(&self) -> &str {
        "widgets"
    }

    fn commands(&self) -> Vec<CommandSpec> {
        let mut v = Vec::new();
        v.extend(apps::specs());
        v
    }

    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
        match method.split('.').next().unwrap_or_default() {
            "app" => apps::call(ctx, method, params).await,
            _ => Err(JidrError::usage(format!("unknown method {method}"))),
        }
    }
}
