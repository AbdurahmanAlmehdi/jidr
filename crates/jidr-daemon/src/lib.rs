//! Jidr daemon: command registry, JSON-RPC 2.0 server, jobs, events and
//! per-device locks. Registers the core provider plus any provider handed to
//! the builder (`jidr-widgets`, `jidr-root`).

pub mod bundle;
pub mod core_provider;
pub mod jobs;
pub mod lifecycle;
pub mod locks;
pub mod protocol_doc;
pub mod registry;
pub use jidr_core::rpc;
pub mod server;
pub mod state;
pub mod store;
pub mod testing;
pub mod validate;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use jidr_backend::Backend;
use jidr_core::JidrResult;
use jidr_core::provider::CommandProvider;

pub use jidr_core::rpc::Event;
pub use registry::Registry;
pub use state::{DaemonConfig, State};

/// The daemon version clients must match (`hello`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Daemon {
    pub(crate) state: Arc<State>,
    pub(crate) jobs: Arc<jobs::JobManager>,
    idle_timeout: Option<std::time::Duration>,
    /// Fired by `daemon.shutdown`.
    pub(crate) stop: tokio::sync::Notify,
    /// Set when serving ends; open connections close.
    pub(crate) closing: tokio::sync::watch::Sender<bool>,
    pub(crate) registry: Registry,
    pub(crate) version: String,
    clients: AtomicUsize,
}

impl std::fmt::Debug for Daemon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Daemon")
            .field("version", &self.version)
            .field("registry", &self.registry)
            .finish()
    }
}

pub struct DaemonBuilder {
    config: DaemonConfig,
    backend: Arc<dyn Backend>,
    providers: Vec<Arc<dyn CommandProvider>>,
    version: Option<String>,
}

impl DaemonBuilder {
    /// Adds a provider (registered after the core one, in call order).
    pub fn provider(mut self, p: Arc<dyn CommandProvider>) -> Self {
        self.providers.push(p);
        self
    }

    /// Overrides the advertised version (tests of the stale-daemon path).
    pub fn version(mut self, v: &str) -> Self {
        self.version = Some(v.to_string());
        self
    }

    pub fn build(self) -> JidrResult<Arc<Daemon>> {
        let idle_timeout = self.config.idle_timeout;
        let state = State::new(self.config, self.backend)?;
        let jobs = Arc::new(jobs::JobManager::new());
        let mut registry = Registry::new();
        let core: Arc<dyn CommandProvider> =
            Arc::new(core_provider::CoreProvider::new(state.clone()));
        registry.register(core)?;
        registry.register(Arc::new(jobs::JobsProvider::new(jobs.clone())))?;
        for p in self.providers {
            registry.register(p)?;
        }
        state.set_providers(registry.providers().to_vec());
        Ok(Arc::new(Daemon {
            state,
            jobs,
            idle_timeout,
            stop: tokio::sync::Notify::new(),
            closing: tokio::sync::watch::channel(false).0,
            registry,
            version: self.version.unwrap_or_else(|| VERSION.to_string()),
            clients: AtomicUsize::new(0),
        }))
    }
}

impl Daemon {
    pub fn builder(config: DaemonConfig, backend: Arc<dyn Backend>) -> DaemonBuilder {
        DaemonBuilder {
            config,
            backend,
            providers: Vec::new(),
            version: None,
        }
    }

    pub fn state(&self) -> &Arc<State> {
        &self.state
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub(crate) fn client_connected(&self) {
        self.clients.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn client_disconnected(&self) {
        self.clients.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn client_count(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }

    pub fn jobs(&self) -> &Arc<jobs::JobManager> {
        &self.jobs
    }

    /// No running devices, no connected clients, no active jobs (D4).
    pub fn is_idle(&self) -> bool {
        self.state.running_count() == 0 && self.client_count() == 0 && self.jobs.active() == 0
    }

    /// Binds the socket, re-adopts running emulators and serves until
    /// `shutdown` resolves or the daemon has been idle for `idle_timeout`.
    /// The socket file is removed on exit.
    pub async fn serve(self: Arc<Self>, shutdown: impl Future<Output = ()>) -> JidrResult<()> {
        let path = self.state.paths.socket();
        let listener = server::bind(&path)?;
        self.state.adopt_running().await;
        tracing::info!(socket = %path.display(), version = %self.version, "daemon listening");
        let idle = {
            let me = self.clone();
            let timeout = self.idle_timeout;
            async move {
                match timeout {
                    Some(t) => lifecycle::idle_for(t, || me.is_idle()).await,
                    None => std::future::pending().await,
                }
            }
        };
        tokio::pin!(shutdown);
        tokio::pin!(idle);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                _ = self.stop.notified() => {
                    tracing::info!("shutdown requested");
                    break;
                }
                _ = &mut idle => {
                    tracing::info!("idle; exiting (D4)");
                    break;
                }
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(server::handle_connection(self.clone(), stream));
                    }
                    Err(e) => tracing::warn!("accept failed: {e}"),
                },
            }
        }
        let _ = std::fs::remove_file(&path);
        self.closing.send_replace(true);
        Ok(())
    }
}
