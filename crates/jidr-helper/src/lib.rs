//! The rooted on-device helper (ADR-0003, #26).
//!
//! One small static binary, `jidr-helper`, runs as root on the Android guest
//! and answers a typed RPC ([`rpc`]) on an abstract Unix socket. The daemon
//! pushes it, starts it under `adb root` or Magisk `su`, and reaches it
//! through `adb forward` ([`host`], behind the `host` feature). Privileged
//! widgets (#17 packet loss first, #19 the Android ID, then #18,
//! operator/SIM, disk limits, gamepad) add methods here instead of each
//! shipping its own daemon.
//!
//! - [`rpc`]: the wire types, shared by both sides;
//! - [`server`]: the request loop, independent of the socket;
//! - [`netem`]: packet loss with `tc … netem` (#17);
//! - [`identity`]: the Android ID, as apps read it (#19);
//! - [`testing`]: a fake guest for offline tests.

pub mod identity;
pub mod netem;
pub mod rpc;
pub mod server;

#[cfg(feature = "host")]
pub mod host;
#[cfg(feature = "host")]
pub mod testing;

/// The helper's version: the Jidr version it was built with.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Hex SHA-256 of a file (the helper reports its own; the daemon compares).
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(std::fs::read(path)?)))
}
