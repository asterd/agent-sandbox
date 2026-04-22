use crate::{config::BackendsSection, external_backend::ExternalBackend};
use agentsandbox_sdk::{
    backend::SandboxBackend, error::BackendError, ir::SandboxIR, plugin::PluginDescriptor,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

pub struct BackendRegistry {
    descriptors: HashMap<String, PluginDescriptor>,
    instances: HashMap<String, Arc<dyn SandboxBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
            instances: HashMap::new(),
        }
    }

    pub fn register_instance(
        &mut self,
        descriptor: PluginDescriptor,
        backend: Arc<dyn SandboxBackend>,
    ) {
        tracing::info!(
            backend_id = %descriptor.id,
            version = %descriptor.version,
            trait_version = %descriptor.trait_version,
            "backend registrato"
        );
        self.instances.insert(descriptor.id.clone(), backend);
        self.descriptors.insert(descriptor.id.clone(), descriptor);
    }

    pub async fn discover(backends: &BackendsSection) -> Self {
        let mut registry = Self::new();
        let mut seen = HashSet::new();

        for path in candidate_paths(backends) {
            let Some(backend_id) = ExternalBackend::id_from_path(&path) else {
                continue;
            };
            if !backends.is_enabled(&backend_id) || !seen.insert(backend_id.clone()) {
                continue;
            }

            match ExternalBackend::spawn(path.clone(), backends.config_for(&backend_id)).await {
                Ok((descriptor, backend)) => {
                    tracing::info!(
                        backend_id = %descriptor.id,
                        executable = %path.display(),
                        "backend plugin disponibile"
                    );
                    registry.register_instance(descriptor, backend);
                }
                Err(error) => {
                    tracing::warn!(
                        backend_id = %backend_id,
                        executable = %path.display(),
                        error = %error,
                        "backend plugin ignorato"
                    );
                }
            }
        }

        registry
    }

    pub async fn select(
        &self,
        ir: &SandboxIR,
    ) -> Result<(String, Arc<dyn SandboxBackend>), RegistryError> {
        if let Some(hint) = &ir.backend_hint {
            let backend = self
                .instances
                .get(hint)
                .cloned()
                .ok_or_else(|| RegistryError::RequestedUnavailable(hint.clone()))?;
            backend
                .can_satisfy(ir)
                .await
                .map_err(|error| RegistryError::Unsatisfied(hint.clone(), error))?;
            return Ok((hint.clone(), backend));
        }

        for (backend_id, backend) in &self.instances {
            if backend.can_satisfy(ir).await.is_ok() {
                return Ok((backend_id.clone(), backend.clone()));
            }
        }

        if self.instances.is_empty() {
            Err(RegistryError::NoneAvailable)
        } else {
            Err(RegistryError::NoCompatibleBackend)
        }
    }

    pub fn get(&self, backend_id: &str) -> Result<Arc<dyn SandboxBackend>, RegistryError> {
        self.instances
            .get(backend_id)
            .cloned()
            .ok_or_else(|| RegistryError::RequestedUnavailable(backend_id.to_string()))
    }

    pub fn available_descriptor(&self, backend_id: &str) -> Option<&PluginDescriptor> {
        self.descriptors.get(backend_id)
    }

    pub fn list_available(&self) -> Vec<&PluginDescriptor> {
        self.descriptors.values().collect()
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn candidate_paths(backends: &BackendsSection) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.extend(backends.search_dirs.iter().cloned().map(PathBuf::from));
    if let Some(path_var) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path_var));
    }

    let mut candidates = Vec::new();
    let mut seen_paths = HashSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !file_name.starts_with("agentsandbox-backend-") {
                continue;
            }
            if seen_paths.insert(path.clone()) {
                candidates.push(path);
            }
        }
    }
    candidates
}

#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("nessun backend disponibile")]
    NoneAvailable,
    #[error("nessun backend compatibile con la sandbox richiesta")]
    NoCompatibleBackend,
    #[error("backend '{0}' richiesto ma non disponibile")]
    RequestedUnavailable(String),
    #[error("backend '{0}' non soddisfa la richiesta: {1}")]
    Unsatisfied(String, BackendError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsandbox_sdk::{
        backend::{ExecResult, SandboxState, SandboxStatus},
        plugin::{PluginCapabilities, PluginDescriptor},
    };
    use async_trait::async_trait;
    use chrono::Utc;

    struct TestBackend {
        supported: bool,
        backend_id: String,
    }

    #[async_trait]
    impl SandboxBackend for TestBackend {
        async fn create(&self, _ir: &SandboxIR) -> Result<String, BackendError> {
            Ok("handle".into())
        }

        async fn exec(
            &self,
            _handle: &str,
            _command: &str,
            _timeout_ms: Option<u64>,
        ) -> Result<ExecResult, BackendError> {
            Ok(ExecResult {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
                duration_ms: 0,
                resource_usage: None,
            })
        }

        async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
            Ok(SandboxStatus {
                sandbox_id: handle.into(),
                state: SandboxState::Running,
                created_at: Utc::now(),
                expires_at: Utc::now(),
                backend_id: self.backend_id.clone(),
            })
        }

        async fn destroy(&self, _handle: &str) -> Result<(), BackendError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), BackendError> {
            Ok(())
        }

        async fn can_satisfy(&self, _ir: &SandboxIR) -> Result<(), BackendError> {
            if self.supported {
                Ok(())
            } else {
                Err(BackendError::NotSupported("unsupported".into()))
            }
        }
    }

    fn descriptor(id: &str) -> PluginDescriptor {
        PluginDescriptor {
            id: id.into(),
            display_name: id.into(),
            version: "test".into(),
            trait_version: "1".into(),
            capabilities: PluginCapabilities {
                network_isolation: false,
                memory_hard_limit: false,
                cpu_hard_limit: false,
                persistent_storage: false,
                self_contained: false,
                isolation_level: "Process".into(),
                supported_presets: vec!["python".into()],
                rootless: false,
                snapshot_restore: false,
            },
            extensions_schema: None,
        }
    }

    #[tokio::test]
    async fn select_prefers_compatible_backend() {
        let mut registry = BackendRegistry::new();
        registry.register_instance(
            descriptor("docker"),
            Arc::new(TestBackend {
                supported: false,
                backend_id: "docker".into(),
            }),
        );
        registry.register_instance(
            descriptor("wasmtime"),
            Arc::new(TestBackend {
                supported: true,
                backend_id: "wasmtime".into(),
            }),
        );

        let (backend_id, backend) = registry.select(&SandboxIR::default()).await.unwrap();
        let status = backend.status("sandbox-1").await.unwrap();
        assert_eq!(backend_id, "wasmtime");
        assert_eq!(status.backend_id, "wasmtime");
    }

    #[tokio::test]
    async fn select_respects_backend_hint() {
        let mut registry = BackendRegistry::new();
        registry.register_instance(
            descriptor("podman"),
            Arc::new(TestBackend {
                supported: true,
                backend_id: "podman".into(),
            }),
        );

        let ir = SandboxIR {
            backend_hint: Some("podman".into()),
            ..SandboxIR::default()
        };
        let (backend_id, _) = registry.select(&ir).await.unwrap();
        assert_eq!(backend_id, "podman");
    }
}
