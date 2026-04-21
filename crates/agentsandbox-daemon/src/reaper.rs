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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use agentsandbox_core::{AdapterError, ExecResult, SandboxAdapter, SandboxInfo, SandboxStatus};
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
    use std::str::FromStr;

    use crate::state::AppState;
    use crate::store;

    #[derive(Default)]
    struct MockAdapter {
        destroyed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SandboxAdapter for MockAdapter {
        async fn create(&self, _ir: &agentsandbox_core::SandboxIR) -> Result<String, AdapterError> {
            Err(AdapterError::Internal("unused".into()))
        }

        async fn exec(
            &self,
            _sandbox_id: &str,
            _command: &str,
        ) -> Result<ExecResult, AdapterError> {
            Err(AdapterError::Internal("unused".into()))
        }

        async fn inspect(&self, sandbox_id: &str) -> Result<SandboxInfo, AdapterError> {
            Ok(SandboxInfo {
                sandbox_id: sandbox_id.into(),
                status: SandboxStatus::Running,
                created_at: Utc::now(),
                expires_at: Utc::now(),
            })
        }

        async fn destroy(&self, sandbox_id: &str) -> Result<(), AdapterError> {
            self.destroyed.lock().unwrap().push(sandbox_id.into());
            Ok(())
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }

        async fn health_check(&self) -> Result<(), AdapterError> {
            Ok(())
        }
    }

    async fn test_db() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn insert_sandbox(pool: &SqlitePool, id: &str, expires_at: chrono::DateTime<Utc>) {
        let created_at = expires_at - Duration::seconds(60);
        sqlx::query(
            "INSERT INTO sandboxes \
             (id, lease_token, status, backend, spec_json, ir_json, created_at, expires_at) \
             VALUES (?1, ?2, 'running', 'mock', '{}', '{}', ?3, ?4)",
        )
        .bind(id)
        .bind(format!("lease-{id}"))
        .bind(created_at.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sweep_destroys_and_stops_expired_sandboxes() {
        let db = test_db().await;
        insert_sandbox(&db, "expired-1", Utc::now() - Duration::seconds(1)).await;
        insert_sandbox(&db, "future-1", Utc::now() + Duration::seconds(600)).await;

        let adapter = Arc::new(MockAdapter::default());
        let state = Arc::new(AppState {
            db: db.clone(),
            adapter: adapter.clone(),
        });

        let count = sweep(&state).await.unwrap();
        assert_eq!(count, 1);

        let destroyed = adapter.destroyed.lock().unwrap().clone();
        assert_eq!(destroyed, vec!["expired-1".to_string()]);

        let expired = store::get_sandbox(&db, "expired-1").await.unwrap().unwrap();
        assert_eq!(expired.status, "stopped");

        let future = store::get_sandbox(&db, "future-1").await.unwrap().unwrap();
        assert_eq!(future.status, "running");
    }
}
