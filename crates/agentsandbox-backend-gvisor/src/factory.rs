use crate::GVisorBackend;
use agentsandbox_sdk::{
    backend::{
        BackendCapabilities, BackendDescriptor, BackendFactory, IsolationLevel, SandboxBackend,
    },
    error::BackendError,
};
use std::collections::HashMap;

pub struct GVisorBackendFactory;

impl BackendFactory for GVisorBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "gvisor",
            display_name: "gVisor (runsc)",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: false,
                isolation_level: IsolationLevel::KernelSandbox,
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
        let runtime = config
            .get("runtime")
            .cloned()
            .unwrap_or_else(|| "runsc".into());

        let client = bollard::Docker::connect_with_unix(socket, 30, bollard::API_DEFAULT_VERSION)
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;

        let inner = agentsandbox_backend_docker::DockerBackend::with_runtime(
            client.clone(),
            Some(runtime.clone()),
        );

        Ok(Box::new(GVisorBackend::new(
            Box::new(inner),
            client,
            runtime,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_reports_kernel_sandbox_capabilities() {
        let descriptor = GVisorBackendFactory.describe();
        assert_eq!(descriptor.id, "gvisor");
        assert_eq!(
            descriptor.capabilities.isolation_level,
            IsolationLevel::KernelSandbox
        );
        assert!(!descriptor.capabilities.rootless);
        assert_eq!(
            descriptor.extensions_schema,
            Some(include_str!("../schema/extensions.schema.json"))
        );
    }
}
