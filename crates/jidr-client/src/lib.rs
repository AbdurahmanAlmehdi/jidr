//! Jidr daemon protocol client used by the CLI, the GUI and the MCP server.
//!
//! ```no_run
//! # async fn demo() -> jidr_core::JidrResult<()> {
//! let paths = jidr_core::JidrPaths::discover()?;
//! let client = jidr_client::Client::connect_or_spawn(&paths, "cli", &Default::default()).await?;
//! let devices = client.call("device.list", serde_json::json!({})).await?;
//! let booted = client.run("device.start", serde_json::json!({"device": "Phone"}), |_| {}).await?;
//! # Ok(()) }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use jidr_core::rpc::{EVENT_METHOD, Event, HelloResult, Response, RpcError};
use jidr_core::{ErrorCode, JidrError, JidrPaths, JidrResult, Job, JobState, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// The client version sent in `hello`; must equal the daemon's.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How long to wait for a freshly spawned daemon's socket.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(15);

type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

/// How to start the daemon when none is running (D4: on demand).
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// The `jidr` binary. Default: `$JIDR_DAEMON_BIN`, then the `jidr` shipped
    /// with the current executable ([`bundled_jidr`]), then `jidr` on `PATH`.
    pub program: Option<PathBuf>,
    /// Replace a daemon of another version instead of failing with `daemon_stale`.
    pub restart_stale: bool,
    /// How long to wait for a spawned daemon (default 15 s).
    pub timeout: Option<Duration>,
}

impl SpawnOptions {
    pub fn program(&self) -> PathBuf {
        if let Some(p) = &self.program {
            return p.clone();
        }
        if let Some(p) = std::env::var_os("JIDR_DAEMON_BIN") {
            return PathBuf::from(p);
        }
        std::env::current_exe()
            .ok()
            .and_then(|exe| bundled_jidr(&exe))
            .unwrap_or_else(|| PathBuf::from("jidr"))
    }
}

/// The `jidr` binary shipped with `exe`: `Contents/Helpers/jidr` in the macOS
/// app bundle, else a `jidr` next to it. Never `exe` itself: on the default
/// case-insensitive APFS volume, `Jidr.app/Contents/MacOS/jidr` *is* the GUI
/// `MacOS/Jidr`, and spawning the GUI as the daemon makes every copy spawn
/// another one.
pub fn bundled_jidr(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    [dir.join("../Helpers/jidr"), dir.join("jidr")]
        .into_iter()
        .find(|p| p.is_file() && !same_file(p, exe))
}

/// Whether two paths name the same file on disk (case aliases included).
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

pub struct Client {
    writer: Mutex<OwnedWriteHalf>,
    pending: Pending,
    events: broadcast::Sender<Event>,
    next: AtomicU64,
    reader: JoinHandle<()>,
    hello: HelloResult,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("daemon", &self.hello)
            .finish_non_exhaustive()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn is_unavailable(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

impl Client {
    /// Connects to a running daemon and says hello.
    pub async fn connect(socket: &Path, client: &str) -> JidrResult<Self> {
        let stream = UnixStream::connect(socket).await.map_err(|e| {
            JidrError::new(
                ErrorCode::Unexpected,
                format!("no Jidr daemon at {}: {e}", socket.display()),
            )
            .with_data(json!({ "unavailable": is_unavailable(&e) }))
        })?;
        Self::handshake(stream, client).await
    }

    async fn handshake(stream: UnixStream, client: &str) -> JidrResult<Self> {
        let (r, w) = stream.into_split();
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(1024);
        let reader = tokio::spawn(read_loop(
            BufReader::new(r),
            pending.clone(),
            events.clone(),
        ));
        let mut c = Self {
            writer: Mutex::new(w),
            pending,
            events,
            next: AtomicU64::new(1),
            reader,
            hello: HelloResult {
                version: String::new(),
                protocol: String::new(),
                pid: 0,
            },
        };
        let hello = c
            .call("hello", json!({ "version": VERSION, "client": client }))
            .await?;
        c.hello = serde_json::from_value(hello).ctx(ErrorCode::Unexpected, "bad hello reply")?;
        Ok(c)
    }

    /// Connects, starting the daemon first if none is running. With
    /// `restart_stale`, a daemon of another version is asked to exit and
    /// replaced (its running emulators are re-adopted by the new one).
    pub async fn connect_or_spawn(
        paths: &JidrPaths,
        client: &str,
        opts: &SpawnOptions,
    ) -> JidrResult<Self> {
        let socket = paths.socket();
        match Self::connect(&socket, client).await {
            Ok(c) => return Ok(c),
            Err(e) if e.code == ErrorCode::DaemonStale && opts.restart_stale => {
                shutdown_daemon(&socket).await?;
            }
            Err(e) if is_unavailable_err(&e) => {}
            Err(e) => return Err(e),
        }
        spawn_daemon(paths, opts)?;
        let wait = opts.timeout.unwrap_or(SPAWN_TIMEOUT);
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match Self::connect(&socket, client).await {
                Ok(c) => return Ok(c),
                Err(e) if !is_unavailable_err(&e) => return Err(e),
                Err(_) if tokio::time::Instant::now() >= deadline => {
                    return Err(JidrError::unexpected(format!(
                        "the Jidr daemon did not start within {} s; see {}",
                        wait.as_secs(),
                        paths.logs.join("daemon.log").display()
                    )));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }

    /// What the daemon said in `hello`.
    pub fn daemon(&self) -> &HelloResult {
        &self.hello
    }

    /// Calls a method; JSON-RPC errors become `JidrError`s with their code.
    pub async fn call(&self, method: &str, params: Value) -> JidrResult<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut line =
            serde_json::to_string(&req).ctx(ErrorCode::Unexpected, "encoding request")?;
        line.push('\n');
        {
            let mut w = self.writer.lock().await;
            if let Err(e) = w.write_all(line.as_bytes()).await {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                return Err(JidrError::unexpected(format!(
                    "daemon connection lost: {e}"
                )));
            }
        }
        let resp = rx
            .await
            .map_err(|_| JidrError::unexpected("the daemon closed the connection"))?;
        match (resp.result, resp.error) {
            (_, Some(e)) => Err(e.to_jidr()),
            (Some(v), None) => Ok(v),
            (None, None) => Ok(Value::Null),
        }
    }

    /// Typed call.
    pub async fn call_as<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> JidrResult<R> {
        let params = serde_json::to_value(params).ctx(ErrorCode::Usage, "encoding params")?;
        let v = self.call(method, params).await?;
        serde_json::from_value(v).ctx(ErrorCode::Unexpected, format!("decoding {method} result"))
    }

    /// Calls a method and, if it started a job, follows the job to the end,
    /// reporting each progress update. Returns the job's result.
    pub async fn run(
        &self,
        method: &str,
        params: Value,
        on_progress: impl Fn(&Job),
    ) -> JidrResult<Value> {
        let v = self.call(method, params).await?;
        let Some(job) = pending_job(&v) else {
            return Ok(v);
        };
        let mut last = None;
        loop {
            let j = self.wait_job(job, Some(Duration::from_secs(1))).await?;
            if last.as_ref() != Some(&(j.progress, j.message.clone())) {
                on_progress(&j);
                last = Some((j.progress, j.message.clone()));
            }
            match j.state {
                JobState::Succeeded => return Ok(j.result.unwrap_or(Value::Null)),
                JobState::Failed => {
                    return Err(j
                        .error
                        .unwrap_or_else(|| JidrError::unexpected(format!("{method} failed"))));
                }
                JobState::Cancelled => return Err(jidr_core::cancel::cancelled()),
                JobState::Pending | JobState::Running => {}
            }
        }
    }

    /// Waits for a job to end, or `timeout` (then returns it as it is).
    pub async fn wait_job(&self, job: Uuid, timeout: Option<Duration>) -> JidrResult<Job> {
        let params = match timeout {
            Some(t) => json!({ "job": job, "timeout_s": t.as_secs().max(1) }),
            None => json!({ "job": job }),
        };
        self.call_as("jobs.wait", params).await
    }

    pub async fn cancel_job(&self, job: Uuid) -> JidrResult<Job> {
        self.call_as("jobs.cancel", json!({ "job": job })).await
    }

    /// Subscribes to events whose topic starts with any of `topics` (all if
    /// empty) and returns a receiver for them.
    pub async fn subscribe(&self, topics: &[&str]) -> JidrResult<broadcast::Receiver<Event>> {
        let rx = self.events.subscribe();
        self.call("events.subscribe", json!({ "topics": topics }))
            .await?;
        Ok(rx)
    }

    /// A receiver for events on this connection (after `subscribe` or
    /// `logcat.follow`).
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}

/// The job id of a `{job, pending: true}` answer.
pub fn pending_job(v: &Value) -> Option<Uuid> {
    if v.get("pending") != Some(&Value::Bool(true)) {
        return None;
    }
    v.get("job")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn is_unavailable_err(e: &JidrError) -> bool {
    e.data.as_ref().and_then(|d| d.get("unavailable")) == Some(&Value::Bool(true))
}

async fn read_loop(
    mut r: BufReader<tokio::net::unix::OwnedReadHalf>,
    pending: Pending,
    events: broadcast::Sender<Event>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match r.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("method").and_then(Value::as_str) == Some(EVENT_METHOD) {
            if let Some(ev) = v
                .get("params")
                .and_then(|p| serde_json::from_value::<Event>(p.clone()).ok())
            {
                let _ = events.send(ev);
            }
            continue;
        }
        let Ok(resp) = serde_json::from_value::<Response>(v) else {
            continue;
        };
        let Some(id) = resp.id.as_u64() else {
            // A protocol error without an id (e.g. parse error): nothing to route.
            continue;
        };
        if let Some(tx) = pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        {
            let _ = tx.send(resp);
        }
    }
    // Wake every waiter: their senders drop here.
    pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Asks the daemon at `socket` to exit (allowed before `hello`) and waits for
/// its socket to disappear.
pub async fn shutdown_daemon(socket: &Path) -> JidrResult<()> {
    let mut s = UnixStream::connect(socket).await.ctx(
        ErrorCode::Unexpected,
        format!("connecting to {}", socket.display()),
    )?;
    s.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"daemon.shutdown\"}\n")
        .await
        .ctx(ErrorCode::Unexpected, "asking the daemon to exit")?;
    for _ in 0..100 {
        if !socket.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(JidrError::unexpected("the old daemon did not exit"))
}

/// Starts `jidr daemon` detached from this process and its terminal.
fn spawn_daemon(paths: &JidrPaths, opts: &SpawnOptions) -> JidrResult<()> {
    std::fs::create_dir_all(&paths.logs).ctx(
        ErrorCode::Unexpected,
        format!("creating {}", paths.logs.display()),
    )?;
    let log_path = paths.logs.join("daemon.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ctx(
            ErrorCode::Unexpected,
            format!("opening {}", log_path.display()),
        )?;
    let log_err = log
        .try_clone()
        .ctx(ErrorCode::Unexpected, "duplicating log handle")?;
    let program = opts.program();
    let mut cmd = std::process::Command::new(&program);
    cmd.arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: Ctrl-C in the user's terminal doesn't reach it.
        cmd.process_group(0);
    }
    cmd.spawn().map_err(|e| {
        JidrError::unexpected(format!(
            "cannot start the daemon ({}): {e}",
            program.display()
        ))
    })?;
    Ok(())
}

/// Turns a raw JSON-RPC error into a `JidrError` (for callers that speak
/// the protocol themselves).
pub fn error_from_rpc(e: &RpcError) -> JidrError {
    e.to_jidr()
}

#[cfg(test)]
mod tests {
    use super::bundled_jidr;
    use std::path::Path;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"bin").unwrap();
    }

    #[test]
    fn the_app_bundle_uses_its_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let contents = tmp.path().join("Jidr.app/Contents");
        touch(&contents.join("MacOS/Jidr"));
        touch(&contents.join("Helpers/jidr"));
        let found = bundled_jidr(&contents.join("MacOS/Jidr")).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            contents.join("Helpers/jidr").canonicalize().unwrap()
        );
    }

    /// Regression: launching Jidr.app made the GUI spawn itself as the daemon
    /// (`MacOS/jidr` is `MacOS/Jidr` on case-insensitive APFS), and every copy
    /// spawned another.
    #[test]
    fn never_the_executable_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let gui = tmp.path().join("Jidr.app/Contents/MacOS/Jidr");
        touch(&gui);
        assert_eq!(bundled_jidr(&gui), None);
    }

    #[test]
    fn a_sibling_jidr_outside_a_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let gui = tmp.path().join("bin/jidr-gui");
        touch(&gui);
        touch(&tmp.path().join("bin/jidr"));
        assert_eq!(bundled_jidr(&gui), Some(tmp.path().join("bin/jidr")));
    }
}
