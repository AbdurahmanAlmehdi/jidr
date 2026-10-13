//! Jidr daemon: command registry, JSON-RPC 2.0 server, jobs, events and
//! per-device locks. Registers the core provider plus any provider handed to
//! the builder (`jidr-widgets`, `jidr-root`).

pub mod core_provider;
pub mod locks;
pub mod registry;
pub mod rpc;
pub mod server;
pub mod state;
pub mod store;
pub mod validate;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use jidr_backend::Backend;
use jidr_core::JidrResult;
use jidr_core::provider::CommandProvider;

pub use registry::Registry;
pub use state::{DaemonConfig, Event, State};

/// The daemon version clients must match (`hello`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Daemon {
    pub(crate) state: Arc<State>,
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
}

impl DaemonBuilder {
    /// Adds a provider (registered after the core one, in call order).
    pub fn provider(mut self, p: Arc<dyn CommandProvider>) -> Self {
        self.providers.push(p);
        self
    }

    pub fn build(self) -> JidrResult<Arc<Daemon>> {
        let state = State::new(self.config, self.backend)?;
        let mut registry = Registry::new();
        let core: Arc<dyn CommandProvider> =
            Arc::new(core_provider::CoreProvider::new(state.clone()));
        registry.register(core)?;
        for p in self.providers {
            registry.register(p)?;
        }
        state.set_providers(registry.providers().to_vec());
        Ok(Arc::new(Daemon {
            state,
            registry,
            version: VERSION.to_string(),
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

    /// Binds the socket, re-adopts running emulators and serves until
    /// `shutdown` resolves. The socket file is removed on exit.
    pub async fn serve(self: Arc<Self>, shutdown: impl Future<Output = ()>) -> JidrResult<()> {
        let path = self.state.paths.socket();
        let listener = server::bind(&path)?;
        self.state.adopt_running().await;
        tracing::info!(socket = %path.display(), version = %self.version, "daemon listening");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(server::handle_connection(self.clone(), stream));
                    }
                    Err(e) => tracing::warn!("accept failed: {e}"),
                },
            }
        }
        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
