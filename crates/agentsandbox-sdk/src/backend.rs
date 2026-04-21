use crate::{error::BackendError, ir::SandboxIR, BACKEND_TRAIT_VERSION};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct BackendDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub version: &'static str,
    pub trait_version: &'static str,
    pub capabilities: BackendCapabilities,
    pub extensions_schema: Option<&'static str>,
}

impl BackendDescriptor {
    pub fn trait_version(&self) -> &'static str {
        if self.trait_version.is_empty() {
            BACKEND_TRAIT_VERSION
        } else {
            self.trait_version
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BackendCapabilities {
    pub network_isolation: bool,
    pub memory_hard_limit: bool,
    pub cpu_hard_limit: bool,
    pub persistent_storage: bool,
    pub self_contained: bool,
    pub isolation_level: IsolationLevel,
    pub supported_presets: Vec<&'static str>,
    pub rootless: bool,
    pub snapshot_restore: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    #[default]
    Process,
    Container,
    KernelSandbox,
    MicroVM,
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
    pub resource_usage: Option<ResourceUsage>,
}

#[derive(Debug, Clone)]
pub struct ResourceUsage {
    pub cpu_user_ms: Option<u64>,
    pub memory_peak_mb: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SandboxStatus {
    pub sandbox_id: String,
    pub state: SandboxState,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub backend_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxState {
    Creating,
    Running,
    Stopped,
    Failed(String),
    Expired,
}

impl SandboxState {
    pub fn as_str(&self) -> &str {
        match self {
            SandboxState::Creating => "creating",
            SandboxState::Running => "running",
            SandboxState::Stopped => "stopped",
            SandboxState::Failed(_) => "error",
            SandboxState::Expired => "expired",
        }
    }
}

pub trait BackendFactory: Send + Sync {
    fn describe(&self) -> BackendDescriptor;
    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError>;
}

#[async_trait]
pub trait SandboxBackend: Send + Sync {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError>;
    async fn exec(
        &self,
        handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError>;
    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError>;
    async fn destroy(&self, handle: &str) -> Result<(), BackendError>;
    async fn health_check(&self) -> Result<(), BackendError>;

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        if ir.extensions.is_some() {
            return Err(BackendError::NotSupported(
                "questo backend non supporta extensions".into(),
            ));
        }
        Ok(())
    }

    async fn upload_file(
        &self,
        handle: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), BackendError> {
        let _ = (handle, path, content);
        Err(BackendError::NotSupported("upload_file".into()))
    }

    async fn download_file(&self, handle: &str, path: &str) -> Result<Vec<u8>, BackendError> {
        let _ = (handle, path);
        Err(BackendError::NotSupported("download_file".into()))
    }

    async fn snapshot(&self, handle: &str) -> Result<String, BackendError> {
        let _ = handle;
        Err(BackendError::NotSupported("snapshot".into()))
    }

    async fn restore(&self, snapshot_id: &str, ir: &SandboxIR) -> Result<String, BackendError> {
        let _ = (snapshot_id, ir);
        Err(BackendError::NotSupported("restore".into()))
    }
}
