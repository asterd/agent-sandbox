//! HTTP handlers for the v1 API.
//!
//! Contract is documented in `docs/api-http-v1.md`. Every handler converts
//! its result into a JSON body and lets the [`ApiError`] extractor do the
//! status-code mapping; handlers never call `StatusCode` directly.

use agentsandbox_core::{compile, AdapterError, SandboxStatus};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap},
    Json,
};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::audit::{self, Event};
use crate::error::ApiError;
use crate::state::SharedState;
use crate::store::{self, SandboxRow};

const LEASE_HEADER: &str = "X-Lease-Token";

// ---------- /v1/health ----------

pub async fn health(State(state): State<SharedState>) -> Json<Value> {
    Json(json!({ "status": "ok", "backend": state.adapter.backend_name() }))
}

// ---------- POST /v1/sandboxes ----------

#[derive(Serialize)]
pub struct CreateResponse {
    pub sandbox_id: String,
    pub lease_token: String,
    pub status: String,
    pub expires_at: String,
    pub backend: String,
}

/// Accept either `application/json` or `application/yaml` (+ `text/yaml`).
/// Content-Type drives parsing; absent/unknown is treated as JSON.
fn parse_spec_body(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<agentsandbox_core::SandboxSpec, ApiError> {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    if ct.contains("yaml") {
        Ok(serde_yaml::from_slice(body)?)
    } else {
        Ok(serde_json::from_slice(body)?)
    }
}

pub async fn create_sandbox(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(axum::http::StatusCode, Json<CreateResponse>), ApiError> {
    let spec = parse_spec_body(&headers, &body)?;
    // Keep the original submission for audit — reserialise as JSON so the
    // DB column format stays stable even when clients sent YAML.
    let spec_json = serde_json::to_string(&spec)?;
    let ir = compile(spec)?;

    let lease_token = uuid::Uuid::new_v4().to_string();
    let backend = state.adapter.backend_name();

    let row = store::insert_sandbox(
        &state.db,
        store::NewSandbox {
            id: &ir.id,
            lease_token: &lease_token,
            backend,
            spec_json: &spec_json,
            ir: &ir,
            ttl_seconds: ir.ttl_seconds,
        },
    )
    .await?;

    // Create the actual backend resource. On failure, mark the DB row as
    // error and surface the adapter error. We don't delete the row: the
    // audit trail is more useful than a clean table.
    match state.adapter.create(&ir).await {
        Ok(_) => {
            store::set_status(&state.db, &ir.id, SandboxStatus::Running).await?;
            audit::record(&state.db, &ir.id, Event::Created, Some(backend)).await;
        }
        Err(e) => {
            let msg = e.to_string();
            store::set_status(&state.db, &ir.id, SandboxStatus::Error(msg.clone())).await?;
            audit::record(&state.db, &ir.id, Event::Error, Some(&msg)).await;
            return Err(e.into());
        }
    }

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateResponse {
            sandbox_id: row.id,
            lease_token: row.lease_token,
            status: "running".into(),
            expires_at: row.expires_at.to_rfc3339(),
            backend: backend.to_string(),
        }),
    ))
}

// ---------- GET /v1/sandboxes/:id ----------

#[derive(Serialize)]
pub struct InspectResponse {
    pub sandbox_id: String,
    pub status: String,
    pub backend: String,
    pub created_at: String,
    pub expires_at: String,
    pub error_message: Option<String>,
}

fn is_backend_observed_status(status: &str) -> bool {
    matches!(status, "creating" | "running")
}

fn status_from_adapter(status: SandboxStatus) -> String {
    match status {
        SandboxStatus::Creating => "creating".into(),
        SandboxStatus::Running => "running".into(),
        SandboxStatus::Stopped => "stopped".into(),
        SandboxStatus::Error(message) => {
            tracing::warn!(error = %message, "backend reported error state");
            "error".into()
        }
    }
}

async fn refresh_runtime_status(
    state: &SharedState,
    row: SandboxRow,
) -> Result<SandboxRow, ApiError> {
    if !is_backend_observed_status(&row.status) {
        return Ok(row);
    }

    match state.adapter.inspect(&row.id).await {
        Ok(info) => {
            let observed_status = status_from_adapter(info.status);
            if observed_status != row.status {
                store::set_status(
                    &state.db,
                    &row.id,
                    match observed_status.as_str() {
                        "creating" => SandboxStatus::Creating,
                        "running" => SandboxStatus::Running,
                        "stopped" => SandboxStatus::Stopped,
                        _ => SandboxStatus::Error("backend reported failure".into()),
                    },
                )
                .await?;
            }
            Ok(SandboxRow {
                status: observed_status,
                ..row
            })
        }
        Err(AdapterError::NotFound(_)) => {
            store::set_status(&state.db, &row.id, SandboxStatus::Stopped).await?;
            Ok(SandboxRow {
                status: "stopped".into(),
                ..row
            })
        }
        Err(err) => Err(err.into()),
    }
}

pub async fn inspect_sandbox(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<InspectResponse>, ApiError> {
    let row = store::get_sandbox(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::not_found(&id))?;
    let row = refresh_runtime_status(&state, row).await?;

    Ok(Json(InspectResponse {
        sandbox_id: row.id,
        status: row.status,
        backend: row.backend,
        created_at: row.created_at.to_rfc3339(),
        expires_at: row.expires_at.to_rfc3339(),
        error_message: row.error_message,
    }))
}

// ---------- GET /v1/sandboxes ----------

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

pub async fn list_sandboxes(
    State(state): State<SharedState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Value>, ApiError> {
    let limit = params.limit.clamp(1, 500);
    let offset = params.offset.max(0);
    let rows = store::list_active(&state.db, limit, offset).await?;
    let items: Vec<_> = rows
        .into_iter()
        .map(|row| async {
            match refresh_runtime_status(&state, row).await {
                Ok(fresh) if is_backend_observed_status(&fresh.status) => Some(InspectResponse {
                    sandbox_id: fresh.id,
                    status: fresh.status,
                    backend: fresh.backend,
                    created_at: fresh.created_at.to_rfc3339(),
                    expires_at: fresh.expires_at.to_rfc3339(),
                    error_message: fresh.error_message,
                }),
                Ok(_) => None,
                Err(err) => {
                    tracing::warn!(error = %err, "list_sandboxes could not refresh backend state");
                    None
                }
            }
        })
        .collect::<FuturesUnordered<_>>()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect();
    Ok(Json(json!({ "items": items, "limit": limit, "offset": offset })))
}

// ---------- POST /v1/sandboxes/:id/exec ----------

#[derive(Deserialize)]
pub struct ExecRequest {
    pub command: String,
}

#[derive(Serialize)]
pub struct ExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
}

async fn require_lease(
    state: &SharedState,
    id: &str,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let token = headers
        .get(LEASE_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::lease_invalid)?;
    if !store::verify_lease(&state.db, id, token).await? {
        return Err(ApiError::lease_invalid());
    }
    Ok(())
}

pub async fn exec_sandbox(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecRequest>,
) -> Result<Json<ExecResponse>, ApiError> {
    require_lease(&state, &id, &headers).await?;

    let row = store::get_sandbox(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::not_found(&id))?;
    if row.status != "running" {
        return Err(ApiError::new(
            crate::error::ApiErrorCode::SandboxExpired,
            format!("sandbox {id} non è in esecuzione (status={})", row.status),
        ));
    }

    let result = state.adapter.exec(&id, &req.command).await?;
    audit::record(
        &state.db,
        &id,
        Event::Exec,
        Some(&format!(
            "exit={} duration_ms={}",
            result.exit_code, result.duration_ms
        )),
    )
    .await;

    Ok(Json(ExecResponse {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code,
        duration_ms: result.duration_ms,
    }))
}

// ---------- DELETE /v1/sandboxes/:id ----------

pub async fn destroy_sandbox(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, ApiError> {
    // Destroy accepts the lease when present but also works without it when
    // the sandbox row doesn't exist (idempotent cleanup by id). When the row
    // DOES exist the lease is required — otherwise anyone could kill it.
    if store::get_sandbox(&state.db, &id).await?.is_some() {
        require_lease(&state, &id, &headers).await?;
    }

    state.adapter.destroy(&id).await?;
    store::delete_sandbox(&state.db, &id).await?;
    audit::record(&state.db, &id, Event::Destroyed, None).await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use agentsandbox_core::{ExecResult, SandboxAdapter, SandboxInfo};
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
    use std::str::FromStr;

    use crate::state::AppState;

    enum InspectBehavior {
        Status(SandboxStatus),
        NotFound,
    }

    struct MockAdapter {
        inspect_behavior: InspectBehavior,
    }

    #[async_trait]
    impl SandboxAdapter for MockAdapter {
        async fn create(&self, _ir: &agentsandbox_core::SandboxIR) -> Result<String, AdapterError> {
            Err(AdapterError::Internal("unused".into()))
        }

        async fn exec(&self, _sandbox_id: &str, _command: &str) -> Result<ExecResult, AdapterError> {
            Err(AdapterError::Internal("unused".into()))
        }

        async fn inspect(&self, _sandbox_id: &str) -> Result<SandboxInfo, AdapterError> {
            match &self.inspect_behavior {
                InspectBehavior::Status(status) => Ok(SandboxInfo {
                    sandbox_id: "test".into(),
                    status: status.clone(),
                    created_at: Utc::now(),
                    expires_at: Utc::now(),
                }),
                InspectBehavior::NotFound => Err(AdapterError::NotFound("missing".into())),
            }
        }

        async fn destroy(&self, _sandbox_id: &str) -> Result<(), AdapterError> {
            Ok(())
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }

        async fn health_check(&self) -> Result<(), AdapterError> {
            Ok(())
        }
    }

    async fn test_db() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn insert_running_row(pool: &SqlitePool, id: &str) {
        let created_at = Utc::now();
        let expires_at = created_at + Duration::seconds(60);
        sqlx::query(
            "INSERT INTO sandboxes \
             (id, lease_token, status, backend, spec_json, ir_json, created_at, expires_at) \
             VALUES (?1, ?2, 'running', 'mock', '{}', '{}', ?3, ?4)",
        )
        .bind(id)
        .bind("lease")
        .bind(created_at.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn refresh_runtime_status_updates_row_when_backend_stopped() {
        let db = test_db().await;
        insert_running_row(&db, "sb-1").await;
        let row = store::get_sandbox(&db, "sb-1").await.unwrap().unwrap();
        let state = Arc::new(AppState {
            db: db.clone(),
            adapter: Arc::new(MockAdapter {
                inspect_behavior: InspectBehavior::Status(SandboxStatus::Stopped),
            }),
        });

        let fresh = refresh_runtime_status(&state, row).await.unwrap();
        assert_eq!(fresh.status, "stopped");

        let persisted = store::get_sandbox(&db, "sb-1").await.unwrap().unwrap();
        assert_eq!(persisted.status, "stopped");
    }

    #[tokio::test]
    async fn refresh_runtime_status_marks_missing_backend_as_stopped() {
        let db = test_db().await;
        insert_running_row(&db, "sb-2").await;
        let row = store::get_sandbox(&db, "sb-2").await.unwrap().unwrap();
        let state = Arc::new(AppState {
            db: db.clone(),
            adapter: Arc::new(MockAdapter {
                inspect_behavior: InspectBehavior::NotFound,
            }),
        });

        let fresh = refresh_runtime_status(&state, row).await.unwrap();
        assert_eq!(fresh.status, "stopped");

        let persisted = store::get_sandbox(&db, "sb-2").await.unwrap().unwrap();
        assert_eq!(persisted.status, "stopped");
    }
}
