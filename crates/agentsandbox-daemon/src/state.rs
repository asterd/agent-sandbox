//! Shared application state wired into every handler.

use crate::registry::BackendRegistry;
use crate::{config::DaemonConfig, store::TenantRecord};
use sqlx::SqlitePool;
use std::sync::Arc;

pub struct AppState {
    pub db: SqlitePool,
    pub config: DaemonConfig,
    pub registry: Arc<BackendRegistry>,
}

/// Type alias we pass to axum's `State` extractor.
pub type SharedState = Arc<AppState>;

#[derive(Debug, Clone)]
pub enum AuthContext {
    SingleUser,
    Tenant(TenantRecord),
}

impl AuthContext {
    pub fn single_user() -> Self {
        Self::SingleUser
    }

    pub fn tenant(tenant: TenantRecord) -> Self {
        Self::Tenant(tenant)
    }

    pub fn tenant_id(&self) -> Option<&str> {
        match self {
            Self::SingleUser => None,
            Self::Tenant(tenant) => Some(&tenant.id),
        }
    }

    pub fn hourly_quota(&self) -> Option<i64> {
        match self {
            Self::SingleUser => None,
            Self::Tenant(tenant) => Some(tenant.quota_hourly),
        }
    }
}
