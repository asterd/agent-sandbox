//! Append-only audit log.
//!
//! Every lifecycle operation the daemon performs produces a row here. The
//! table is write-only from the handlers' perspective — nothing in the API
//! reads it back yet, but it's the paper trail for post-mortems and SOC2.

use chrono::Utc;
use sqlx::SqlitePool;

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Created,
    Exec,
    Destroyed,
    Expired,
    Error,
}

impl Event {
    fn as_str(&self) -> &'static str {
        match self {
            Event::Created => "created",
            Event::Exec => "exec",
            Event::Destroyed => "destroyed",
            Event::Expired => "expired",
            Event::Error => "error",
        }
    }
}

/// Best-effort audit insert. Failures are logged but never propagate to the
/// caller: we never let an audit failure mask or abort a real operation.
pub async fn record(pool: &SqlitePool, sandbox_id: &str, event: Event, detail: Option<&str>) {
    let res = sqlx::query(
        "INSERT INTO audit_log (sandbox_id, event, detail, ts) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(sandbox_id)
    .bind(event.as_str())
    .bind(detail)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await;
    if let Err(e) = res {
        tracing::error!(sandbox_id = %sandbox_id, event = event.as_str(), error = %e,
                        "audit log insert failed");
    }
}
