//! Shared application state wired into every handler.

use crate::registry::BackendRegistry;
use sqlx::SqlitePool;
use std::sync::Arc;

pub struct AppState {
    pub db: SqlitePool,
    pub registry: Arc<BackendRegistry>,
}

/// Type alias we pass to axum's `State` extractor.
pub type SharedState = Arc<AppState>;
