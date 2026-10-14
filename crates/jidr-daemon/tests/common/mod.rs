//! A raw-socket client on top of `jidr_daemon::testing::TestDaemon`.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use jidr_backend::fake::FakeBackend;
use jidr_core::JidrPaths;
use jidr_daemon::Daemon;
use jidr_daemon::testing::TestDaemon;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub struct Harness {
    pub tmp: tempfile::TempDir,
    pub paths: JidrPaths,
    pub backend: Arc<FakeBackend>,
    pub daemon: Arc<Daemon>,
    inner: TestDaemon,
}

impl Harness {
    pub async fn start() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let inner = TestDaemon::start(tmp.path()).await.unwrap();
        Self {
            paths: inner.paths.clone(),
            backend: inner.backend.clone(),
            daemon: inner.daemon.clone(),
            tmp,
            inner,
        }
    }

    pub async fn conn(&self) -> Conn {
        Conn::open(&self.paths.socket()).await
    }

    /// A connection that already said hello.
    pub async fn client(&self) -> Conn {
        let mut c = self.conn().await;
        let r = c
            .call(
                "hello",
                json!({ "version": jidr_daemon::VERSION, "client": "test" }),
            )
            .await;
        assert!(r.get("result").is_some(), "{r}");
        c
    }

    pub async fn stop(self) {
        self.inner.stop().await;
    }
}

pub struct Conn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next: u64,
}

impl Conn {
    pub async fn open(path: &Path) -> Self {
        let s = UnixStream::connect(path).await.unwrap();
        let (r, w) = s.into_split();
        Self {
            reader: BufReader::new(r),
            writer: w,
            next: 1,
        }
    }

    pub async fn send_raw(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    pub async fn read(&mut self) -> Value {
        let mut l = String::new();
        tokio::time::timeout(Duration::from_secs(10), self.reader.read_line(&mut l))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_str(&l).unwrap()
    }

    /// Sends a request and returns the full response object.
    pub async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send_raw(&req.to_string()).await;
        loop {
            let v = self.read().await;
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    pub async fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params).await;
        r.get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} failed: {r}"))
    }

    /// Like `ok`, but follows a `{job, pending}` answer to the job's result.
    pub async fn run(&mut self, method: &str, params: Value) -> Value {
        let r = self.ok(method, params).await;
        if r.get("pending") != Some(&json!(true)) {
            return r;
        }
        let job = self.ok("jobs.wait", json!({ "job": r["job"] })).await;
        assert_eq!(job["state"], "succeeded", "{method} job: {job}");
        job["result"].clone()
    }

    pub async fn err(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params).await;
        r.get("error")
            .cloned()
            .unwrap_or_else(|| panic!("{method} unexpectedly succeeded: {r}"))
    }
}
