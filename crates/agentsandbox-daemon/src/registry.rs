use agentsandbox_sdk::{
    backend::{BackendDescriptor, BackendFactory, SandboxBackend},
    ir::SandboxIR,
};
use std::{collections::HashMap, sync::Arc};

pub struct BackendRegistry {
    descriptors: HashMap<String, BackendDescriptor>,
    instances: HashMap<String, Arc<dyn SandboxBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
            instances: HashMap::new(),
        }
    }

    pub fn register(&mut self, factory: &dyn BackendFactory) {
        let descriptor = factory.describe();
        tracing::info!(
            backend_id = descriptor.id,
            version = descriptor.version,
            trait_version = descriptor.trait_version(),
            "backend registrato"
        );
        self.descriptors
            .insert(descriptor.id.to_string(), descriptor);
    }

    pub async fn initialize(
        &mut self,
        factory: &dyn BackendFactory,
        config: &HashMap<String, String>,
    ) {
        let descriptor = factory.describe();
        match factory.create(config) {
            Ok(backend) => match backend.health_check().await {
                Ok(()) => {
                    tracing::info!(backend_id = descriptor.id, "backend healthy");
                    self.instances
                        .insert(descriptor.id.to_string(), Arc::from(backend));
                }
                Err(error) => {
                    tracing::warn!(
                        backend_id = descriptor.id,
                        error = %error,
                        "backend health check fallito"
                    );
                }
            },
            Err(error) => {
                tracing::warn!(
                    backend_id = descriptor.id,
                    error = %error,
                    "backend inizializzazione fallita"
                );
            }
        }
    }

    pub fn select(
        &self,
        ir: &SandboxIR,
    ) -> Result<(String, Arc<dyn SandboxBackend>), RegistryError> {
        if let Some(hint) = &ir.backend_hint {
            let backend = self
                .instances
                .get(hint)
                .cloned()
                .ok_or_else(|| RegistryError::RequestedUnavailable(hint.clone()))?;
            return Ok((hint.clone(), backend));
        }

        self.instances
            .iter()
            .next()
            .map(|(backend_id, backend)| (backend_id.clone(), backend.clone()))
            .ok_or(RegistryError::NoneAvailable)
    }

    pub fn get(&self, backend_id: &str) -> Result<Arc<dyn SandboxBackend>, RegistryError> {
        self.instances
            .get(backend_id)
            .cloned()
            .ok_or_else(|| RegistryError::RequestedUnavailable(backend_id.to_string()))
    }

    pub fn list_available(&self) -> Vec<&BackendDescriptor> {
        self.descriptors
            .values()
            .filter(|descriptor| self.instances.contains_key(descriptor.id))
            .collect()
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("nessun backend disponibile")]
    NoneAvailable,
    #[error("backend '{0}' richiesto ma non disponibile")]
    RequestedUnavailable(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsandbox_sdk::{
        backend::{
            BackendCapabilities, BackendDescriptor, BackendFactory, ExecResult, IsolationLevel,
            SandboxState, SandboxStatus,
        },
        error::BackendError,
    };
    use async_trait::async_trait;
    use chrono::Utc;

    struct TestFactory {
        descriptor: BackendDescriptor,
    }

    impl BackendFactory for TestFactory {
        fn describe(&self) -> BackendDescriptor {
            self.descriptor.clone()
        }

        fn create(
            &self,
            _config: &HashMap<String, String>,
        ) -> Result<Box<dyn SandboxBackend>, BackendError> {
            Ok(Box::new(TestBackend {
                backend_id: self.descriptor.id,
            }))
        }
    }

    struct TestBackend {
        backend_id: &'static str,
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
                backend_id: self.backend_id.into(),
            })
        }

        async fn destroy(&self, _handle: &str) -> Result<(), BackendError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), BackendError> {
            Ok(())
        }
    }

    fn make_factory(id: &'static str) -> TestFactory {
        TestFactory {
            descriptor: BackendDescriptor {
                id,
                display_name: id,
                version: "test",
                trait_version: "1",
                capabilities: BackendCapabilities {
                    isolation_level: IsolationLevel::Container,
                    ..Default::default()
                },
                extensions_schema: None,
            },
        }
    }

    #[tokio::test]
    async fn select_uses_requested_backend_hint() {
        let mut registry = BackendRegistry::new();
        let docker = make_factory("docker");
        let podman = make_factory("podman");

        registry.register(&docker);
        registry.initialize(&docker, &HashMap::new()).await;
        registry.register(&podman);
        registry.initialize(&podman, &HashMap::new()).await;

        let ir = SandboxIR {
            backend_hint: Some("podman".into()),
            ..SandboxIR::default()
        };

        let (backend_id, backend) = registry.select(&ir).unwrap();
        let status = backend.status("sandbox-1").await.unwrap();
        assert_eq!(backend_id, "podman");
        assert_eq!(status.backend_id, "podman");
    }
}
