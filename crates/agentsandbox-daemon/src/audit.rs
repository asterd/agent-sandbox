//! Append-only structured audit log.

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub sandbox_id: String,
    pub tenant_id: Option<String>,
    pub backend_id: String,
    pub event: AuditEventKind,
}

impl AuditEvent {
    pub fn sandbox_created(
        sandbox_id: &str,
        tenant_id: Option<&str>,
        backend_id: &str,
        ttl_seconds: u64,
    ) -> Self {
        Self::new(
            sandbox_id,
            tenant_id,
            backend_id,
            AuditEventKind::SandboxCreated {
                backend: backend_id.to_string(),
                ttl_seconds,
            },
        )
    }

    pub fn exec_started(
        sandbox_id: &str,
        tenant_id: Option<&str>,
        backend_id: &str,
        command: &str,
    ) -> Self {
        Self::new(
            sandbox_id,
            tenant_id,
            backend_id,
            AuditEventKind::ExecStarted {
                command_hash: command_hash(command),
            },
        )
    }

    pub fn exec_finished(
        sandbox_id: &str,
        tenant_id: Option<&str>,
        backend_id: &str,
        exit_code: i64,
        duration_ms: u64,
    ) -> Self {
        Self::new(
            sandbox_id,
            tenant_id,
            backend_id,
            AuditEventKind::ExecFinished {
                exit_code,
                duration_ms,
            },
        )
    }

    pub fn sandbox_destroyed(
        sandbox_id: &str,
        tenant_id: Option<&str>,
        backend_id: &str,
        reason: DestroyReason,
    ) -> Self {
        Self::new(
            sandbox_id,
            tenant_id,
            backend_id,
            AuditEventKind::SandboxDestroyed { reason },
        )
    }

    pub fn backend_error(
        sandbox_id: &str,
        tenant_id: Option<&str>,
        backend_id: &str,
        error: impl Into<String>,
    ) -> Self {
        Self::new(
            sandbox_id,
            tenant_id,
            backend_id,
            AuditEventKind::BackendError {
                error: error.into(),
            },
        )
    }

    fn new(
        sandbox_id: &str,
        tenant_id: Option<&str>,
        backend_id: &str,
        event: AuditEventKind,
    ) -> Self {
        Self {
            ts: Utc::now(),
            sandbox_id: sandbox_id.to_string(),
            tenant_id: tenant_id.map(ToOwned::to_owned),
            backend_id: backend_id.to_string(),
            event,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEventKind {
    SandboxCreated { backend: String, ttl_seconds: u64 },
    ExecStarted { command_hash: String },
    ExecFinished { exit_code: i64, duration_ms: u64 },
    SandboxDestroyed { reason: DestroyReason },
    EgressAllowed { hostname: String },
    EgressDenied { hostname: String },
    BackendError { error: String },
}

impl AuditEventKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SandboxCreated { .. } => "sandbox_created",
            Self::ExecStarted { .. } => "exec_started",
            Self::ExecFinished { .. } => "exec_finished",
            Self::SandboxDestroyed { .. } => "sandbox_destroyed",
            Self::EgressAllowed { .. } => "egress_allowed",
            Self::EgressDenied { .. } => "egress_denied",
            Self::BackendError { .. } => "backend_error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DestroyReason {
    ClientRequest,
    TtlExpired,
    BackendError,
}

pub fn command_hash(command: &str) -> String {
    let digest = Sha256::digest(command.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Best-effort audit insert. Failures are logged but never propagate to the
/// caller: we never let an audit failure mask or abort a real operation.
pub async fn record(pool: &SqlitePool, event: AuditEvent) {
    let event_name = event.event.as_str();
    let sandbox_id = event.sandbox_id.clone();
    let detail = match serde_json::to_string(&event) {
        Ok(detail) => detail,
        Err(error) => {
            tracing::error!(
                sandbox_id = %sandbox_id,
                event = event_name,
                error = %error,
                "audit log serialization failed"
            );
            return;
        }
    };

    let res = sqlx::query(
        "INSERT INTO audit_log (sandbox_id, event, detail, ts) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&sandbox_id)
    .bind(event_name)
    .bind(detail)
    .bind(event.ts.to_rfc3339())
    .execute(pool)
    .await;
    if let Err(error) = res {
        tracing::error!(
            sandbox_id = %sandbox_id,
            event = event_name,
            error = %error,
            "audit log insert failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_hash_does_not_expose_plaintext() {
        let command = "python -c 'print(42)'";
        let hash = command_hash(command);
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, command);
        assert!(!hash.contains("python"));
    }
}
