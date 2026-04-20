//! Router wiring. Isolated so tests can mount the same routes against an
//! in-memory SQLite DB and a mock adapter.

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::{handlers, state::SharedState};

pub fn build(state: SharedState) -> Router {
    Router::new()
        .route("/v1/health", get(handlers::health))
        .route("/v1/sandboxes", post(handlers::create_sandbox))
        .route("/v1/sandboxes", get(handlers::list_sandboxes))
        .route("/v1/sandboxes/:id", get(handlers::inspect_sandbox))
        .route("/v1/sandboxes/:id/exec", post(handlers::exec_sandbox))
        .route("/v1/sandboxes/:id", delete(handlers::destroy_sandbox))
        .with_state(state)
}
