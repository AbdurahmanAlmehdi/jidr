//! Daemon lifetime (D4): started on demand by the first client, alive while
//! devices run, clients are connected or jobs are active, gone after
//! `IDLE_TIMEOUT` of idleness.

use std::time::Duration;

use tokio::time::{Instant, interval};

pub const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// Resolves once `is_idle()` has been true continuously for `timeout`.
pub async fn idle_for(timeout: Duration, is_idle: impl Fn() -> bool) {
    let mut tick = interval(CHECK_EVERY);
    let mut idle_since: Option<Instant> = None;
    loop {
        tick.tick().await;
        if is_idle() {
            let since = *idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= timeout {
                return;
            }
        } else {
            idle_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Time-mocked: busy for 15 virtual minutes, then idle; exits 10 minutes
    /// after becoming idle, not before.
    #[tokio::test(start_paused = true)]
    async fn exits_ten_minutes_after_last_activity() {
        let idle = Arc::new(AtomicBool::new(false));
        let flag = idle.clone();
        let start = Instant::now();
        let monitor = tokio::spawn(async move {
            idle_for(IDLE_TIMEOUT, move || flag.load(Ordering::Relaxed)).await
        });

        tokio::time::sleep(Duration::from_secs(15 * 60)).await;
        assert!(!monitor.is_finished(), "busy daemon stays up");
        idle.store(true, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(9 * 60)).await;
        assert!(!monitor.is_finished(), "not yet: only 9 idle minutes");
        // Activity resets the clock.
        idle.store(false, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(2)).await;
        idle.store(true, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(9 * 60 + 50)).await;
        assert!(!monitor.is_finished(), "clock restarted after activity");
        monitor.await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_secs(34 * 60) && elapsed < Duration::from_secs(35 * 60),
            "{elapsed:?}"
        );
    }
}
