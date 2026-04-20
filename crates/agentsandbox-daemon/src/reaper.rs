//! TTL reaper: periodic sweep that destroys expired sandboxes.
//!
//! Runs as a `tokio::spawn` task in `main`. It is the primary enforcement
//! mechanism for `ttl_seconds`; the backend backstop (a long `sleep` PID 1
//! inside the container) is a safety net for when the daemon is down.

use crate::audit::{self, Event};
use crate::state::SharedState;
use crate::store;
use std::time::Duration;

const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Run the reaper forever at `DEFAULT_INTERVAL`. Cancellation is left to
/// `tokio::spawn` / runtime shutdown; the loop body itself never panics.
pub async fn run(state: SharedState) {
    let mut ticker = tokio::time::interval(DEFAULT_INTERVAL);
    // Don't fire a burst after a long pause — just resume the cadence.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if let Err(e) = sweep(&state).await {
            tracing::error!(error = %e, "reaper sweep failed");
        }
    }
}

/// Single reaper pass. Exposed for tests.
pub async fn sweep(state: &SharedState) -> Result<usize, crate::error::ApiError> {
    let now = chrono::Utc::now();
    let expired = store::list_expired(&state.db, now).await?;
    let count = expired.len();
    for id in expired {
        tracing::info!(sandbox_id = %id, "reaping expired sandbox");
        if let Err(e) = state.adapter.destroy(&id).await {
            tracing::warn!(sandbox_id = %id, error = %e, "destroy during reap failed");
        }
        store::set_status(&state.db, &id, agentsandbox_core::SandboxStatus::Stopped).await?;
        audit::record(&state.db, &id, Event::Expired, None).await;
    }
    Ok(count)
}
