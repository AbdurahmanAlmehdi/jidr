//! Jidr Slint GUI (GPL-3.0-only, D18). A thin client: every action is a
//! protocol call through `jidr-client`; only view state lives here.

/// Code generated from `ui/*.slint` (it uses `unwrap` internally).
#[allow(
    clippy::all,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented
)]
mod ui {
    slint::include_modules!();
}
pub use ui::*;

pub mod app;
pub mod bridge;
pub mod fixtures;
pub mod model;
pub mod offscreen;
pub mod snapshots;

pub use app::run;
