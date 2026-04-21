use agentsandbox_sdk::{
    backend::{
        BackendCapabilities, BackendDescriptor, BackendFactory, IsolationLevel, SandboxBackend,
    },
    error::BackendError,
};
use std::collections::HashMap;

pub struct DockerBackendFactory;

impl BackendFactory for DockerBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "docker",
            display_name: "Docker",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: false,
                isolation_level: IsolationLevel::Container,
                supported_presets: vec!["python", "node", "rust", "shell"],
                rootless: false,
                snapshot_restore: false,
            },
            extensions_schema: Some(include_str!("../schema/extensions.schema.json")),
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let socket = config
            .get("socket")
            .map(String::as_str)
            .unwrap_or("/var/run/docker.sock");

        let client = bollard::Docker::connect_with_unix(socket, 30, bollard::API_DEFAULT_VERSION)
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;

        Ok(Box::new(crate::DockerBackend::with_client(client)))
    }
}
