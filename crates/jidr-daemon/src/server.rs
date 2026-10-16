//! JSON-RPC 2.0 over newline-delimited JSON on a Unix socket.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jidr_adb::Adb;
use jidr_core::provider::{BytesFn, CallCtx, DaemonContext, ProgressFn, parse_params};
use jidr_core::rpc::Notification;
use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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
    let mut subscription: Option<JoinHandle<()>> = None;
    let mut closing = daemon.closing.subscribe();
    let mut streams: Vec<JoinHandle<()>> = Vec::new();
    let mut reader = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let mut limited = tokio::io::AsyncReadExt::take(&mut reader, MAX_LINE as u64 + 1);
        let read = limited.read_until(b'\n', &mut buf);
        let n = tokio::select! {
            r = read => match r {
                Ok(n) => n,
                Err(_) => break,
            },
            _ = closing.wait_for(|c| *c) => break,
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
        if req.method == "daemon.shutdown" {
            // Allowed before hello, so a newer client can replace a stale
            // daemon. Running emulators keep running and are re-adopted.
            let _ = tx.send(line(&Response::ok(id, json!({ "stopping": true }))));
            daemon.stop.notify_one();
            continue;
        }
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
        // Connection-scoped streams.
        let stream = match req.method.as_str() {
            "events.subscribe" => Some(daemon.subscribe(req.params.clone(), tx.clone())),
            "logcat.follow" => Some(daemon.logcat_follow(req.params.clone(), tx.clone()).await),
            _ => None,
        };
        if let Some(started) = stream {
            let resp = match started {
                Ok((result, task)) => {
                    if req.method == "events.subscribe" {
                        // A new subscription replaces the previous filter.
                        if let Some(old) = subscription.replace(task) {
                            old.abort();
                        }
                    } else {
                        streams.push(task);
                    }
                    Response::ok(id, result)
                }
                Err(e) => Response::err(id, RpcError::from_jidr(&e)),
            };
            let _ = tx.send(line(&resp));
            continue;
        }
        let (daemon, tx) = (daemon.clone(), tx.clone());
        tokio::spawn(async move {
            let resp = daemon.dispatch(id, req.method, req.params).await;
            let _ = tx.send(line(&resp));
        });
    }
    for t in streams.into_iter().chain(subscription) {
        t.abort();
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

    /// Runs one request.
    pub(crate) async fn dispatch(
        self: &Arc<Self>,
        id: Value,
        method: String,
        params: Value,
    ) -> Response {
        if method == "commands.list" {
            return Response::ok(
                id,
                serde_json::to_value(self.registry.list()).unwrap_or(Value::Null),
            );
        }
        let Some((provider, spec)) = self.registry.get(&method) else {
            return Response::err(
                id,
                RpcError::new(METHOD_NOT_FOUND, format!("unknown method {method}")),
            );
        };
        let (provider, spec) = (provider.clone(), spec.clone());
        let result: JidrResult<Value> = async {
            crate::validate::validate(&spec.params, &params)?;
            let (guard, device) = if spec.locks_device {
                let selector = params
                    .get("device")
                    .and_then(Value::as_str)
                    .ok_or_else(|| JidrError::usage("invalid params: missing field `device`"))?;
                let device = self.state.device(selector).await?;
                let guard = self
                    .state
                    .locks
                    .try_lock(device.id, &device.name, &method, None)?;
                (Some(guard), Some(device.id))
            } else {
                (None, None)
            };
            let daemon: Arc<dyn DaemonContext> = self.state.clone();
            if !spec.long_running {
                return provider.call(&CallCtx::new(daemon), &method, params).await;
            }
            // Long-running: answer now with the job; the lock moves into it.
            let (job, cancel) = self.jobs.create(&method, device);
            if let Some(d) = device {
                self.state.locks.set_job(d, job);
            }
            let (jobs, events) = (self.jobs.clone(), daemon.clone());
            let progress: ProgressFn = Arc::new(move |fraction, message| {
                jobs.progress(job, fraction, message);
                events.emit(
                    "job.progress",
                    json!({ "job": job, "progress": fraction, "message": message }),
                );
            });
            let (jobs, events) = (self.jobs.clone(), daemon.clone());
            let bytes: BytesFn = Arc::new(move |fraction, message, done, total| {
                jobs.progress_bytes(job, fraction, message, done, total);
                events.emit(
                    "job.progress",
                    json!({
                        "job": job,
                        "progress": fraction,
                        "message": message,
                        "bytes": { "done": done, "total": total },
                    }),
                );
            });
            let ctx = CallCtx {
                daemon: daemon.clone(),
                progress,
                bytes,
                cancel,
            };
            let jobs = self.jobs.clone();
            daemon.emit(
                "job.started",
                json!({ "job": job, "kind": method, "device": device }),
            );
            tokio::spawn(async move {
                let _guard = guard;
                crate::jobs::run_job(jobs, daemon, job, provider.call(&ctx, &method, params)).await;
            });
            Ok(json!({ "job": job, "pending": true }))
        }
        .await;
        match result {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(id, RpcError::from_jidr(&e)),
        }
    }

    /// `events.subscribe {topics}`: forwards matching events to this connection.
    fn subscribe(
        &self,
        params: Value,
        tx: mpsc::UnboundedSender<String>,
    ) -> JidrResult<(Value, JoinHandle<()>)> {
        let p: SubscribeParams = parse_params(params)?;
        let mut rx = self.state.subscribe();
        let topics = p.topics.clone();
        let task = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        if topics.is_empty()
                            || topics.iter().any(|t| ev.topic.starts_with(t.as_str()))
                        {
                            let n = Notification::new(
                                rpc::EVENT_METHOD,
                                serde_json::to_value(&ev).unwrap_or(Value::Null),
                            );
                            if tx.send(line(&n)).is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let n = Notification::new(
                            rpc::EVENT_METHOD,
                            json!({ "topic": "events.lagged", "data": { "missed": n } }),
                        );
                        let _ = tx.send(line(&n));
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok((json!({ "subscribed": p.topics }), task))
    }

    /// `logcat.follow {device, filter}`: streams `logcat` events to this connection.
    async fn logcat_follow(
        &self,
        params: Value,
        tx: mpsc::UnboundedSender<String>,
    ) -> JidrResult<(Value, JoinHandle<()>)> {
        let p: LogcatFollowParams = parse_params(params)?;
        let device = self.state.device(&p.device).await?;
        let info = self.state.connect_info(device.id).await?;
        let mut args = vec!["-v", "threadtime"];
        args.extend(p.filter.iter().map(String::as_str));
        let mut stream = Adb::new(info.adb_path)
            .device(info.adb_serial)
            .logcat_follow(&args)?;
        let id = device.id;
        let task = tokio::spawn(async move {
            while let Some(l) = stream.lines.recv().await {
                let ev = json!({ "topic": "logcat", "data": { "device": id, "line": l } });
                if tx
                    .send(line(&Notification::new(rpc::EVENT_METHOD, ev)))
                    .is_err()
                {
                    break;
                }
            }
            stream.stop().await;
        });
        Ok((json!({ "following": id }), task))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscribeParams {
    /// Topic prefixes, e.g. `["device.", "job."]`; empty = everything.
    #[serde(default)]
    pub topics: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogcatFollowParams {
    pub device: String,
    /// logcat filter specs.
    #[serde(default)]
    pub filter: Vec<String>,
}
