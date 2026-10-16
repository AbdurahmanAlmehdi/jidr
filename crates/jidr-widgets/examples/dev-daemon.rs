//! A real daemon with the widgets provider registered, for running the GUI
//! and the widgets by hand until `jidr daemon` lands (gap B1). Not shipped.
//!
//! ```text
//! cargo build -p jidr-widgets --example dev-daemon
//! JIDR_DAEMON_BIN=target/debug/examples/dev-daemon cargo run -p jidr-gui
//! ```
//! The GUI starts it as `<bin> daemon`; the argument is accepted and ignored.
//! Android Studio's SDK (`JIDR_TEST_SDK`, default `~/Library/Android/sdk`)
//! is used read-only (D3).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use jidr_backend::AospBackend;
use jidr_core::JidrPaths;
use jidr_daemon::{Daemon, DaemonConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let paths = JidrPaths::discover()?;
    let studio = std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join("Library/Android/sdk")))
        .filter(|p| p.is_dir());
    let daemon = Daemon::builder(
        DaemonConfig {
            paths,
            studio_sdk: studio,
            idle_timeout: Some(Duration::from_secs(600)),
            sdk_repo: None,
        },
        Arc::new(AospBackend::new()),
    )
    .provider(jidr_widgets::provider())
    .build()?;
    daemon
        .serve(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
