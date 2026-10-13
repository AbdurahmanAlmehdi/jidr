//! docs/control-protocol.md's command reference must match `commands.list`.
//! `JIDR_UPDATE_DOCS=1 cargo nextest run -p jidr-daemon protocol_doc` rewrites it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use jidr_core::JidrPaths;
use jidr_daemon::protocol_doc::{render, splice};
use jidr_daemon::{Daemon, DaemonConfig};

#[test]
fn protocol_doc_matches_commands_list() {
    let tmp = tempfile::tempdir().unwrap();
    let daemon = Daemon::builder(
        DaemonConfig {
            paths: JidrPaths::under(tmp.path()),
            studio_sdk: None,
            idle_timeout: None,
            sdk_repo: None,
        },
        Arc::new(jidr_backend::fake::FakeBackend::new()),
    )
    .build()
    .unwrap();
    let generated = render(&daemon.registry().list());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/control-protocol.md");
    let doc = std::fs::read_to_string(&path).unwrap();
    let updated =
        splice(&doc, &generated).expect("docs/control-protocol.md lost its commands markers");
    if std::env::var_os("JIDR_UPDATE_DOCS").is_some() {
        std::fs::write(&path, &updated).unwrap();
        return;
    }
    assert!(
        updated == doc,
        "docs/control-protocol.md is out of date with commands.list; run\n  JIDR_UPDATE_DOCS=1 cargo nextest run -p jidr-daemon protocol_doc"
    );
}
