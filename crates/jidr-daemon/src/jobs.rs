//! Jobs: long-running commands (boot, image install, Magisk patch) run in the
//! background with progress, cancel and a waitable result.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use jidr_core::provider::{CallCtx, CommandProvider, CommandSpec, DaemonContext, parse_params};
use jidr_core::{Cancel, JidrError, JidrResult, Job, JobBytes, JobState};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use uuid::Uuid;

/// Finished jobs kept for `jobs.get` / `jobs.list`.
const KEEP_FINISHED: usize = 200;

#[derive(Debug)]
struct Entry {
    job: watch::Sender<Job>,
    cancel: Cancel,
}

#[derive(Debug, Default)]
pub struct JobManager {
    entries: Mutex<VecDeque<(Uuid, Entry)>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl JobManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a running job and returns its id and cancel flag.
    pub fn create(&self, kind: &str, device: Option<Uuid>) -> (Uuid, Cancel) {
        let id = Uuid::new_v4();
        let cancel = Cancel::new();
        let job = Job {
            id,
            kind: kind.to_string(),
            device,
            progress: Some(0.0),
            message: String::new(),
            bytes: None,
            state: JobState::Running,
            error: None,
            result: None,
            created_at: Utc::now(),
        };
        let (tx, _) = watch::channel(job);
        let mut entries = lock(&self.entries);
        entries.push_back((
            id,
            Entry {
                job: tx,
                cancel: cancel.clone(),
            },
        ));
        // Drop the oldest finished jobs beyond the cap.
        while entries.len() > KEEP_FINISHED {
            let Some(pos) = entries
                .iter()
                .position(|(_, e)| e.job.borrow().state.is_terminal())
            else {
                break;
            };
            entries.remove(pos);
        }
        (id, cancel)
    }

    fn with<T>(&self, id: Uuid, f: impl FnOnce(&Entry) -> T) -> JidrResult<T> {
        lock(&self.entries)
            .iter()
            .find(|(j, _)| *j == id)
            .map(|(_, e)| f(e))
            .ok_or_else(|| JidrError::not_found(format!("no job {id}")))
    }

    pub fn progress(&self, id: Uuid, fraction: Option<f32>, message: &str) {
        self.update(id, fraction, message, None);
    }

    /// Progress with the bytes of the current download (gap B5).
    pub fn progress_bytes(
        &self,
        id: Uuid,
        fraction: Option<f32>,
        message: &str,
        done: u64,
        total: u64,
    ) {
        self.update(id, fraction, message, Some(JobBytes { done, total }));
    }

    fn update(&self, id: Uuid, fraction: Option<f32>, message: &str, bytes: Option<JobBytes>) {
        let _ = self.with(id, |e| {
            e.job.send_modify(|j| {
                if !j.state.is_terminal() {
                    j.progress = fraction.map(|f| f.clamp(0.0, 1.0));
                    j.message = message.to_string();
                    j.bytes = bytes;
                }
            })
        });
    }

    /// Records the outcome; a cancelled error ends as `cancelled`.
    pub fn finish(&self, id: Uuid, result: JidrResult<Value>) -> Option<Job> {
        self.with(id, |e| {
            e.job.send_modify(|j| match result {
                Ok(v) => {
                    j.state = JobState::Succeeded;
                    j.progress = Some(1.0);
                    j.result = Some(v);
                }
                Err(err) => {
                    j.state = if jidr_core::cancel::is_cancelled(&err) || e.cancel.is_cancelled() {
                        JobState::Cancelled
                    } else {
                        JobState::Failed
                    };
                    j.error = Some(err);
                }
            });
            e.job.borrow().clone()
        })
        .ok()
    }

    pub fn get(&self, id: Uuid) -> JidrResult<Job> {
        self.with(id, |e| e.job.borrow().clone())
    }

    pub fn list(&self, include_finished: bool) -> Vec<Job> {
        lock(&self.entries)
            .iter()
            .map(|(_, e)| e.job.borrow().clone())
            .filter(|j| include_finished || !j.state.is_terminal())
            .collect()
    }

    pub fn active(&self) -> usize {
        self.list(false).len()
    }

    pub fn cancel(&self, id: Uuid) -> JidrResult<Job> {
        self.with(id, |e| {
            e.cancel.cancel();
            e.job.borrow().clone()
        })
    }

    /// Waits until the job ends, or `timeout` passes (then returns it as is).
    pub async fn wait(&self, id: Uuid, timeout: Option<Duration>) -> JidrResult<Job> {
        let mut rx = self.with(id, |e| e.job.subscribe())?;
        let done = async {
            let _ = rx.wait_for(|j| j.state.is_terminal()).await;
        };
        match timeout {
            Some(t) => {
                let _ = tokio::time::timeout(t, done).await;
            }
            None => done.await,
        }
        self.get(id)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobParams {
    /// Job id.
    pub job: Uuid,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobsListParams {
    /// Include finished jobs.
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobWaitParams {
    pub job: Uuid,
    /// Give up after this many seconds and return the job as it is.
    #[serde(default)]
    pub timeout_s: Option<u64>,
}

/// `jobs.*` commands.
pub struct JobsProvider {
    jobs: Arc<JobManager>,
}

impl JobsProvider {
    pub fn new(jobs: Arc<JobManager>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl CommandProvider for JobsProvider {
    fn name(&self) -> &str {
        "jobs"
    }

    fn commands(&self) -> Vec<CommandSpec> {
        vec![
            CommandSpec::new::<JobsListParams>(
                "jobs.list",
                "Running jobs (and finished ones with all=true)",
            ),
            CommandSpec::new::<JobParams>("jobs.get", "A job's state, progress and result"),
            CommandSpec::new::<JobParams>("jobs.cancel", "Cancel a running job"),
            CommandSpec::new::<JobWaitParams>(
                "jobs.wait",
                "Wait for a job to finish and return it",
            ),
        ]
    }

    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
        let to_json =
            |j: Job| serde_json::to_value(j).map_err(|e| JidrError::unexpected(e.to_string()));
        match method {
            "jobs.list" => {
                let p: JobsListParams = parse_params(params)?;
                serde_json::to_value(self.jobs.list(p.all))
                    .map_err(|e| JidrError::unexpected(e.to_string()))
            }
            "jobs.get" => to_json(self.jobs.get(parse_params::<JobParams>(params)?.job)?),
            "jobs.cancel" => {
                let job = self.jobs.cancel(parse_params::<JobParams>(params)?.job)?;
                ctx.daemon.emit("job.cancelling", json!({ "job": job.id }));
                to_json(job)
            }
            "jobs.wait" => {
                let p: JobWaitParams = parse_params(params)?;
                to_json(
                    self.jobs
                        .wait(p.job, p.timeout_s.map(Duration::from_secs))
                        .await?,
                )
            }
            other => Err(JidrError::usage(format!("no command {other}"))),
        }
    }
}

/// Runs `fut` as job `id`, reporting the outcome through `jobs` and events.
pub async fn run_job(
    jobs: Arc<JobManager>,
    daemon: Arc<dyn DaemonContext>,
    id: Uuid,
    fut: impl std::future::Future<Output = JidrResult<Value>>,
) {
    let result = fut.await;
    if let Some(job) = jobs.finish(id, result) {
        daemon.emit(
            "job.finished",
            serde_json::to_value(&job).unwrap_or(Value::Null),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lifecycle_and_wait() {
        let m = Arc::new(JobManager::new());
        let (id, cancel) = m.create("device.start", None);
        m.progress(id, Some(0.5), "booting");
        assert_eq!(m.get(id).unwrap().progress, Some(0.5));
        assert_eq!(m.active(), 1);
        let waiter = {
            let m = m.clone();
            tokio::spawn(async move { m.wait(id, None).await.unwrap() })
        };
        m.cancel(id).unwrap();
        assert!(cancel.is_cancelled());
        m.finish(id, Err(jidr_core::cancel::cancelled()));
        let j = waiter.await.unwrap();
        assert_eq!(j.state, JobState::Cancelled);
        assert_eq!(m.active(), 0);
        assert_eq!(m.list(true).len(), 1);

        let (ok, _) = m.create("image.install", None);
        m.finish(ok, Ok(json!({"x": 1})));
        assert_eq!(m.get(ok).unwrap().state, JobState::Succeeded);
        let (bad, _) = m.create("image.install", None);
        m.finish(bad, Err(JidrError::sdk("disk full")));
        assert_eq!(m.get(bad).unwrap().state, JobState::Failed);
        assert_eq!(
            m.get(Uuid::new_v4()).unwrap_err().code,
            jidr_core::ErrorCode::NotFound
        );
    }

    #[tokio::test]
    async fn wait_times_out_with_the_current_state() {
        let m = JobManager::new();
        let (id, _) = m.create("x", None);
        let j = m.wait(id, Some(Duration::from_millis(20))).await.unwrap();
        assert_eq!(j.state, JobState::Running);
    }

    /// Gap B5: a download's bytes show on the job; any other progress clears
    /// them.
    #[test]
    fn download_bytes_are_set_then_cleared() {
        let m = JobManager::new();
        let (id, _) = m.create("image.install", None);
        m.progress_bytes(id, Some(0.1), "Downloading x", 10, 100);
        let j = m.get(id).unwrap();
        assert_eq!(
            j.bytes,
            Some(JobBytes {
                done: 10,
                total: 100
            })
        );
        assert_eq!(j.message, "Downloading x");
        m.progress(id, Some(0.9), "Extracting x");
        assert_eq!(m.get(id).unwrap().bytes, None);
        let json = serde_json::to_value(m.get(id).unwrap()).unwrap();
        assert!(json.get("bytes").is_none(), "absent, not null: {json}");
    }

    #[test]
    fn finished_jobs_are_capped() {
        let m = JobManager::new();
        for _ in 0..(KEEP_FINISHED + 50) {
            let (id, _) = m.create("x", None);
            m.finish(id, Ok(Value::Null));
        }
        assert!(m.list(true).len() <= KEEP_FINISHED);
    }
}
