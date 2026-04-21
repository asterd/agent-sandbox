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
