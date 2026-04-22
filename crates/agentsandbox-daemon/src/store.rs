//! SQLite persistence for the daemon.
//!
//! We use `sqlx::query` at runtime (not the compile-time `query!` macro) so
//! that `cargo build` doesn't need `DATABASE_URL` or an existing DB. The
//! trade-off is no query-time checking; we compensate with tight types and
//! unit-tested helpers.

use agentsandbox_sdk::{
    backend::SandboxState,
    ir::{AuditLevel, EgressMode, SandboxIR, SchedulingPriority},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    pub allow_ips: Vec<String>,
    pub deny_by_default: bool,
    pub egress_mode: EgressMode,
    pub ttl_seconds: u64,
    pub timeout_ms: u64,
    pub working_dir: String,
    pub labels: std::collections::HashMap<String, String>,
    pub extensions: Option<serde_json::Value>,
    pub runtime_version: Option<String>,
    pub backend_hint: Option<String>,
    pub prefer_warm: bool,
    pub priority: Option<SchedulingPriority>,
    pub storage_volumes: Vec<serde_json::Value>,
    pub audit_level: Option<AuditLevel>,
    pub metrics_enabled: bool,
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
            egress_allow: ir.egress.allow_hostnames.clone(),
            allow_ips: ir.egress.allow_ips.clone(),
            deny_by_default: ir.egress.deny_by_default,
            egress_mode: ir.egress.mode.clone(),
            ttl_seconds: ir.ttl_seconds,
            timeout_ms: ir.timeout_ms,
            working_dir: ir.working_dir.clone(),
            labels: ir.labels.clone(),
            extensions: ir.extensions.clone(),
            runtime_version: ir.runtime_version.clone(),
            backend_hint: ir.backend_hint.clone(),
            prefer_warm: ir.prefer_warm,
            priority: ir.priority,
            storage_volumes: ir.storage_volumes.clone(),
            audit_level: ir.audit_level,
            metrics_enabled: ir.metrics_enabled,
        }
    }
}

/// Row in the `sandboxes` table, as returned to API consumers.
#[derive(Debug, Clone)]
pub struct SandboxRow {
    pub id: String,
    pub tenant_id: Option<String>,
    pub lease_token: String,
    pub status: String,
    pub backend: String,
    pub backend_handle: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub error_message: Option<String>,
}

impl SandboxRow {
    pub fn runtime_handle(&self) -> &str {
        self.backend_handle.as_deref().unwrap_or(&self.id)
    }

    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            tenant_id: row.try_get("tenant_id")?,
            lease_token: row.try_get("lease_token")?,
            status: row.try_get("status")?,
            backend: row.try_get("backend")?,
            backend_handle: row.try_get("backend_handle")?,
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
    pub tenant_id: Option<&'a str>,
    pub lease_token: &'a str,
    pub backend: &'a str,
    pub spec_json: &'a str,
    pub ir: &'a SandboxIR,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct TenantRecord {
    pub id: String,
    pub quota_hourly: i64,
    pub quota_concurrent: i64,
    pub allowed_backends: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum AccessScope<'a> {
    All,
    Tenant(&'a str),
}

fn api_key_hash(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn current_window_start(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%dT%H:00:00+00:00").to_string()
}

pub async fn insert_sandbox(
    pool: &SqlitePool,
    new: NewSandbox<'_>,
) -> Result<SandboxRow, ApiError> {
    let created_at = Utc::now();
    let expires_at = created_at + chrono::Duration::seconds(new.ttl_seconds as i64);
    let ir_json = serde_json::to_string(&StoredIr::from(new.ir))?;

    sqlx::query(
        "INSERT INTO sandboxes \
         (id, tenant_id, lease_token, status, backend, backend_handle, spec_json, ir_json, created_at, expires_at) \
         VALUES (?1, ?2, ?3, 'creating', ?4, NULL, ?5, ?6, ?7, ?8)",
    )
    .bind(new.id)
    .bind(new.tenant_id)
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
        tenant_id: new.tenant_id.map(ToOwned::to_owned),
        lease_token: new.lease_token.to_string(),
        status: "creating".into(),
        backend: new.backend.to_string(),
        backend_handle: None,
        created_at,
        expires_at,
        error_message: None,
    })
}

pub async fn set_backend_handle(
    pool: &SqlitePool,
    id: &str,
    backend_handle: &str,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE sandboxes SET backend_handle = ?1 WHERE id = ?2")
        .bind(backend_handle)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_status(pool: &SqlitePool, id: &str, status: SandboxState) -> Result<(), ApiError> {
    let (status_str, err) = match status {
        SandboxState::Failed(msg) => ("error".to_string(), Some(msg)),
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

pub async fn get_sandbox_scoped(
    pool: &SqlitePool,
    id: &str,
    scope: AccessScope<'_>,
) -> Result<Option<SandboxRow>, ApiError> {
    let row = match scope {
        AccessScope::All => {
            sqlx::query("SELECT * FROM sandboxes WHERE id = ?1")
                .bind(id)
                .fetch_optional(pool)
                .await?
        }
        AccessScope::Tenant(tenant_id) => {
            sqlx::query("SELECT * FROM sandboxes WHERE id = ?1 AND tenant_id = ?2")
                .bind(id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?
        }
    };

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
    rows.iter()
        .map(SandboxRow::from_row)
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

pub async fn list_active_scoped(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    scope: AccessScope<'_>,
) -> Result<Vec<SandboxRow>, ApiError> {
    let rows = match scope {
        AccessScope::All => {
            sqlx::query(
                "SELECT * FROM sandboxes \
                 WHERE status IN ('creating','running') \
                 ORDER BY created_at DESC \
                 LIMIT ?1 OFFSET ?2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        AccessScope::Tenant(tenant_id) => {
            sqlx::query(
                "SELECT * FROM sandboxes \
                 WHERE tenant_id = ?1 AND status IN ('creating','running') \
                 ORDER BY created_at DESC \
                 LIMIT ?2 OFFSET ?3",
            )
            .bind(tenant_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    };

    rows.iter()
        .map(SandboxRow::from_row)
        .collect::<Result<_, _>>()
        .map_err(Into::into)
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

pub async fn verify_lease_scoped(
    pool: &SqlitePool,
    id: &str,
    token: &str,
    scope: AccessScope<'_>,
) -> Result<bool, ApiError> {
    let row = match scope {
        AccessScope::All => {
            sqlx::query("SELECT lease_token FROM sandboxes WHERE id = ?1")
                .bind(id)
                .fetch_optional(pool)
                .await?
        }
        AccessScope::Tenant(tenant_id) => {
            sqlx::query("SELECT lease_token FROM sandboxes WHERE id = ?1 AND tenant_id = ?2")
                .bind(id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?
        }
    };
    Ok(row
        .and_then(|r| r.try_get::<String, _>("lease_token").ok())
        .is_some_and(|t| t == token))
}

pub async fn verify_api_key(
    pool: &SqlitePool,
    key: &str,
) -> Result<Option<TenantRecord>, ApiError> {
    let row = sqlx::query(
        "SELECT id, quota_hourly, quota_concurrent \
         FROM tenants WHERE api_key_hash = ?1 AND enabled = 1",
    )
    .bind(api_key_hash(key))
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| TenantRecord {
        id: row.get("id"),
        quota_hourly: row.get("quota_hourly"),
        quota_concurrent: row.get("quota_concurrent"),
        allowed_backends: Vec::new(),
    }))
}

pub async fn consume_hourly_quota(
    pool: &SqlitePool,
    tenant_id: &str,
    quota_hourly: i64,
) -> Result<(), ApiError> {
    let window_start = current_window_start(Utc::now());
    sqlx::query(
        "INSERT INTO rate_limit_windows (tenant_id, window_start, count) \
         VALUES (?1, ?2, 0) \
         ON CONFLICT(tenant_id, window_start) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(&window_start)
    .execute(pool)
    .await?;

    let result = sqlx::query(
        "UPDATE rate_limit_windows \
         SET count = count + 1 \
         WHERE tenant_id = ?1 AND window_start = ?2 AND count < ?3",
    )
    .bind(tenant_id)
    .bind(&window_start)
    .bind(quota_hourly)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::rate_limited("quota oraria superata"));
    }

    Ok(())
}

pub async fn consume_concurrent_slot(
    pool: &SqlitePool,
    tenant_id: &str,
    quota_concurrent: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO tenant_usage (tenant_id, concurrent_in_use) VALUES (?1, 0) \
         ON CONFLICT(tenant_id) DO NOTHING",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    let result = sqlx::query(
        "UPDATE tenant_usage SET concurrent_in_use = concurrent_in_use + 1 \
         WHERE tenant_id = ?1 AND concurrent_in_use < ?2",
    )
    .bind(tenant_id)
    .bind(quota_concurrent)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::rate_limited("quota concorrente superata"));
    }

    Ok(())
}

pub async fn release_concurrent_slot(pool: &SqlitePool, tenant_id: &str) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE tenant_usage \
         SET concurrent_in_use = CASE \
             WHEN concurrent_in_use > 0 THEN concurrent_in_use - 1 \
             ELSE 0 \
         END \
         WHERE tenant_id = ?1",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn reconcile_concurrent_usage(pool: &SqlitePool) -> Result<(), ApiError> {
    let rows = sqlx::query(
        "SELECT tenant_id, COUNT(*) AS active_count \
         FROM sandboxes \
         WHERE tenant_id IS NOT NULL AND status IN ('creating', 'running') \
         GROUP BY tenant_id",
    )
    .fetch_all(pool)
    .await?;

    sqlx::query("DELETE FROM tenant_usage")
        .execute(pool)
        .await?;
    for row in rows {
        let tenant_id: String = row.get("tenant_id");
        let active_count: i64 = row.get("active_count");
        sqlx::query("INSERT INTO tenant_usage (tenant_id, concurrent_in_use) VALUES (?1, ?2)")
            .bind(tenant_id)
            .bind(active_count)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn get_tenant_usage(
    pool: &SqlitePool,
    tenant_id: &str,
) -> Result<Option<(i64, i64)>, ApiError> {
    let window_start = current_window_start(Utc::now());
    let row = sqlx::query(
        "SELECT \
             COALESCE((SELECT concurrent_in_use FROM tenant_usage WHERE tenant_id = ?1), 0) AS concurrent_in_use, \
             COALESCE((SELECT count FROM rate_limit_windows WHERE tenant_id = ?1 AND window_start = ?2), 0) AS hourly_count",
    )
    .bind(tenant_id)
    .bind(window_start)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| {
        (
            row.get::<i64, _>("concurrent_in_use"),
            row.get::<i64, _>("hourly_count"),
        )
    }))
}

pub async fn set_runtime_metadata(
    pool: &SqlitePool,
    key: &str,
    value: &str,
) -> Result<(), ApiError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO runtime_metadata (key, value, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn cleanup_old_records(
    pool: &SqlitePool,
    audit_retain_days: u64,
) -> Result<(), ApiError> {
    let audit_cutoff = (Utc::now() - chrono::Duration::days(audit_retain_days as i64)).to_rfc3339();
    let rate_limit_cutoff = current_window_start(Utc::now() - chrono::Duration::days(2));

    sqlx::query("DELETE FROM audit_log WHERE ts < ?1")
        .bind(audit_cutoff)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM rate_limit_windows WHERE window_start < ?1")
        .bind(rate_limit_cutoff)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn vacuum(pool: &SqlitePool) -> Result<(), ApiError> {
    sqlx::query("VACUUM").execute(pool).await?;
    Ok(())
}

#[cfg(test)]
pub fn hash_api_key_for_tests(key: &str) -> String {
    api_key_hash(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_ir_omits_secret_env_from_serialization() {
        let mut ir = SandboxIR::default();
        ir.secret_env
            .push(("API_KEY".into(), "super-secret-value".into()));

        let stored = StoredIr::from(&ir);
        let encoded = serde_json::to_string(&stored).unwrap();

        assert!(!encoded.contains("secret_env"));
        assert!(!encoded.contains("super-secret-value"));
    }

    #[test]
    fn runtime_handle_prefers_backend_handle_when_present() {
        let row = SandboxRow {
            id: "sb-1".into(),
            tenant_id: None,
            lease_token: "lease".into(),
            status: "running".into(),
            backend: "docker".into(),
            backend_handle: Some("native-123".into()),
            created_at: Utc::now(),
            expires_at: Utc::now(),
            error_message: None,
        };

        assert_eq!(row.runtime_handle(), "native-123");
    }

    #[test]
    fn runtime_handle_falls_back_to_public_id() {
        let row = SandboxRow {
            id: "sb-1".into(),
            tenant_id: None,
            lease_token: "lease".into(),
            status: "running".into(),
            backend: "docker".into(),
            backend_handle: None,
            created_at: Utc::now(),
            expires_at: Utc::now(),
            error_message: None,
        };

        assert_eq!(row.runtime_handle(), "sb-1");
    }
}
