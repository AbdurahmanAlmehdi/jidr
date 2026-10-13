//! JSON-RPC 2.0 over newline-delimited JSON on a Unix socket.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::Daemon;
use crate::rpc::{
    self, HelloParams, HelloResult, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, Request,
    Response, RpcError,
};

/// Longest request line we accept (APK paths, not APK bytes, go over the wire).
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// macOS limits Unix socket paths to 104 bytes.
const MAX_SOCKET_PATH: usize = 100;

/// Binds the daemon socket (mode 0600), replacing a stale one. Fails with
/// `busy` if another daemon is listening.
pub fn bind(path: &Path) -> JidrResult<UnixListener> {
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(JidrError::usage(format!(
            "socket path is too long for a Unix socket ({} bytes): {}",
            path.as_os_str().len(),
            path.display()
        )));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .ctx(ErrorCode::Unexpected, format!("creating {}", dir.display()))?;
    }
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(JidrError::busy(format!(
                "another Jidr daemon is listening on {}",
                path.display()
            )));
        }
        std::fs::remove_file(path).ctx(ErrorCode::Unexpected, "removing stale socket")?;
    }
    let listener = UnixListener::bind(path)
        .ctx(ErrorCode::Unexpected, format!("binding {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .ctx(ErrorCode::Unexpected, "setting socket permissions")?;
    }
    Ok(listener)
}

fn line(v: &impl serde::Serialize) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"encoding failed"}}"#.into()
    })
}

pub(crate) async fn handle_connection(daemon: Arc<Daemon>, stream: UnixStream) {
    let (r, mut w) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(mut l) = rx.recv().await {
            l.push('\n');
            if w.write_all(l.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    daemon.client_connected();
    let hello = Arc::new(AtomicBool::new(false));
    let mut reader = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = match tokio::io::AsyncReadExt::take(&mut reader, MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
            .await
        {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        if buf.len() > MAX_LINE {
            let _ = tx.send(line(&Response::err(
                Value::Null,
                RpcError::new(INVALID_REQUEST, "request line too long"),
            )));
            break;
        }
        let text = String::from_utf8_lossy(&buf);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(line(&Response::err(
                    Value::Null,
                    RpcError::new(PARSE_ERROR, format!("parse error: {e}")),
                )));
                continue;
            }
        };
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let req = match serde_json::from_value::<Request>(value) {
            Ok(r) if r.jsonrpc == "2.0" => r,
            Ok(_) => {
                let _ = tx.send(line(&Response::err(
                    id,
                    RpcError::new(INVALID_REQUEST, "jsonrpc must be \"2.0\""),
                )));
                continue;
            }
            Err(e) => {
                let _ = tx.send(line(&Response::err(
                    id,
                    RpcError::new(INVALID_REQUEST, format!("invalid request: {e}")),
                )));
                continue;
            }
        };
        let Some(id) = req.id.clone() else {
            // Notifications from clients have no meaning here.
            continue;
        };
        if req.method == "hello" {
            let resp = match daemon.hello(req.params) {
                Ok(r) => {
                    hello.store(true, Ordering::Relaxed);
                    Response::ok(id, r)
                }
                Err(e) => Response::err(id, RpcError::from_jidr(&e)),
            };
            let _ = tx.send(line(&resp));
            continue;
        }
        if !hello.load(Ordering::Relaxed) {
            let _ = tx.send(line(&Response::err(
                id,
                RpcError::new(INVALID_REQUEST, "call hello first"),
            )));
            continue;
        }
        let (daemon, tx) = (daemon.clone(), tx.clone());
        tokio::spawn(async move {
            let resp = daemon.dispatch(id, req.method, req.params, &tx).await;
            if let Some(resp) = resp {
                let _ = tx.send(line(&resp));
            }
        });
    }
    drop(tx);
    let _ = writer.await;
    daemon.client_disconnected();
}

impl Daemon {
    fn hello(&self, params: Value) -> JidrResult<Value> {
        let p: HelloParams = jidr_core::provider::parse_params(params)?;
        if p.version != self.version {
            return Err(JidrError::new(
                ErrorCode::DaemonStale,
                format!(
                    "the running daemon is {} but this client is {}; restart the daemon",
                    self.version, p.version
                ),
            )
            .with_data(json!({ "daemon_version": self.version, "client_version": p.version })));
        }
        serde_json::to_value(HelloResult {
            version: self.version.clone(),
            protocol: rpc::PROTOCOL_VERSION.into(),
            pid: std::process::id(),
        })
        .map_err(|e| JidrError::unexpected(e.to_string()))
    }

    /// Runs one request. `None` means the response is sent elsewhere.
    pub(crate) async fn dispatch(
        &self,
        id: Value,
        method: String,
        params: Value,
        _tx: &mpsc::UnboundedSender<String>,
    ) -> Option<Response> {
        if method == "commands.list" {
            return Some(Response::ok(
                id,
                serde_json::to_value(self.registry.list()).unwrap_or(Value::Null),
            ));
        }
        let Some((provider, spec)) = self.registry.get(&method) else {
            return Some(Response::err(
                id,
                RpcError::new(METHOD_NOT_FOUND, format!("unknown method {method}")),
            ));
        };
        let result = async {
            crate::validate::validate(&spec.params, &params)?;
            let _guard = if spec.locks_device {
                let selector = params
                    .get("device")
                    .and_then(Value::as_str)
                    .ok_or_else(|| JidrError::usage("invalid params: missing field `device`"))?;
                let device = self.state.device(selector).await?;
                Some(
                    self.state
                        .locks
                        .try_lock(device.id, &device.name, &method, None)?,
                )
            } else {
                None
            };
            let ctx = CallCtx::new(self.state.clone() as Arc<dyn DaemonContext>);
            provider.call(&ctx, &method, params).await
        }
        .await;
        Some(match result {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(id, RpcError::from_jidr(&e)),
        })
    }
}
