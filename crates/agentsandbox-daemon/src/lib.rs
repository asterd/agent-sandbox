//! AgentSandbox HTTP daemon — library surface.
//!
//! The binary in `src/main.rs` is a thin wrapper that boots logging and the
//! HTTP server. Everything testable (state, handlers, error envelope,
//! persistence, reaper) lives here so it can be exercised without spawning
//! the process.

pub mod audit;
pub mod config;
pub mod error;
pub mod handlers;
pub mod reaper;
pub mod registry;
pub mod router;
pub mod state;
pub mod store;

pub use error::{ApiError, ApiErrorCode};
pub use state::AppState;
