//! Backend adapter contract.
//!
//! Every sandbox backend (Docker today, Firecracker tomorrow) implements
//! [`SandboxAdapter`]. The trait is intentionally small: the daemon owns
//! persistence, lifecycle bookkeeping, and audit; an adapter only translates
//! an [`SandboxIR`](crate::ir::SandboxIR) into concrete backend operations.
//!
//! Backend-specific types (container ids, VM handles, ...) MUST NOT leak out
//! of the adapter crate — callers only see the opaque `sandbox_id` string
//! (the IR id) and the shared [`ExecResult`] / [`SandboxInfo`] types.

use crate::ir::SandboxIR;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// Result of a single `exec` call inside a sandbox.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
}

impl ExecResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Runtime view of a sandbox as reported by the backend.
///
/// `created_at` / `expires_at` come from the adapter when it can recover them
/// (e.g. container labels); otherwise the daemon fills these in from SQLite,
/// which is the authoritative source for lifecycle metadata.
#[derive(Debug, Clone)]
pub struct SandboxInfo {
    pub sandbox_id: String,
    pub status: SandboxStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxStatus {
    Creating,
    Running,
    Stopped,
    Error(String),
}

impl SandboxStatus {
    /// Canonical lowercase label used in logs, API and DB.
    pub fn as_str(&self) -> &str {
        match self {
            SandboxStatus::Creating => "creating",
            SandboxStatus::Running => "running",
            SandboxStatus::Stopped => "stopped",
            SandboxStatus::Error(_) => "error",
        }
    }
}

#[async_trait]
pub trait SandboxAdapter: Send + Sync {
    /// Create the sandbox described by `ir` and return its id.
    ///
    /// The returned id MUST match `ir.id`; the adapter uses it as the only
    /// handle the daemon ever passes back in.
    async fn create(&self, ir: &SandboxIR) -> Result<String, AdapterError>;

    /// Run `command` inside the sandbox and return its captured output.
    ///
    /// A non-zero exit code is NOT an error: it is returned inside
    /// [`ExecResult::exit_code`]. `AdapterError` is reserved for backend
    /// failures (sandbox gone, daemon unreachable, ...).
    async fn exec(&self, sandbox_id: &str, command: &str) -> Result<ExecResult, AdapterError>;

    /// Return the current backend-observed state of the sandbox.
    async fn inspect(&self, sandbox_id: &str) -> Result<SandboxInfo, AdapterError>;

    /// Destroy the sandbox and free its resources. Destroying an already-gone
    /// sandbox MUST be a no-op (`Ok(())`), not an error — the daemon relies on
    /// this for idempotent reaping.
    async fn destroy(&self, sandbox_id: &str) -> Result<(), AdapterError>;

    /// Backend name used for logging, audit and the `/v1/health` endpoint.
    fn backend_name(&self) -> &'static str;

    /// Verify the backend is available. Called once at daemon startup.
    async fn health_check(&self) -> Result<(), AdapterError>;
}

#[derive(thiserror::Error, Debug)]
pub enum AdapterError {
    #[error("sandbox non trovata: {0}")]
    NotFound(String),
    #[error("backend non disponibile: {0}")]
    BackendUnavailable(String),
    #[error("exec fallita con exit code {exit_code}: {stderr}")]
    ExecFailed { exit_code: i64, stderr: String },
    #[error("timeout dopo {0}ms")]
    Timeout(u64),
    #[error("errore interno: {0}")]
    Internal(String),
}

impl AdapterError {
    /// Stable machine-readable code used by the HTTP error envelope.
    pub fn code(&self) -> &'static str {
        match self {
            AdapterError::NotFound(_) => "SANDBOX_NOT_FOUND",
            AdapterError::BackendUnavailable(_) => "BACKEND_UNAVAILABLE",
            AdapterError::ExecFailed { .. } => "EXEC_FAILED",
            AdapterError::Timeout(_) => "EXEC_TIMEOUT",
            AdapterError::Internal(_) => "INTERNAL_ERROR",
        }
    }
}
