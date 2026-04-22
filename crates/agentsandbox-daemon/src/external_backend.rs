use agentsandbox_sdk::{
    backend::{ExecResult, SandboxBackend, SandboxStatus},
    error::BackendError,
    ir::SandboxIR,
    plugin::{PluginDescriptor, PluginRequest, PluginResponse},
};
use async_trait::async_trait;
use std::{path::Path, path::PathBuf, process::Stdio, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

struct PluginSession {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

pub struct ExternalBackend {
    path: PathBuf,
    descriptor: PluginDescriptor,
    session: Mutex<PluginSession>,
}

impl ExternalBackend {
    pub async fn spawn(
        path: PathBuf,
        config: std::collections::HashMap<String, String>,
    ) -> Result<(PluginDescriptor, Arc<Self>), BackendError> {
        let config_json = serde_json::to_string(&config)
            .map_err(|error| BackendError::Configuration(error.to_string()))?;
        let mut child = Command::new(&path)
            .arg("serve")
            .env("AGENTSANDBOX_PLUGIN_CONFIG_JSON", config_json)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                BackendError::Unavailable(format!(
                    "impossibile avviare plugin {}: {error}",
                    path.display()
                ))
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            BackendError::Unavailable(format!(
                "plugin {} avviato senza stdin piped",
                path.display()
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BackendError::Unavailable(format!(
                "plugin {} avviato senza stdout piped",
                path.display()
            ))
        })?;

        let mut backend = Self {
            path,
            descriptor: PluginDescriptor {
                id: String::new(),
                display_name: String::new(),
                version: String::new(),
                trait_version: String::new(),
                capabilities: agentsandbox_sdk::plugin::PluginCapabilities {
                    network_isolation: false,
                    memory_hard_limit: false,
                    cpu_hard_limit: false,
                    persistent_storage: false,
                    self_contained: false,
                    isolation_level: "Process".into(),
                    supported_presets: Vec::new(),
                    rootless: false,
                    snapshot_restore: false,
                },
                extensions_schema: None,
            },
            session: Mutex::new(PluginSession {
                child,
                stdin,
                stdout: BufReader::new(stdout).lines(),
            }),
        };

        let descriptor = match backend.request(PluginRequest::Metadata).await? {
            PluginResponse::Metadata { metadata } => metadata,
            PluginResponse::Error { error } => return Err(error),
            other => {
                return Err(BackendError::Internal(format!(
                    "plugin {} ha restituito una risposta inattesa a metadata: {other:?}",
                    backend.path.display()
                )))
            }
        };

        backend.descriptor = descriptor.clone();
        let backend = Arc::new(backend);

        match backend.request(PluginRequest::HealthCheck).await? {
            PluginResponse::Ok => Ok((descriptor, backend)),
            PluginResponse::Error { error } => Err(error),
            other => Err(BackendError::Internal(format!(
                "plugin {} ha restituito una risposta inattesa a health_check: {other:?}",
                backend.path.display()
            ))),
        }
    }

    pub fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    pub fn id_from_path(path: &Path) -> Option<String> {
        let file_name = path.file_name()?.to_str()?;
        let stem = file_name.strip_suffix(".exe").unwrap_or(file_name);
        stem.strip_prefix("agentsandbox-backend-")
            .map(str::to_string)
    }

    async fn request(&self, request: PluginRequest) -> Result<PluginResponse, BackendError> {
        let mut session = self.session.lock().await;
        let encoded = serde_json::to_string(&request)
            .map_err(|error| BackendError::Internal(error.to_string()))?;
        session
            .stdin
            .write_all(encoded.as_bytes())
            .await
            .map_err(|error| self.io_error("write request", error))?;
        session
            .stdin
            .write_all(b"\n")
            .await
            .map_err(|error| self.io_error("write newline", error))?;
        session
            .stdin
            .flush()
            .await
            .map_err(|error| self.io_error("flush request", error))?;

        let Some(line) = session
            .stdout
            .next_line()
            .await
            .map_err(|error| self.io_error("read response", error))?
        else {
            let status = session.child.wait().await.ok();
            return Err(BackendError::Unavailable(format!(
                "plugin {} ha chiuso lo stream stdout{}",
                self.path.display(),
                status
                    .map(|value| format!(" (status: {value})"))
                    .unwrap_or_default()
            )));
        };

        serde_json::from_str(&line).map_err(|error| {
            BackendError::Internal(format!(
                "plugin {} ha restituito JSON non valido: {error}",
                self.path.display()
            ))
        })
    }

    fn io_error(&self, action: &str, error: std::io::Error) -> BackendError {
        BackendError::Unavailable(format!(
            "plugin {} non disponibile durante {action}: {error}",
            self.path.display()
        ))
    }
}

#[async_trait]
impl SandboxBackend for ExternalBackend {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        match self
            .request(PluginRequest::Create { ir: ir.clone() })
            .await?
        {
            PluginResponse::Created { handle } => Ok(handle),
            PluginResponse::Error { error } => Err(error),
            other => Err(BackendError::Internal(format!(
                "plugin {} ha restituito una risposta inattesa a create: {other:?}",
                self.path.display()
            ))),
        }
    }

    async fn exec(
        &self,
        handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError> {
        match self
            .request(PluginRequest::Exec {
                handle: handle.into(),
                command: command.into(),
                timeout_ms,
            })
            .await?
        {
            PluginResponse::ExecResult { result } => Ok(result.into()),
            PluginResponse::Error { error } => Err(error),
            other => Err(BackendError::Internal(format!(
                "plugin {} ha restituito una risposta inattesa a exec: {other:?}",
                self.path.display()
            ))),
        }
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        match self
            .request(PluginRequest::Status {
                handle: handle.into(),
            })
            .await?
        {
            PluginResponse::Status { status } => status.try_into(),
            PluginResponse::Error { error } => Err(error),
            other => Err(BackendError::Internal(format!(
                "plugin {} ha restituito una risposta inattesa a status: {other:?}",
                self.path.display()
            ))),
        }
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        match self
            .request(PluginRequest::Destroy {
                handle: handle.into(),
            })
            .await?
        {
            PluginResponse::Ok => Ok(()),
            PluginResponse::Error { error } => Err(error),
            other => Err(BackendError::Internal(format!(
                "plugin {} ha restituito una risposta inattesa a destroy: {other:?}",
                self.path.display()
            ))),
        }
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        match self.request(PluginRequest::HealthCheck).await? {
            PluginResponse::Ok => Ok(()),
            PluginResponse::Error { error } => Err(error),
            other => Err(BackendError::Internal(format!(
                "plugin {} ha restituito una risposta inattesa a health_check: {other:?}",
                self.path.display()
            ))),
        }
    }

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        match self
            .request(PluginRequest::CanSatisfy { ir: ir.clone() })
            .await?
        {
            PluginResponse::Ok => Ok(()),
            PluginResponse::Error { error } => Err(error),
            other => Err(BackendError::Internal(format!(
                "plugin {} ha restituito una risposta inattesa a can_satisfy: {other:?}",
                self.path.display()
            ))),
        }
    }
}
