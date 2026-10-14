//! Jidr core model: devices, profiles, images, root state, jobs, error codes,
//! protocol types and the `CommandProvider` trait.

pub mod cancel;
pub mod error;
pub mod model;
pub mod paths;
pub mod provider;
pub mod rpc;

pub use cancel::Cancel;
pub use error::{ErrorCode, JidrError, JidrResult, ResultExt};
pub use model::*;
pub use paths::JidrPaths;
