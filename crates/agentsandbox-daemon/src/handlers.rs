//! HTTP handlers for the v1 API.
//!
//! Contract is documented in `docs/api-http-v1.md`. Every handler converts
//! its result into a JSON body and lets the [`ApiError`] extractor do the
//! status-code mapping; handlers never call `StatusCode` directly.

use agentsandbox_core::compile_value;
use agentsandbox_sdk::backend::{BackendCapabilities, SandboxState};
use agentsandbox_sdk::error::BackendError;
use axum::{
    extract::{Extension, Path, Query, State},
    http::{header, HeaderMap},
    response::IntoResponse,
    Json,
};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::audit::{self, AuditEvent, DestroyReason};
use crate::error::ApiError;
use crate::state::{AuthContext, SharedState};
use crate::store::{self, AccessScope, SandboxRow};

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

pub async fn metrics(State(state): State<SharedState>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.to_prometheus(),
    )
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

pub async fn get_backend_extensions_schema(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let descriptor = state
        .registry
        .available_descriptor(&id)
        .ok_or_else(|| ApiError::backend_not_found(&id))?;
    let raw_schema = descriptor
        .extensions_schema
        .ok_or_else(|| ApiError::backend_not_found(&id))?;
    let schema = serde_json::from_str(raw_schema)
        .map_err(|e| ApiError::internal(format!("schema extensions non valida per {id}: {e}")))?;
    Ok(Json(schema))
}

// ---------- POST /v1/sandboxes ----------

#[derive(Deserialize, Serialize)]
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

fn extension_path(raw_path: &str) -> String {
    if raw_path.is_empty() {
        "/spec/extensions".into()
    } else if raw_path.starts_with('/') {
        format!("/spec/extensions{raw_path}")
    } else {
        format!("/spec/extensions/{raw_path}")
    }
}

fn validate_backend_extensions_schema(
    backend_id: &str,
    schema_raw: &str,
    extensions: &Value,
) -> Result<(), ApiError> {
    let schema_json: Value = serde_json::from_str(schema_raw).map_err(|e| {
        ApiError::internal(format!(
            "schema extensions non valida per backend {backend_id}: {e}"
        ))
    })?;
    let compiled = jsonschema::JSONSchema::options()
        .compile(&schema_json)
        .map_err(|e| {
            ApiError::internal(format!(
                "schema extensions non compilabile per backend {backend_id}: {e}"
            ))
        })?;

    if let Err(errors) = compiled.validate(extensions) {
        let validation_errors: Vec<_> = errors
            .map(|issue| {
                json!({
                    "path": extension_path(&issue.instance_path.to_string()),
                    "message": issue.to_string(),
                })
            })
            .collect();
        return Err(ApiError::spec_invalid(format!(
            "extensions non valide per backend {backend_id}"
        ))
        .with_details(json!({
            "backend": backend_id,
            "validationErrors": validation_errors,
        })));
    }

    Ok(())
}

pub async fn create_sandbox(
    State(state): State<SharedState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(axum::http::StatusCode, Json<CreateResponse>), ApiError> {
    if let (Some(tenant_id), Some(hourly_quota)) = (auth.tenant_id(), auth.hourly_quota()) {
        store::consume_hourly_quota(&state.db, tenant_id, hourly_quota).await?;
    }

    let raw_spec = parse_spec_body(&headers, &body)?;
    // Keep the original submission for audit — reserialise as JSON so the
    // DB column format stays stable even when clients sent YAML.
    let spec_json = serde_json::to_string(&raw_spec)?;
    let ir = compile_value(raw_spec)?;

    let lease_token = uuid::Uuid::new_v4().to_string();
    let (backend_id, backend) = state.registry.select(&ir).map_err(|e| {
        ApiError::new(
            crate::error::ApiErrorCode::BackendUnavailable,
            e.to_string(),
        )
    })?;
    if let (Some(extensions), Some(descriptor)) = (
        ir.extensions.as_ref(),
        state.registry.available_descriptor(&backend_id),
    ) {
        if let Some(schema) = descriptor.extensions_schema {
            validate_backend_extensions_schema(&backend_id, schema, extensions)?;
        }
    }
    backend.can_satisfy(&ir).await?;

    let row = store::insert_sandbox(
        &state.db,
        store::NewSandbox {
            id: &ir.id,
            tenant_id: auth.tenant_id(),
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
            if let Err(error) = persist_created_sandbox(
                &state,
                &backend,
                &backend_id,
                &ir.id,
                &handle,
                auth.tenant_id(),
                ir.ttl_seconds,
            )
            .await
            {
                return Err(error);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            store::set_status(&state.db, &ir.id, SandboxState::Failed(msg.clone())).await?;
            state.metrics.backend_error();
            audit::record(
                &state.db,
                AuditEvent::backend_error(&ir.id, auth.tenant_id(), &backend_id, msg),
            )
            .await;
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
    tenant_id: Option<&str>,
    ttl_seconds: u64,
) -> Result<(), ApiError> {
    if let Err(error) = store::set_backend_handle(&state.db, sandbox_id, handle).await {
        cleanup_after_persist_failure(
            state, backend, backend_id, sandbox_id, handle, tenant_id, &error,
        )
        .await;
        return Err(error);
    }

    if let Err(error) = store::set_status(&state.db, sandbox_id, SandboxState::Running).await {
        cleanup_after_persist_failure(
            state, backend, backend_id, sandbox_id, handle, tenant_id, &error,
        )
        .await;
        return Err(error);
    }

    state.metrics.sandbox_created();
    audit::record(
        &state.db,
        AuditEvent::sandbox_created(sandbox_id, tenant_id, backend_id, ttl_seconds),
    )
    .await;
    Ok(())
}

async fn cleanup_after_persist_failure(
    state: &SharedState,
    backend: &std::sync::Arc<dyn agentsandbox_sdk::backend::SandboxBackend>,
    backend_id: &str,
    sandbox_id: &str,
    handle: &str,
    tenant_id: Option<&str>,
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
        state.metrics.backend_error();
        tracing::error!(
            sandbox_id = %sandbox_id,
            backend_id = %backend_id,
            backend_handle = %handle,
            error = %destroy_error,
            "cleanup backend fallito dopo persist error"
        );
        audit::record(
            &state.db,
            AuditEvent::backend_error(sandbox_id, tenant_id, backend_id, destroy_error.to_string()),
        )
        .await;
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
    state.metrics.backend_error();
    audit::record(
        &state.db,
        AuditEvent::backend_error(sandbox_id, tenant_id, backend_id, message),
    )
    .await;
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

    let backend = state.registry.get(&row.backend).map_err(|e| {
        ApiError::new(
            crate::error::ApiErrorCode::BackendUnavailable,
            e.to_string(),
        )
    })?;

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
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<Json<InspectResponse>, ApiError> {
    let row = store::get_sandbox_scoped(&state.db, &id, access_scope(&auth))
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
    Extension(auth): Extension<AuthContext>,
    Query(params): Query<ListParams>,
) -> Result<Json<Value>, ApiError> {
    let limit = params.limit.clamp(1, 500);
    let offset = params.offset.max(0);
    let rows = store::list_active_scoped(&state.db, limit, offset, access_scope(&auth)).await?;
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

fn access_scope(auth: &AuthContext) -> AccessScope<'_> {
    match auth {
        AuthContext::SingleUser => AccessScope::All,
        AuthContext::Tenant(tenant) => AccessScope::Tenant(&tenant.id),
    }
}

async fn require_lease(
    state: &SharedState,
    auth: &AuthContext,
    id: &str,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let token = headers
        .get(LEASE_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::lease_invalid)?;
    if !store::verify_lease_scoped(&state.db, id, token, access_scope(auth)).await? {
        return Err(ApiError::lease_invalid());
    }
    Ok(())
}

pub async fn exec_sandbox(
    State(state): State<SharedState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecRequest>,
) -> Result<Json<ExecResponse>, ApiError> {
    require_lease(&state, &auth, &id, &headers).await?;

    let row = store::get_sandbox_scoped(&state.db, &id, access_scope(&auth))
        .await?
        .ok_or_else(|| ApiError::not_found(&id))?;
    if row.status != "running" {
        return Err(ApiError::new(
            crate::error::ApiErrorCode::SandboxExpired,
            format!("sandbox {id} non è in esecuzione (status={})", row.status),
        ));
    }

    let backend = state.registry.get(&row.backend).map_err(|e| {
        ApiError::new(
            crate::error::ApiErrorCode::BackendUnavailable,
            e.to_string(),
        )
    })?;
    audit::record(
        &state.db,
        AuditEvent::exec_started(&id, row.tenant_id.as_deref(), &row.backend, &req.command),
    )
    .await;
    let result = match backend.exec(row.runtime_handle(), &req.command, None).await {
        Ok(result) => result,
        Err(error) => {
            state.metrics.backend_error();
            audit::record(
                &state.db,
                AuditEvent::backend_error(
                    &id,
                    row.tenant_id.as_deref(),
                    &row.backend,
                    error.to_string(),
                ),
            )
            .await;
            return Err(error.into());
        }
    };
    state.metrics.exec_finished();
    audit::record(
        &state.db,
        AuditEvent::exec_finished(
            &id,
            row.tenant_id.as_deref(),
            &row.backend,
            result.exit_code,
            result.duration_ms,
        ),
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
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, ApiError> {
    // Destroy accepts the lease when present but also works without it when
    // the sandbox row doesn't exist (idempotent cleanup by id). When the row
    // DOES exist the lease is required — otherwise anyone could kill it.
    if store::get_sandbox_scoped(&state.db, &id, access_scope(&auth))
        .await?
        .is_some()
    {
        require_lease(&state, &auth, &id, &headers).await?;
    }

    if let Some(row) = store::get_sandbox_scoped(&state.db, &id, access_scope(&auth)).await? {
        let backend = state.registry.get(&row.backend).map_err(|e| {
            ApiError::new(
                crate::error::ApiErrorCode::BackendUnavailable,
                e.to_string(),
            )
        })?;
        if let Err(error) = backend.destroy(row.runtime_handle()).await {
            state.metrics.backend_error();
            audit::record(
                &state.db,
                AuditEvent::backend_error(
                    &id,
                    row.tenant_id.as_deref(),
                    &row.backend,
                    error.to_string(),
                ),
            )
            .await;
            return Err(error.into());
        }
        let was_active = matches!(row.status.as_str(), "creating" | "running");
        store::delete_sandbox(&state.db, &id).await?;
        state.metrics.sandbox_destroyed(was_active);
        audit::record(
            &state.db,
            AuditEvent::sandbox_destroyed(
                &id,
                row.tenant_id.as_deref(),
                &row.backend,
                DestroyReason::ClientRequest,
            ),
        )
        .await;
        return Ok(axum::http::StatusCode::NO_CONTENT);
    }
    store::delete_sandbox(&state.db, &id).await?;

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

    use agentsandbox_sdk::{
        backend::{
            BackendCapabilities, BackendDescriptor, BackendFactory, ExecResult, IsolationLevel,
            SandboxBackend, SandboxState, SandboxStatus,
        },
        error::BackendError,
        ir::SandboxIR,
    };
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use sqlx::{sqlite::SqliteConnectOptions, Row, SqlitePool};
    use std::str::FromStr;
    use tower::ServiceExt;

    use crate::{
        config::{
            AuthMode, AuthSection, BackendsSection, DaemonConfig, DaemonSection, DatabaseSection,
        },
        metrics::Metrics,
        registry::BackendRegistry,
        router,
        state::AppState,
        store,
    };

    enum InspectBehavior {
        Status(SandboxState),
        NotFound,
    }

    struct MockBackend {
        backend_id: &'static str,
        inspect_behavior: InspectBehavior,
        destroyed: Arc<Mutex<Vec<String>>>,
        last_extensions: Arc<Mutex<Option<Value>>>,
        allow_extensions: bool,
    }

    #[async_trait]
    impl SandboxBackend for MockBackend {
        async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
            *self.last_extensions.lock().unwrap() = ir.extensions.clone();
            Ok(format!("handle-{}", ir.id))
        }

        async fn exec(
            &self,
            _handle: &str,
            command: &str,
            _timeout_ms: Option<u64>,
        ) -> Result<ExecResult, BackendError> {
            Ok(ExecResult {
                stdout: command.to_string(),
                stderr: String::new(),
                exit_code: 0,
                duration_ms: 7,
                resource_usage: None,
            })
        }

        async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
            match &self.inspect_behavior {
                InspectBehavior::Status(status) => Ok(SandboxStatus {
                    sandbox_id: handle.into(),
                    state: status.clone(),
                    created_at: Utc::now(),
                    expires_at: Utc::now(),
                    backend_id: self.backend_id.into(),
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

        async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
            *self.last_extensions.lock().unwrap() = ir.extensions.clone();
            if ir.extensions.is_some() && !self.allow_extensions {
                return Err(BackendError::NotSupported(
                    "questo backend non supporta extensions".into(),
                ));
            }
            Ok(())
        }
    }

    struct MockFactory {
        backend_id: &'static str,
        inspect_behavior: InspectBehavior,
        destroyed: Arc<Mutex<Vec<String>>>,
        last_extensions: Arc<Mutex<Option<Value>>>,
        allow_extensions: bool,
        extensions_schema: Option<&'static str>,
    }

    impl BackendFactory for MockFactory {
        fn describe(&self) -> BackendDescriptor {
            BackendDescriptor {
                id: self.backend_id,
                display_name: "Mock",
                version: "test",
                trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
                capabilities: BackendCapabilities {
                    isolation_level: IsolationLevel::Process,
                    ..BackendCapabilities::default()
                },
                extensions_schema: self.extensions_schema,
            }
        }

        fn create(
            &self,
            _config: &HashMap<String, String>,
        ) -> Result<Box<dyn SandboxBackend>, BackendError> {
            Ok(Box::new(MockBackend {
                backend_id: self.backend_id,
                inspect_behavior: match &self.inspect_behavior {
                    InspectBehavior::Status(state) => InspectBehavior::Status(state.clone()),
                    InspectBehavior::NotFound => InspectBehavior::NotFound,
                },
                destroyed: self.destroyed.clone(),
                last_extensions: self.last_extensions.clone(),
                allow_extensions: self.allow_extensions,
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

    async fn test_db_pre_multitenancy() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query(include_str!("../migrations/001_initial.sql"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(include_str!("../migrations/002_backend_handle.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn test_state_with_factory(
        db: SqlitePool,
        factory: MockFactory,
    ) -> (
        Arc<AppState>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Option<Value>>>,
    ) {
        let mut registry = BackendRegistry::new();
        let destroyed = factory.destroyed.clone();
        let last_extensions = factory.last_extensions.clone();
        registry.register(&factory);
        registry.initialize(&factory, &HashMap::new()).await;
        (
            Arc::new(AppState {
                db,
                config: test_config(AuthMode::SingleUser, factory.backend_id),
                registry: Arc::new(registry),
                metrics: Metrics::new(),
            }),
            destroyed,
            last_extensions,
        )
    }

    async fn test_state(
        db: SqlitePool,
        inspect_behavior: InspectBehavior,
    ) -> (
        Arc<AppState>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Option<Value>>>,
    ) {
        let mut registry = BackendRegistry::new();
        let destroyed = Arc::new(Mutex::new(Vec::new()));
        let last_extensions = Arc::new(Mutex::new(None));
        let factory = MockFactory {
            backend_id: "mock",
            inspect_behavior,
            destroyed: destroyed.clone(),
            last_extensions: last_extensions.clone(),
            allow_extensions: false,
            extensions_schema: None,
        };
        let _ = registry;
        test_state_with_factory(db, factory).await
    }

    async fn test_state_with_extensions(
        db: SqlitePool,
        inspect_behavior: InspectBehavior,
    ) -> (
        Arc<AppState>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Option<Value>>>,
    ) {
        let mut registry = BackendRegistry::new();
        let destroyed = Arc::new(Mutex::new(Vec::new()));
        let last_extensions = Arc::new(Mutex::new(None));
        let factory = MockFactory {
            backend_id: "mock",
            inspect_behavior,
            destroyed: destroyed.clone(),
            last_extensions: last_extensions.clone(),
            allow_extensions: true,
            extensions_schema: Some("{}"),
        };
        let _ = registry;
        test_state_with_factory(db, factory).await
    }

    async fn test_state_for_backend_with_schema(
        db: SqlitePool,
        inspect_behavior: InspectBehavior,
        backend_id: &'static str,
        extensions_schema: Option<&'static str>,
    ) -> (
        Arc<AppState>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Option<Value>>>,
    ) {
        let destroyed = Arc::new(Mutex::new(Vec::new()));
        let last_extensions = Arc::new(Mutex::new(None));
        let factory = MockFactory {
            backend_id,
            inspect_behavior,
            destroyed: destroyed.clone(),
            last_extensions: last_extensions.clone(),
            allow_extensions: extensions_schema.is_some(),
            extensions_schema,
        };
        test_state_with_factory(db, factory).await
    }

    fn test_config(mode: AuthMode, backend_id: &str) -> DaemonConfig {
        DaemonConfig {
            daemon: DaemonSection {
                host: "127.0.0.1".into(),
                port: 7847,
                log_level: "info".into(),
                log_format: "text".into(),
            },
            database: DatabaseSection {
                url: "sqlite::memory:".into(),
            },
            auth: AuthSection { mode },
            backends: BackendsSection {
                enabled: vec![backend_id.into()],
                bubblewrap: Default::default(),
                docker: Default::default(),
                gvisor: Default::default(),
                libkrun: Default::default(),
                nsjail: Default::default(),
                podman: Default::default(),
                wasmtime: Default::default(),
            },
        }
    }

    async fn insert_running_row(pool: &SqlitePool, id: &str) {
        let created_at = Utc::now();
        let expires_at = created_at + Duration::seconds(60);
        sqlx::query(
            "INSERT INTO sandboxes \
             (id, tenant_id, lease_token, status, backend, backend_handle, spec_json, ir_json, created_at, expires_at) \
             VALUES (?1, NULL, ?2, 'running', 'mock', ?3, '{}', '{}', ?4, ?5)",
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
        let (state, _, _) =
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
        let (state, _, _) = test_state(db.clone(), InspectBehavior::NotFound).await;

        let fresh = refresh_runtime_status(&state, row).await.unwrap();
        assert_eq!(fresh.status, "stopped");

        let persisted = store::get_sandbox(&db, "sb-2").await.unwrap().unwrap();
        assert_eq!(persisted.status, "stopped");
    }

    #[tokio::test]
    async fn create_sandbox_accepts_v1_json() {
        let db = test_db().await;
        let (state, _, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
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
    async fn metrics_endpoint_returns_prometheus_payload() {
        let db = test_db().await;
        let (state, _, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
        state.metrics.sandbox_created();
        state.metrics.exec_finished();
        state.metrics.backend_error();
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/plain; version=0.0.4; charset=utf-8")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload = String::from_utf8(body.to_vec()).unwrap();
        assert!(payload.contains("agentsandbox_sandboxes_created_total 1"));
        assert!(payload.contains("agentsandbox_exec_total 1"));
        assert!(payload.contains("agentsandbox_backend_errors_total 1"));
    }

    #[tokio::test]
    async fn create_sandbox_accepts_v1_yaml() {
        let db = test_db().await;
        let (state, _, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
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
    async fn sandbox_lifecycle_writes_structured_audit_log() {
        let db = test_db().await;
        let (state, _, _) =
            test_state(db.clone(), InspectBehavior::Status(SandboxState::Running)).await;
        let app = router::build(state);

        let create = app
            .clone()
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
        assert_eq!(create.status(), axum::http::StatusCode::CREATED);

        let create_body = axum::body::to_bytes(create.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: CreateResponse = serde_json::from_slice(&create_body).unwrap();

        let exec = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/sandboxes/{}/exec", created.sandbox_id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(LEASE_HEADER, &created.lease_token)
                    .body(Body::from(r#"{"command":"echo hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exec.status(), axum::http::StatusCode::OK);

        let destroy = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/sandboxes/{}", created.sandbox_id))
                    .header(LEASE_HEADER, &created.lease_token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(destroy.status(), axum::http::StatusCode::NO_CONTENT);

        let rows = sqlx::query(
            "SELECT event, detail FROM audit_log WHERE sandbox_id = ?1 ORDER BY id ASC",
        )
        .bind(&created.sandbox_id)
        .fetch_all(&db)
        .await
        .unwrap();

        let events: Vec<String> = rows.iter().map(|row| row.get("event")).collect();
        assert_eq!(
            events,
            vec![
                "sandbox_created",
                "exec_started",
                "exec_finished",
                "sandbox_destroyed"
            ]
        );

        let exec_started_detail: Value = serde_json::from_str(rows[1].get("detail")).unwrap();
        assert_eq!(
            exec_started_detail["event"]["command_hash"],
            crate::audit::command_hash("echo hello")
        );
        assert_eq!(exec_started_detail["backend_id"], "mock");
        assert_eq!(exec_started_detail["event"]["type"], "exec_started");
        assert!(!rows[1].get::<String, _>("detail").contains("echo hello"));

        let destroyed_detail: Value = serde_json::from_str(rows[3].get("detail")).unwrap();
        assert_eq!(destroyed_detail["event"]["reason"], "client_request");
    }

    #[tokio::test]
    async fn create_sandbox_returns_structured_schema_errors() {
        let db = test_db().await;
        let (state, _, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
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

        let (state, destroyed, _) =
            test_state(db.clone(), InspectBehavior::Status(SandboxState::Running)).await;
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

        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );

        let destroyed = destroyed.lock().unwrap().clone();
        assert_eq!(destroyed.len(), 1);
        assert!(destroyed[0].starts_with("handle-"));
    }

    #[tokio::test]
    async fn create_sandbox_accepts_extensions_in_spec() {
        let db = test_db().await;
        let (state, _, last_extensions) =
            test_state_with_extensions(db, InspectBehavior::Status(SandboxState::Running)).await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sandboxes")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"apiVersion":"sandbox.ai/v1","kind":"Sandbox","metadata":{},"spec":{"runtime":{"preset":"python"},"scheduling":{"backend":"mock"},"extensions":{"mock":{"debug":true}}}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let captured = last_extensions.lock().unwrap().clone();
        assert_eq!(captured, Some(serde_json::json!({"mock":{"debug":true}})));
    }

    #[tokio::test]
    async fn create_sandbox_rejects_extensions_without_backend() {
        let db = test_db().await;
        let (state, _, _) = test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sandboxes")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"apiVersion":"sandbox.ai/v1","kind":"Sandbox","metadata":{},"spec":{"runtime":{"preset":"python"},"extensions":{"docker":{"hostConfig":{"capAdd":["NET_ADMIN"]}}}}}"#,
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
        assert!(payload["error"]["message"]
            .as_str()
            .unwrap()
            .contains("scheduling.backend"));
    }

    #[tokio::test]
    async fn create_sandbox_rejects_invalid_docker_extensions_field() {
        let db = test_db().await;
        let (state, _, _) = test_state_for_backend_with_schema(
            db,
            InspectBehavior::Status(SandboxState::Running),
            "docker",
            Some(include_str!(
                "../../agentsandbox-backend-docker/schema/extensions.schema.json"
            )),
        )
        .await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sandboxes")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"apiVersion":"sandbox.ai/v1","kind":"Sandbox","metadata":{},"spec":{"runtime":{"preset":"python"},"scheduling":{"backend":"docker"},"extensions":{"docker":{"hostConfig":{"unknownField":true}}}}}"#,
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
        let errors = payload["error"]["details"]["validationErrors"]
            .as_array()
            .unwrap();
        assert!(errors
            .iter()
            .any(|issue| issue["message"].as_str().unwrap().contains("unknownField")));
    }

    #[tokio::test]
    async fn get_backend_extensions_schema_returns_json_schema() {
        let db = test_db().await;
        let (state, _, _) = test_state_for_backend_with_schema(
            db,
            InspectBehavior::Status(SandboxState::Running),
            "docker",
            Some(include_str!(
                "../../agentsandbox-backend-docker/schema/extensions.schema.json"
            )),
        )
        .await;
        let app = router::build(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/backends/docker/extensions-schema")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["title"], "Docker Backend Extensions");
    }

    #[tokio::test]
    async fn api_key_mode_rejects_requests_without_header() {
        let db = test_db().await;
        let (mut state, _, _) =
            test_state(db, InspectBehavior::Status(SandboxState::Running)).await;
        Arc::get_mut(&mut state).unwrap().config.auth.mode = AuthMode::ApiKey;
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

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn api_key_mode_persists_tenant_and_enforces_hourly_quota() {
        let db = test_db().await;
        sqlx::query(
            "INSERT INTO tenants (id, api_key_hash, quota_hourly, quota_concurrent, enabled, created_at) \
             VALUES (?1, ?2, 1, 10, 1, ?3)",
        )
        .bind("tenant-a")
        .bind(store::hash_api_key_for_tests("secret-key"))
        .bind(Utc::now().to_rfc3339())
        .execute(&db)
        .await
        .unwrap();

        let (mut state, _, _) =
            test_state(db.clone(), InspectBehavior::Status(SandboxState::Running)).await;
        Arc::get_mut(&mut state).unwrap().config.auth.mode = AuthMode::ApiKey;
        let app = router::build(state);

        let request = || {
            Request::builder()
                .method("POST")
                .uri("/v1/sandboxes")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "secret-key")
                .body(Body::from(
                    r#"{"apiVersion":"sandbox.ai/v1","kind":"Sandbox","metadata":{},"spec":{"runtime":{"preset":"python"}}}"#,
                ))
                .unwrap()
        };

        let first = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(first.status(), axum::http::StatusCode::CREATED);

        let row = sqlx::query("SELECT tenant_id FROM sandboxes LIMIT 1")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(
            row.get::<Option<String>, _>("tenant_id"),
            Some("tenant-a".into())
        );

        let second = app.oneshot(request()).await.unwrap();
        assert_eq!(second.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn multitenancy_migration_applies_to_existing_schema() {
        let db = test_db_pre_multitenancy().await;

        sqlx::query(include_str!("../migrations/003_multitenancy.sql"))
            .execute(&db)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO tenants (id, api_key_hash, quota_hourly, quota_concurrent, enabled, created_at) \
             VALUES ('tenant-a', 'hash', 10, 5, 1, '2026-01-01T00:00:00+00:00')",
        )
        .execute(&db)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO rate_limit_windows (tenant_id, window_start, count) \
             VALUES ('tenant-a', '2026-01-01T01:00:00+00:00', 1)",
        )
        .execute(&db)
        .await
        .unwrap();
    }
}
