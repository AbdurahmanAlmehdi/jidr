//! A running device as the widgets see it: adb, gRPC and console handles.

use jidr_adb::Adb;
use jidr_backend::Console;
use jidr_core::provider::CallCtx;
use jidr_core::{ConnectInfo, Device, JidrError, JidrResult};
use jidr_proto::{Controller, Snapshots};

#[derive(Debug, Clone)]
pub struct Target {
    pub device: Device,
    pub info: ConnectInfo,
}

/// Resolves a running device (`not_found` when it's stopped).
pub async fn target(ctx: &CallCtx, selector: &str) -> JidrResult<Target> {
    let device = ctx.daemon.device(selector).await?;
    let info = ctx.daemon.connect_info(device.id).await.map_err(|e| {
        if e.code == jidr_core::ErrorCode::NotFound {
            JidrError::not_found(format!("{} isn't running; start it first", device.name))
        } else {
            e
        }
    })?;
    Ok(Target { device, info })
}

impl Target {
    pub fn adb(&self) -> Adb {
        Adb::new(self.info.adb_path.clone()).device(self.info.adb_serial.clone())
    }

    pub async fn grpc(&self) -> JidrResult<Controller> {
        Ok(
            jidr_backend::grpc::connect_port(self.info.grpc_port, &self.info.grpc_token)
                .await?
                .0,
        )
    }

    pub async fn snapshots(&self) -> JidrResult<Snapshots> {
        Ok(
            jidr_backend::grpc::connect_port(self.info.grpc_port, &self.info.grpc_token)
                .await?
                .1,
        )
    }

    pub async fn console(&self) -> JidrResult<Console> {
        let token = jidr_backend::console::default_token_path();
        Console::connect(self.info.console_port, token.as_deref()).await
    }

    /// Runs a console command and fails on `KO:` replies.
    pub async fn console_cmd(&self, cmd: &str) -> JidrResult<Vec<String>> {
        let mut c = self.console().await?;
        c.command(cmd).await
    }
}

/// Maps a gRPC failure to a Jidr error.
pub fn grpc_err(what: &str) -> impl FnOnce(tonic::Status) -> JidrError + '_ {
    move |s| jidr_backend::grpc::status_error(what, s)
}
