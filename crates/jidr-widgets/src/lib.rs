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
pub mod battery;
pub mod device;
pub mod devtools;
pub mod extras;
pub mod gps;
pub mod media;
pub mod network;
pub mod phone;
pub mod sensors;
pub mod snapshots;

/// The widgets provider, ready to register with the daemon.
pub fn provider() -> Arc<dyn CommandProvider> {
    Arc::new(WidgetsProvider::default())
}

#[derive(Debug, Default)]
pub struct WidgetsProvider {
    routes: gps::Routes,
    recordings: media::Recordings,
}

#[async_trait]
impl CommandProvider for WidgetsProvider {
    fn name(&self) -> &str {
        "widgets"
    }

    fn commands(&self) -> Vec<CommandSpec> {
        let mut v = Vec::new();
        v.extend(apps::specs());
        v.extend(battery::specs());
        v.extend(sensors::specs());
        v.extend(network::specs());
        v.extend(phone::specs());
        v.extend(devtools::specs());
        v.extend(extras::specs());
        v.extend(gps::specs());
        v.extend(media::specs());
        v.extend(snapshots::specs());
        v
    }

    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
        match method.split('.').next().unwrap_or_default() {
            "app" => apps::call(ctx, method, params).await,
            "battery" => battery::call(ctx, method, params).await,
            "sensor" => sensors::call(ctx, method, params).await,
            "network" => network::call(ctx, method, params).await,
            "phone" => phone::call(ctx, method, params).await,
            "key" | "dev" | "clipboard" => devtools::call(ctx, method, params).await,
            "fingerprint" | "camera" => extras::call(ctx, method, params).await,
            "gps" => gps::call(ctx, &self.routes, method, params).await,
            "media" => media::call(ctx, &self.recordings, method, params).await,
            "snapshot" => snapshots::call(ctx, method, params).await,
            _ => Err(JidrError::usage(format!("unknown method {method}"))),
        }
    }
}
