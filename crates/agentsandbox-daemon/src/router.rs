//! Router wiring. Isolated so tests can mount the same routes against an
//! in-memory SQLite DB and a mock adapter.

use axum::{
    middleware,
    routing::{delete, get, post},
    Router,
};

use crate::{handlers, middleware::auth::auth_middleware, state::SharedState};

pub fn build(state: SharedState) -> Router {
    Router::new()
        .route("/metrics", get(handlers::metrics))
        .route("/v1/health", get(handlers::health))
        .route("/v1/backends", get(handlers::list_backends))
        .route("/v1/runtime-info", get(handlers::runtime_info))
        .route(
            "/v1/backends/:id/extensions-schema",
            get(handlers::get_backend_extensions_schema),
        )
        .route("/v1/sandboxes", post(handlers::create_sandbox))
        .route("/v1/sandboxes", get(handlers::list_sandboxes))
        .route("/v1/sandboxes/restore", post(handlers::restore_sandbox))
        .route("/v1/sandboxes/:id", get(handlers::inspect_sandbox))
        .route("/v1/sandboxes/:id/exec", post(handlers::exec_sandbox))
        .route("/v1/sandboxes/:id/files", post(handlers::upload_file))
        .route(
            "/v1/sandboxes/:id/files/*path",
            get(handlers::download_file),
        )
        .route(
            "/v1/sandboxes/:id/snapshot",
            post(handlers::snapshot_sandbox),
        )
        .route("/v1/sandboxes/:id", delete(handlers::destroy_sandbox))
        .route(
            "/v1/admin/tenants/:id/usage",
            get(handlers::inspect_tenant_usage),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}
