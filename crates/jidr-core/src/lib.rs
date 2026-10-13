//! Jidr core model: devices, profiles, images, root state, jobs, error codes,
//! protocol types and the `CommandProvider` trait.

pub mod error;
pub mod model;
pub mod paths;

pub use error::{ErrorCode, JidrError, JidrResult, ResultExt};
pub use model::*;
pub use paths::JidrPaths;
