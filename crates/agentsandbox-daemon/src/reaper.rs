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
        if let Some(row) = store::get_sandbox(&state.db, &id).await? {
            match state.registry.get(&row.backend) {
                Ok(backend) => {
                    if let Err(error) = backend.destroy(row.runtime_handle()).await {
                        tracing::warn!(
                            sandbox_id = %id,
                            error = %error,
                            "destroy during reap failed"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(sandbox_id = %id, error = %error, "backend missing during reap");
                }
            }
        }
        store::set_status(&state.db, &id, agentsandbox_sdk::backend::SandboxState::Stopped).await?;
        audit::record(&state.db, &id, Event::Expired, None).await;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::{Arc, Mutex}};

    use async_trait::async_trait;
    use agentsandbox_sdk::{
        backend::{
            BackendCapabilities, BackendDescriptor, BackendFactory, ExecResult, IsolationLevel,
            SandboxBackend,
        },
        error::BackendError,
        ir::SandboxIR,
    };
    use chrono::{Duration, Utc};
    use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
    use std::str::FromStr;

    use crate::{registry::BackendRegistry, state::AppState, store};

    #[derive(Default)]
    struct MockBackend {
        destroyed: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SandboxBackend for MockBackend {
        async fn create(&self, _ir: &SandboxIR) -> Result<String, BackendError> {
            Err(BackendError::Internal("unused".into()))
        }

        async fn exec(
            &self,
            _handle: &str,
            _command: &str,
            _timeout_ms: Option<u64>,
        ) -> Result<ExecResult, BackendError> {
            Err(BackendError::Internal("unused".into()))
        }

        async fn status(
            &self,
            handle: &str,
        ) -> Result<agentsandbox_sdk::backend::SandboxStatus, BackendError> {
            Ok(agentsandbox_sdk::backend::SandboxStatus {
                sandbox_id: handle.into(),
                state: agentsandbox_sdk::backend::SandboxState::Running,
                created_at: Utc::now(),
                expires_at: Utc::now(),
                backend_id: "mock".into(),
            })
        }

        async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
            self.destroyed.lock().unwrap().push(handle.into());
            Ok(())
        }

        async fn health_check(&self) -> Result<(), BackendError> {
            Ok(())
        }
    }

    struct MockFactory {
        backend: Arc<MockBackend>,
    }

    impl BackendFactory for MockFactory {
        fn describe(&self) -> BackendDescriptor {
            BackendDescriptor {
                id: "mock",
                display_name: "Mock",
                version: "test",
                trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
                capabilities: BackendCapabilities {
                    isolation_level: IsolationLevel::Process,
                    ..BackendCapabilities::default()
                },
                extensions_schema: None,
            }
        }

        fn create(
            &self,
            _config: &HashMap<String, String>,
        ) -> Result<Box<dyn SandboxBackend>, BackendError> {
            Ok(Box::new(SharedMockBackend(self.backend.clone())))
        }
    }

    struct SharedMockBackend(Arc<MockBackend>);

    #[async_trait]
    impl SandboxBackend for SharedMockBackend {
        async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
            self.0.create(ir).await
        }

        async fn exec(
            &self,
            handle: &str,
            command: &str,
            timeout_ms: Option<u64>,
        ) -> Result<ExecResult, BackendError> {
            self.0.exec(handle, command, timeout_ms).await
        }

        async fn status(
            &self,
            handle: &str,
        ) -> Result<agentsandbox_sdk::backend::SandboxStatus, BackendError> {
            self.0.status(handle).await
        }

        async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
            self.0.destroy(handle).await
        }

        async fn health_check(&self) -> Result<(), BackendError> {
            self.0.health_check().await
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
             (id, lease_token, status, backend, backend_handle, spec_json, ir_json, created_at, expires_at) \
             VALUES (?1, ?2, 'running', 'mock', ?3, '{}', '{}', ?4, ?5)",
        )
        .bind(id)
        .bind(format!("lease-{id}"))
        .bind(format!("handle-{id}"))
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

        let backend = Arc::new(MockBackend::default());
        let mut registry = BackendRegistry::new();
        let factory = MockFactory {
            backend: backend.clone(),
        };
        registry.register(&factory);
        registry.initialize(&factory, &HashMap::new()).await;
        let state = Arc::new(AppState {
            db: db.clone(),
            registry: Arc::new(registry),
        });

        let count = sweep(&state).await.unwrap();
        assert_eq!(count, 1);

        let destroyed = backend.destroyed.lock().unwrap().clone();
        assert_eq!(destroyed, vec!["handle-expired-1".to_string()]);

        let expired = store::get_sandbox(&db, "expired-1").await.unwrap().unwrap();
        assert_eq!(expired.status, "stopped");

        let future = store::get_sandbox(&db, "future-1").await.unwrap().unwrap();
        assert_eq!(future.status, "running");
    }
}
