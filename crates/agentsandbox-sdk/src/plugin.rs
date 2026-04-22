use crate::{
    backend::{
        BackendCapabilities, BackendDescriptor, ExecResult, ResourceUsage, SandboxBackend,
        SandboxState, SandboxStatus,
    },
    error::BackendError,
    ir::SandboxIR,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginDescriptor {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub trait_version: String,
    pub capabilities: PluginCapabilities,
    pub extensions_schema: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginCapabilities {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
    pub cpu_user_ms: Option<u64>,
    pub memory_peak_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSandboxStatus {
    pub sandbox_id: String,
    pub state: String,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub backend_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginRequest {
    Metadata,
    HealthCheck,
    CanSatisfy {
        ir: SandboxIR,
    },
    Create {
        ir: SandboxIR,
    },
    Exec {
        handle: String,
        command: String,
        timeout_ms: Option<u64>,
    },
    Status {
        handle: String,
    },
    Destroy {
        handle: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginResponse {
    Metadata { metadata: PluginDescriptor },
    Ok,
    Created { handle: String },
    ExecResult { result: PluginExecResult },
    Status { status: PluginSandboxStatus },
    Error { error: BackendError },
}

impl From<BackendDescriptor> for PluginDescriptor {
    fn from(value: BackendDescriptor) -> Self {
        Self {
            id: value.id.to_string(),
            display_name: value.display_name.to_string(),
            version: value.version.to_string(),
            trait_version: value.trait_version().to_string(),
            capabilities: value.capabilities.into(),
            extensions_schema: value.extensions_schema.map(str::to_string),
        }
    }
}

impl From<BackendCapabilities> for PluginCapabilities {
    fn from(value: BackendCapabilities) -> Self {
        Self {
            network_isolation: value.network_isolation,
            memory_hard_limit: value.memory_hard_limit,
            cpu_hard_limit: value.cpu_hard_limit,
            persistent_storage: value.persistent_storage,
            self_contained: value.self_contained,
            isolation_level: format!("{:?}", value.isolation_level),
            supported_presets: value
                .supported_presets
                .into_iter()
                .map(str::to_string)
                .collect(),
            rootless: value.rootless,
            snapshot_restore: value.snapshot_restore,
        }
    }
}

impl From<ExecResult> for PluginExecResult {
    fn from(value: ExecResult) -> Self {
        Self {
            stdout: value.stdout,
            stderr: value.stderr,
            exit_code: value.exit_code,
            duration_ms: value.duration_ms,
            cpu_user_ms: value
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.cpu_user_ms),
            memory_peak_mb: value
                .resource_usage
                .as_ref()
                .and_then(|usage| usage.memory_peak_mb),
        }
    }
}

impl From<PluginExecResult> for ExecResult {
    fn from(value: PluginExecResult) -> Self {
        Self {
            stdout: value.stdout,
            stderr: value.stderr,
            exit_code: value.exit_code,
            duration_ms: value.duration_ms,
            resource_usage: if value.cpu_user_ms.is_some() || value.memory_peak_mb.is_some() {
                Some(ResourceUsage {
                    cpu_user_ms: value.cpu_user_ms,
                    memory_peak_mb: value.memory_peak_mb,
                })
            } else {
                None
            },
        }
    }
}

impl From<SandboxStatus> for PluginSandboxStatus {
    fn from(value: SandboxStatus) -> Self {
        let (state, error) = match value.state {
            SandboxState::Creating => ("creating".to_string(), None),
            SandboxState::Running => ("running".to_string(), None),
            SandboxState::Stopped => ("stopped".to_string(), None),
            SandboxState::Expired => ("expired".to_string(), None),
            SandboxState::Failed(message) => ("error".to_string(), Some(message)),
        };
        Self {
            sandbox_id: value.sandbox_id,
            state,
            error,
            created_at: value.created_at,
            expires_at: value.expires_at,
            backend_id: value.backend_id,
        }
    }
}

impl TryFrom<PluginSandboxStatus> for SandboxStatus {
    type Error = BackendError;

    fn try_from(value: PluginSandboxStatus) -> Result<Self, Self::Error> {
        let state = match value.state.as_str() {
            "creating" => SandboxState::Creating,
            "running" => SandboxState::Running,
            "stopped" => SandboxState::Stopped,
            "expired" => SandboxState::Expired,
            "error" => SandboxState::Failed(
                value
                    .error
                    .unwrap_or_else(|| "plugin returned error state without message".into()),
            ),
            other => {
                return Err(BackendError::Internal(format!(
                    "plugin returned unsupported sandbox state '{other}'"
                )))
            }
        };
        Ok(Self {
            sandbox_id: value.sandbox_id,
            state,
            created_at: value.created_at,
            expires_at: value.expires_at,
            backend_id: value.backend_id,
        })
    }
}

pub async fn serve_plugin(
    factory: &dyn crate::backend::BackendFactory,
) -> Result<(), BackendError> {
    let config_json =
        std::env::var("AGENTSANDBOX_PLUGIN_CONFIG_JSON").unwrap_or_else(|_| "{}".into());
    let config: HashMap<String, String> = serde_json::from_str(&config_json).map_err(|error| {
        BackendError::Configuration(format!("plugin config json non valido: {error}"))
    })?;
    let backend = factory.create(&config)?;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = stdout;

    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|error| BackendError::Internal(error.to_string()))?
    {
        if line.trim().is_empty() {
            continue;
        }

        let request: PluginRequest = serde_json::from_str(&line)
            .map_err(|error| BackendError::Internal(error.to_string()))?;
        let response = handle_request(factory.describe(), backend.as_ref(), request).await;
        let encoded = serde_json::to_string(&response)
            .map_err(|error| BackendError::Internal(error.to_string()))?;
        writer
            .write_all(encoded.as_bytes())
            .await
            .map_err(|error| BackendError::Internal(error.to_string()))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|error| BackendError::Internal(error.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|error| BackendError::Internal(error.to_string()))?;
    }

    Ok(())
}

async fn handle_request(
    descriptor: BackendDescriptor,
    backend: &dyn SandboxBackend,
    request: PluginRequest,
) -> PluginResponse {
    let result = match request {
        PluginRequest::Metadata => Ok(PluginResponse::Metadata {
            metadata: descriptor.into(),
        }),
        PluginRequest::HealthCheck => backend.health_check().await.map(|_| PluginResponse::Ok),
        PluginRequest::CanSatisfy { ir } => {
            backend.can_satisfy(&ir).await.map(|_| PluginResponse::Ok)
        }
        PluginRequest::Create { ir } => backend
            .create(&ir)
            .await
            .map(|handle| PluginResponse::Created { handle }),
        PluginRequest::Exec {
            handle,
            command,
            timeout_ms,
        } => backend
            .exec(&handle, &command, timeout_ms)
            .await
            .map(|result| PluginResponse::ExecResult {
                result: result.into(),
            }),
        PluginRequest::Status { handle } => {
            backend
                .status(&handle)
                .await
                .map(|status| PluginResponse::Status {
                    status: status.into(),
                })
        }
        PluginRequest::Destroy { handle } => {
            backend.destroy(&handle).await.map(|_| PluginResponse::Ok)
        }
    };

    match result {
        Ok(response) => response,
        Err(error) => PluginResponse::Error { error },
    }
}
