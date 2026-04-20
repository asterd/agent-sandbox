//! SQLite persistence for the daemon.
//!
//! We use `sqlx::query` at runtime (not the compile-time `query!` macro) so
//! that `cargo build` doesn't need `DATABASE_URL` or an existing DB. The
//! trade-off is no query-time checking; we compensate with tight types and
//! unit-tested helpers.

use agentsandbox_core::ir::SandboxIR;
use agentsandbox_core::SandboxStatus;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::error::ApiError;

/// Serialisable view of `SandboxIR` with `secret_env` omitted.
///
/// The IR we compile holds resolved secret values; those must never touch
/// SQLite. This struct is the projection we actually persist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredIr {
    pub id: String,
    pub image: String,
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    pub cpu_millicores: u32,
    pub memory_mb: u32,
    pub disk_mb: u32,
    pub egress_allow: Vec<String>,
    pub deny_by_default: bool,
    pub ttl_seconds: u64,
    pub working_dir: String,
}

impl From<&SandboxIR> for StoredIr {
    fn from(ir: &SandboxIR) -> Self {
        Self {
            id: ir.id.clone(),
            image: ir.image.clone(),
            command: ir.command.clone(),
            env: ir.env.clone(),
            cpu_millicores: ir.cpu_millicores,
            memory_mb: ir.memory_mb,
            disk_mb: ir.disk_mb,
            egress_allow: ir.egress_allow.clone(),
            deny_by_default: ir.deny_by_default,
            ttl_seconds: ir.ttl_seconds,
            working_dir: ir.working_dir.clone(),
        }
    }
}

/// Row in the `sandboxes` table, as returned to API consumers.
#[derive(Debug, Clone)]
pub struct SandboxRow {
    pub id: String,
    pub lease_token: String,
    pub status: String,
    pub backend: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub error_message: Option<String>,
}

impl SandboxRow {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            lease_token: row.try_get("lease_token")?,
            status: row.try_get("status")?,
            backend: row.try_get("backend")?,
            created_at: parse_ts(row.try_get::<String, _>("created_at")?)?,
            expires_at: parse_ts(row.try_get::<String, _>("expires_at")?)?,
            error_message: row.try_get("error_message")?,
        })
    }
}

fn parse_ts(s: String) -> Result<DateTime<Utc>, sqlx::Error> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

/// Input for inserting a new sandbox row. Caller owns id/token generation.
pub struct NewSandbox<'a> {
    pub id: &'a str,
    pub lease_token: &'a str,
    pub backend: &'a str,
    pub spec_json: &'a str,
    pub ir: &'a SandboxIR,
    pub ttl_seconds: u64,
}

pub async fn insert_sandbox(pool: &SqlitePool, new: NewSandbox<'_>) -> Result<SandboxRow, ApiError> {
    let created_at = Utc::now();
    let expires_at = created_at + chrono::Duration::seconds(new.ttl_seconds as i64);
    let ir_json = serde_json::to_string(&StoredIr::from(new.ir))?;

    sqlx::query(
        "INSERT INTO sandboxes \
         (id, lease_token, status, backend, spec_json, ir_json, created_at, expires_at) \
         VALUES (?1, ?2, 'creating', ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(new.id)
    .bind(new.lease_token)
    .bind(new.backend)
    .bind(new.spec_json)
    .bind(&ir_json)
    .bind(created_at.to_rfc3339())
    .bind(expires_at.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(SandboxRow {
        id: new.id.to_string(),
        lease_token: new.lease_token.to_string(),
        status: "creating".into(),
        backend: new.backend.to_string(),
        created_at,
        expires_at,
        error_message: None,
    })
}

pub async fn set_status(
    pool: &SqlitePool,
    id: &str,
    status: SandboxStatus,
) -> Result<(), ApiError> {
    let (status_str, err) = match status {
        SandboxStatus::Error(msg) => ("error".to_string(), Some(msg)),
        other => (other.as_str().to_string(), None),
    };
    sqlx::query("UPDATE sandboxes SET status = ?1, error_message = ?2 WHERE id = ?3")
        .bind(status_str)
        .bind(err)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_sandbox(pool: &SqlitePool, id: &str) -> Result<Option<SandboxRow>, ApiError> {
    let row = sqlx::query("SELECT * FROM sandboxes WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(r) => Ok(Some(SandboxRow::from_row(&r)?)),
        None => Ok(None),
    }
}

pub async fn list_active(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<SandboxRow>, ApiError> {
    let rows = sqlx::query(
        "SELECT * FROM sandboxes \
         WHERE status IN ('creating','running') \
         ORDER BY created_at DESC \
         LIMIT ?1 OFFSET ?2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    rows.iter().map(SandboxRow::from_row).collect::<Result<_, _>>().map_err(Into::into)
}

pub async fn list_expired(pool: &SqlitePool, now: DateTime<Utc>) -> Result<Vec<String>, ApiError> {
    let rows = sqlx::query(
        "SELECT id FROM sandboxes \
         WHERE status IN ('creating','running') AND expires_at < ?1",
    )
    .bind(now.to_rfc3339())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| r.try_get::<String, _>("id"))
        .collect::<Result<_, _>>()?)
}

pub async fn delete_sandbox(pool: &SqlitePool, id: &str) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM sandboxes WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Verify a lease token. Returns `true` if the header matches the stored
/// token for the given sandbox id. Absent sandbox → `false`.
pub async fn verify_lease(pool: &SqlitePool, id: &str, token: &str) -> Result<bool, ApiError> {
    let row = sqlx::query("SELECT lease_token FROM sandboxes WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row
        .and_then(|r| r.try_get::<String, _>("lease_token").ok())
        .is_some_and(|t| t == token))
}
