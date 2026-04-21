//! HTTP handlers for the v1 API.
//!
//! Contract is documented in `docs/api-http-v1.md`. Every handler converts
//! its result into a JSON body and lets the [`ApiError`] extractor do the
//! status-code mapping; handlers never call `StatusCode` directly.

use agentsandbox_core::compile_value;
use agentsandbox_sdk::backend::{BackendCapabilities, SandboxState};
use agentsandbox_sdk::error::BackendError;
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
    let backends: Vec<_> = state
        .registry
        .list_available()
        .into_iter()
        .map(|descriptor| descriptor.id)
        .collect();
    let primary_backend = backends.first().copied().unwrap_or("unavailable");
    Json(json!({
        "status": "ok",
        "backend": primary_backend,
        "backends": backends,
    }))
}

#[derive(Serialize)]
pub struct BackendResponse {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub trait_version: String,
    pub capabilities: BackendCapabilitiesResponse,
    pub extensions_supported: bool,
}

#[derive(Serialize)]
pub struct BackendCapabilitiesResponse {
    pub network_isolation: bool,
    pub memory_hard_limit: bool,
    pub cpu_hard_limit: bool,
    pub persistent_storage: bool,
    pub self_contained: bool,
    pub isolation_level: String,
    pub supported_presets: Vec<String>,
    pub rootless: bool,
    pub snapshot_restore: bool,
}

impl From<&BackendCapabilities> for BackendCapabilitiesResponse {
    fn from(value: &BackendCapabilities) -> Self {
        Self {
            network_isolation: value.network_isolation,
            memory_hard_limit: value.memory_hard_limit,
            cpu_hard_limit: value.cpu_hard_limit,
            persistent_storage: value.persistent_storage,
            self_contained: value.self_contained,
            isolation_level: format!("{:?}", value.isolation_level),
            supported_presets: value
                .supported_presets
                .iter()
                .map(|preset| (*preset).to_string())
                .collect(),
            rootless: value.rootless,
            snapshot_restore: value.snapshot_restore,
        }
    }
}

pub async fn list_backends(State(state): State<SharedState>) -> Json<Value> {
    let items: Vec<_> = state
        .registry
        .list_available()
        .into_iter()
        .map(|descriptor| {
            serde_json::to_value(BackendResponse {
                id: descriptor.id.to_string(),
                display_name: descriptor.display_name.to_string(),
                version: descriptor.version.to_string(),
                trait_version: descriptor.trait_version().to_string(),
                capabilities: BackendCapabilitiesResponse::from(&descriptor.capabilities),
                extensions_supported: descriptor.extensions_schema.is_some(),
            })
            .expect("backend descriptor deve essere serializzabile")
        })
        .collect();

    Json(json!({ "items": items }))
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
fn parse_spec_body(headers: &HeaderMap, body: &[u8]) -> Result<Value, ApiError> {
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
    let raw_spec = parse_spec_body(&headers, &body)?;
    // Keep the original submission for audit — reserialise as JSON so the
    // DB column format stays stable even when clients sent YAML.
    let spec_json = serde_json::to_string(&raw_spec)?;
    let ir = compile_value(raw_spec)?;

    let lease_token = uuid::Uuid::new_v4().to_string();
    let (backend_id, backend) = state
        .registry
        .select(&ir)
        .map_err(|e| ApiError::new(crate::error::ApiErrorCode::BackendUnavailable, e.to_string()))?;
    backend.can_satisfy(&ir).await?;

    let row = store::insert_sandbox(
        &state.db,
        store::NewSandbox {
            id: &ir.id,
            lease_token: &lease_token,
            backend: &backend_id,
            spec_json: &spec_json,
            ir: &ir,
            ttl_seconds: ir.ttl_seconds,
        },
    )
    .await?;

    // Create the actual backend resource. On failure, mark the DB row as
    // error and surface the adapter error. We don't delete the row: the
    // audit trail is more useful than a clean table.
    match backend.create(&ir).await {
        Ok(handle) => {
            if let Err(error) =
                persist_created_sandbox(&state, &backend, &backend_id, &ir.id, &handle).await
            {
                return Err(error);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            store::set_status(&state.db, &ir.id, SandboxState::Failed(msg.clone())).await?;
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
            backend: backend_id,
        }),
    ))
}

async fn persist_created_sandbox(
    state: &SharedState,
    backend: &std::sync::Arc<dyn agentsandbox_sdk::backend::SandboxBackend>,
    backend_id: &str,
    sandbox_id: &str,
    handle: &str,
) -> Result<(), ApiError> {
    if let Err(error) = store::set_backend_handle(&state.db, sandbox_id, handle).await {
        cleanup_after_persist_failure(state, backend, backend_id, sandbox_id, handle, &error).await;
        return Err(error);
    }

    if let Err(error) = store::set_status(&state.db, sandbox_id, SandboxState::Running).await {
        cleanup_after_persist_failure(state, backend, backend_id, sandbox_id, handle, &error).await;
        return Err(error);
    }

    audit::record(&state.db, sandbox_id, Event::Created, Some(backend_id)).await;
    Ok(())
}

async fn cleanup_after_persist_failure(
    state: &SharedState,
    backend: &std::sync::Arc<dyn agentsandbox_sdk::backend::SandboxBackend>,
    backend_id: &str,
    sandbox_id: &str,
    handle: &str,
    error: &ApiError,
) {
    tracing::error!(
        sandbox_id = %sandbox_id,
        backend_id = %backend_id,
        backend_handle = %handle,
        persist_error = %error,
        "persistenza sandbox fallita dopo create; cleanup backend in corso"
    );

    if let Err(destroy_error) = backend.destroy(handle).await {
        tracing::error!(
            sandbox_id = %sandbox_id,
            backend_id = %backend_id,
            backend_handle = %handle,
            error = %destroy_error,
            "cleanup backend fallito dopo persist error"
        );
    }

    let message = error.to_string();
    if let Err(status_error) = store::set_status(
        &state.db,
        sandbox_id,
        SandboxState::Failed(format!("persist error dopo create: {message}")),
    )
    .await
    {
        tracing::error!(
            sandbox_id = %sandbox_id,
            error = %status_error,
            "impossibile marcare la sandbox come failed dopo cleanup"
        );
    }
    audit::record(&state.db, sandbox_id, Event::Error, Some(&message)).await;
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

fn status_from_backend(status: SandboxState) -> String {
    match status {
        SandboxState::Creating => "creating".into(),
        SandboxState::Running => "running".into(),
        SandboxState::Stopped => "stopped".into(),
        SandboxState::Expired => "expired".into(),
        SandboxState::Failed(message) => {
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

    let backend = state
        .registry
        .get(&row.backend)
        .map_err(|e| ApiError::new(crate::error::ApiErrorCode::BackendUnavailable, e.to_string()))?;

    match backend.status(row.runtime_handle()).await {
        Ok(info) => {
            let observed_status = status_from_backend(info.state);
            if observed_status != row.status {
                store::set_status(
                    &state.db,
                    &row.id,
                    match observed_status.as_str() {
                        "creating" => SandboxState::Creating,
                        "running" => SandboxState::Running,
                        "stopped" => SandboxState::Stopped,
                        "expired" => SandboxState::Expired,
                        _ => SandboxState::Failed("backend reported failure".into()),
                    },
                )
                .await?;
            }
            Ok(SandboxRow {
                status: observed_status,
                ..row
            })
        }
        Err(BackendError::NotFound(_)) => {
            store::set_status(&state.db, &row.id, SandboxState::Stopped).await?;
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
    Ok(Json(
        json!({ "items": items, "limit": limit, "offset": offset }),
    ))
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

async fn require_lease(state: &SharedState, id: &str, headers: &HeaderMap) -> Result<(), ApiError> {
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

    let backend = state
        .registry
        .get(&row.backend)
        .map_err(|e| ApiError::new(crate::error::ApiErrorCode::BackendUnavailable, e.to_string()))?;
    let result = backend.exec(row.runtime_handle(), &req.command, None).await?;
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

    if let Some(row) = store::get_sandbox(&state.db, &id).await? {
        let backend = state
            .registry
            .get(&row.backend)
            .map_err(|e| ApiError::new(crate::error::ApiErrorCode::BackendUnavailable, e.to_string()))?;
        backend.destroy(row.runtime_handle()).await?;
    }
    store::delete_sandbox(&state.db, &id).await?;
    audit::record(&state.db, &id, Event::Destroyed, None).await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use agentsandbox_sdk::{
        backend::{
            BackendCapabilities, BackendDescriptor, BackendFactory, ExecResult, IsolationLevel,
            SandboxBackend, SandboxState, SandboxStatus,
        },
        error::BackendError,
        ir::SandboxIR,
    };
    use chrono::{Duration, Utc};
    use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
    use std::str::FromStr;
    use tower::ServiceExt;

    use crate::{registry::BackendRegistry, router, state::AppState};

    enum InspectBehavior {
        Status(SandboxState),
        NotFound,
    }

    struct MockBackend {
        inspect_behavior: InspectBehavior,
        destroyed: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl SandboxBackend for MockBackend {
        async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
            Ok(format!("handle-{}", ir.id))
        }

        async fn exec(
            &self,
            _handle: &str,
            _command: &str,
            _timeout_ms: Option<u64>,
        ) -> Result<ExecResult, BackendError> {
            Err(BackendError::Internal("unused".into()))
        }

        async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
            match &self.inspect_behavior {
                InspectBehavior::Status(status) => Ok(SandboxStatus {
                    sandbox_id: handle.into(),
                    state: status.clone(),
                    created_at: Utc::now(),
                    expires_at: Utc::now(),
                    backend_id: "mock".into(),
                }),
                InspectBehavior::NotFound => Err(BackendError::NotFound("missing".into())),
            }
        }

        async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
            self.destroyed.lock().unwrap().push(handle.to_string());
            Ok(())
        }

        async fn health_check(&self) -> Result<(), BackendError> {
            Ok(())
        }
    }

    struct MockFactory {
        inspect_behavior: InspectBehavior,
        destroyed: Arc<Mutex<Vec<String>>>,
    }

    impl BackendFactory for MockFactory {
        fn describe(&self) -> BackendDescriptor {
            BackendDescriptor {
                id: "mock",
                display_name: "Mock",
                version: "test",
                trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
                capabilities: BackendCapabilities {
                    isolation_level: IsolationLevel::Process,
                    ..BackendCapabilities::default()
                },
                extensions_schema: None,
            }
        }

        fn create(
            &self,
            _config: &HashMap<String, String>,
        ) -> Result<Box<dyn SandboxBackend>, BackendError> {
            Ok(Box::new(MockBackend {
                inspect_behavior: match &self.inspect_behavior {
                    InspectBehavior::Status(state) => InspectBehavior::Status(state.clone()),
                    InspectBehavior::NotFound => InspectBehavior::NotFound,
                },
                destroyed: self.destroyed.clone(),
            }))
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

    async fn test_state(
        db: SqlitePool,
        inspect_behavior: InspectBehavior,
    ) -> (Arc<AppState>, Arc<Mutex<Vec<String>>>) {
        let mut registry = BackendRegistry::new();
        let destroyed = Arc::new(Mutex::new(Vec::new()));
        let factory = MockFactory {
            inspect_behavior,
            destroyed: destroyed.clone(),
        };
        registry.register(&factory);
        registry.initialize(&factory, &HashMap::new()).await;
        (
            Arc::new(AppState {
                db,
                registry: Arc::new(registry),
            }),
            destroyed,
        )
    }

    async fn insert_running_row(pool: &SqlitePool, id: &str) {
        let created_at = Utc::now();
        let expires_at = created_at + Duration::seconds(60);
        sqlx::query(
            "INSERT INTO sandboxes \
             (id, lease_token, status, backend, backend_handle, spec_json, ir_json, created_at, expires_at) \
             VALUES (?1, ?2, 'running', 'mock', ?3, '{}', '{}', ?4, ?5)",
        )
        .bind(id)
        .bind("lease")
        .bind(format!("handle-{id}"))
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
        let (state, _) =
            test_state(db.clone(), InspectBehavior::Status(SandboxState::Stopped)).await;

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
        let (state, _) = test_state(db.clone(), InspectBehavior::NotFound).await;

        let fresh = refresh_runtime_status(&state, row).await.unwrap();
        assert_eq!(fresh.status, "stopped");

        let persisted = store::get_sandbox(&db, "sb-2").await.unwrap().unwrap();
        assert_eq!(persisted.status, "stopped");
    }

    #[tokio::test]
    async fn create_sandbox_accepts_v1_json() {
        let db = test_db().await;
        let (state, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sandboxes")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"apiVersion":"sandbox.ai/v1","kind":"Sandbox","metadata":{},"spec":{"runtime":{"preset":"python"}}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_sandbox_accepts_v1_yaml() {
        let db = test_db().await;
        let (state, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sandboxes")
                    .header(header::CONTENT_TYPE, "application/yaml")
                    .body(Body::from(
                        "apiVersion: sandbox.ai/v1\nkind: Sandbox\nmetadata: {}\nspec:\n  runtime:\n    preset: python\n",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    }

    #[tokio::test]
    async fn create_sandbox_returns_structured_schema_errors() {
        let db = test_db().await;
        let (state, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sandboxes")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{
                            "apiVersion":"sandbox.ai/v1",
                            "kind":"Sandbox",
                            "metadata":{},
                            "spec":{
                                "runtime":{"preset":"python"},
                                "resources":{"cpuMillicores":-1},
                                "network":{"egress":{"mode":"bogus"}}
                            }
                        }"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let details = &payload["error"]["details"]["validationErrors"];
        assert!(details.is_array());
        assert!(details.as_array().unwrap().len() >= 2);
    }

    #[tokio::test]
    async fn create_sandbox_cleans_up_backend_if_handle_persist_fails() {
        let db = test_db().await;
        sqlx::query(
            "CREATE TRIGGER fail_backend_handle_update \
             BEFORE UPDATE OF backend_handle ON sandboxes \
             BEGIN \
               SELECT RAISE(FAIL, 'backend_handle update blocked'); \
             END;",
        )
        .execute(&db)
        .await
        .unwrap();

        let (state, destroyed) = test_state(db.clone(), InspectBehavior::Status(SandboxState::Running)).await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sandboxes")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"apiVersion":"sandbox.ai/v1","kind":"Sandbox","metadata":{},"spec":{"runtime":{"preset":"python"}}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);

        let destroyed = destroyed.lock().unwrap().clone();
        assert_eq!(destroyed.len(), 1);
        assert!(destroyed[0].starts_with("handle-"));
    }
}
