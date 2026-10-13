//! Authenticated gRPC clients for a running emulator.

use std::time::Duration;

use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};
use jidr_proto::{BearerToken, Controller, Snapshots};
use tonic::transport::Endpoint;

use crate::InstanceInfo;

/// Connects to `127.0.0.1:<port>` with the bearer token.
pub async fn connect_port(port: u16, token: &str) -> JidrResult<(Controller, Snapshots)> {
    let token = BearerToken::new(token).ctx(ErrorCode::Unexpected, "invalid gRPC token")?;
    let channel = Endpoint::from_shared(format!("http://127.0.0.1:{port}"))
        .ctx(ErrorCode::Unexpected, "gRPC endpoint")?
        .connect_timeout(Duration::from_secs(5))
        .tcp_nodelay(true)
        .connect()
        .await
        .map_err(|e| JidrError::sdk(format!("cannot reach emulator gRPC on port {port}: {e}")))?;
    Ok(jidr_proto::clients(channel, token))
}

pub async fn connect(instance: &InstanceInfo) -> JidrResult<(Controller, Snapshots)> {
    connect_port(instance.grpc_port, &instance.grpc_token).await
}

/// Maps a gRPC status to a Jidr error.
pub fn status_error(what: &str, s: tonic::Status) -> JidrError {
    let code = match s.code() {
        tonic::Code::NotFound => ErrorCode::NotFound,
        tonic::Code::InvalidArgument => ErrorCode::Usage,
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => ErrorCode::Sdk,
        _ => ErrorCode::Sdk,
    };
    JidrError::new(code, format!("{what}: {}", s.message()))
        .with_data(serde_json::json!({"grpc_code": format!("{:?}", s.code())}))
}
