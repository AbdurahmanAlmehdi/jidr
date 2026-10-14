//! A cancellation flag shared between a job and the work it runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{ErrorCode, JidrError, JidrResult};

#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// `Err` (with `data.cancelled = true`) once cancelled.
    pub fn check(&self) -> JidrResult<()> {
        if self.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }
}

/// The error returned by cancelled work. Its code is `unexpected`; callers
/// recognise it with [`is_cancelled`].
pub fn cancelled() -> JidrError {
    JidrError::new(ErrorCode::Unexpected, "cancelled")
        .with_data(serde_json::json!({"cancelled": true}))
}

pub fn is_cancelled(err: &JidrError) -> bool {
    err.data
        .as_ref()
        .and_then(|d| d.get("cancelled"))
        .and_then(|c| c.as_bool())
        == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let a = Cancel::new();
        let b = a.clone();
        assert!(a.check().is_ok());
        b.cancel();
        assert!(a.is_cancelled());
        assert!(is_cancelled(&a.check().unwrap_err()));
        assert!(!is_cancelled(&JidrError::sdk("x")));
    }
}
