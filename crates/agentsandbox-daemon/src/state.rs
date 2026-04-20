//! Shared application state wired into every handler.

use agentsandbox_core::SandboxAdapter;
use sqlx::SqlitePool;
use std::sync::Arc;

pub struct AppState {
    pub db: SqlitePool,
    pub adapter: Arc<dyn SandboxAdapter>,
}

/// Type alias we pass to axum's `State` extractor.
pub type SharedState = Arc<AppState>;
