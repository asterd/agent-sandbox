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
        .route("/v1/sandboxes", post(handlers::create_sandbox))
        .route("/v1/sandboxes", get(handlers::list_sandboxes))
        .route("/v1/sandboxes/:id", get(handlers::inspect_sandbox))
        .route("/v1/sandboxes/:id/exec", post(handlers::exec_sandbox))
        .route("/v1/sandboxes/:id", delete(handlers::destroy_sandbox))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}
