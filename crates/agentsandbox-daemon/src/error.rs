//! HTTP error envelope.
//!
//! Every error that leaves the daemon goes through [`ApiError`]. The JSON
//! shape is part of the public contract (`docs/api-http-v1.md`):
//!
//! ```json
//! { "error": { "code": "SANDBOX_NOT_FOUND", "message": "...", "details": {} } }
//! ```
//!
//! Internal details (sqlx errors, raw bollard strings) are logged but never
//! surfaced verbatim — we map them to stable codes.

use agentsandbox_core::compile::CompileError;
use agentsandbox_sdk::error::BackendError;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub enum ApiErrorCode {
    BackendNotFound,
    SandboxNotFound,
    SandboxExpired,
    SpecInvalid,
    Unauthorized,
    RateLimitExceeded,
    BackendUnavailable,
    ExecTimeout,
    LeaseInvalid,
    InternalError,
}

impl ApiErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiErrorCode::BackendNotFound => "BACKEND_NOT_FOUND",
            ApiErrorCode::SandboxNotFound => "SANDBOX_NOT_FOUND",
            ApiErrorCode::SandboxExpired => "SANDBOX_EXPIRED",
            ApiErrorCode::SpecInvalid => "SPEC_INVALID",
            ApiErrorCode::Unauthorized => "UNAUTHORIZED",
            ApiErrorCode::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
            ApiErrorCode::BackendUnavailable => "BACKEND_UNAVAILABLE",
            ApiErrorCode::ExecTimeout => "EXEC_TIMEOUT",
            ApiErrorCode::LeaseInvalid => "LEASE_INVALID",
            ApiErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            ApiErrorCode::BackendNotFound => StatusCode::NOT_FOUND,
            ApiErrorCode::SandboxNotFound => StatusCode::NOT_FOUND,
            ApiErrorCode::SandboxExpired => StatusCode::GONE,
            ApiErrorCode::SpecInvalid => StatusCode::UNPROCESSABLE_ENTITY,
            ApiErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiErrorCode::RateLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            ApiErrorCode::BackendUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ApiErrorCode::ExecTimeout => StatusCode::GATEWAY_TIMEOUT,
            ApiErrorCode::LeaseInvalid => StatusCode::FORBIDDEN,
            ApiErrorCode::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn not_found(id: &str) -> Self {
        Self::new(
            ApiErrorCode::SandboxNotFound,
            format!("sandbox {id} non trovata"),
        )
    }

    pub fn backend_not_found(id: &str) -> Self {
        Self::new(
            ApiErrorCode::BackendNotFound,
            format!("backend {id} non trovato"),
        )
    }

    pub fn spec_invalid(msg: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::SpecInvalid, msg)
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Unauthorized, msg)
    }

    pub fn rate_limited(msg: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::RateLimitExceeded, msg)
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn lease_invalid() -> Self {
        Self::new(
            ApiErrorCode::LeaseInvalid,
            "lease token mancante o non valido",
        )
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::InternalError, msg)
    }
}

impl From<BackendError> for ApiError {
    fn from(e: BackendError) -> Self {
        let code = match e {
            BackendError::NotFound(_) => ApiErrorCode::SandboxNotFound,
            BackendError::Unavailable(_) => ApiErrorCode::BackendUnavailable,
            BackendError::Timeout(_) => ApiErrorCode::ExecTimeout,
            BackendError::NotSupported(_) | BackendError::Configuration(_) => {
                ApiErrorCode::SpecInvalid
            }
            BackendError::ResourceExhausted(_) => ApiErrorCode::BackendUnavailable,
            BackendError::Internal(_) => ApiErrorCode::InternalError,
        };
        ApiError::new(code, e.to_string())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = %e, "sqlx error");
        ApiError::internal("errore persistenza")
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::spec_invalid(format!("JSON non valido: {e}"))
    }
}

impl From<serde_yaml::Error> for ApiError {
    fn from(e: serde_yaml::Error) -> Self {
        ApiError::spec_invalid(format!("YAML non valido: {e}"))
    }
}

impl From<CompileError> for ApiError {
    fn from(e: CompileError) -> Self {
        match e {
            CompileError::SchemaValidation { version, issues } => {
                ApiError::spec_invalid(format!("spec {} non valida", version.as_str()))
                    .with_details(json!({
                        "apiVersion": version.as_str(),
                        "validationErrors": issues
                            .into_iter()
                            .map(|issue| json!({
                                "path": issue.path,
                                "message": issue.message,
                            }))
                            .collect::<Vec<_>>()
                    }))
            }
            other => ApiError::spec_invalid(other.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code.as_str(),
                "message": self.message,
                "details": self.details.unwrap_or_else(|| json!({})),
            }
        });
        (self.code.status(), Json(body)).into_response()
    }
}
