use agentsandbox_sdk::{
    backend::{
        BackendCapabilities, BackendDescriptor, BackendFactory, ExecResult, IsolationLevel,
        SandboxBackend, SandboxStatus,
    },
    error::BackendError,
    ir::SandboxIR,
};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct LibkrunBackendFactory;

pub struct LibkrunBackend {
    inner: Box<dyn SandboxBackend>,
}

impl LibkrunBackend {
    fn missing_kvm_message() -> String {
        "/dev/kvm non trovato. libkrun richiede KVM e non e' supportato su macOS o host senza virtualizzazione annidata.".into()
    }
}

impl BackendFactory for LibkrunBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "libkrun",
            display_name: "libkrun MicroVM (Podman runtime)",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: false,
                isolation_level: IsolationLevel::MicroVM,
                supported_presets: vec!["python", "node", "shell"],
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
            .cloned()
            .unwrap_or_else(agentsandbox_backend_podman::default_podman_socket);
        let runtime = config
            .get("runtime")
            .cloned()
            .unwrap_or_else(|| "krun".into());

        let client = bollard::Docker::connect_with_unix(&socket, 30, bollard::API_DEFAULT_VERSION)
            .map_err(|error| BackendError::Unavailable(error.to_string()))?;

        let inner = agentsandbox_backend_docker::DockerBackend::with_runtime(client, Some(runtime));
        Ok(Box::new(LibkrunBackend {
            inner: Box::new(inner),
        }))
    }
}

#[async_trait]
impl SandboxBackend for LibkrunBackend {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        self.inner.create(ir).await
    }

    async fn exec(
        &self,
        handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError> {
        self.inner.exec(handle, command, timeout_ms).await
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        let mut status = self.inner.status(handle).await?;
        status.backend_id = "libkrun".into();
        Ok(status)
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        self.inner.destroy(handle).await
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        if !cfg!(target_os = "linux") {
            return Err(BackendError::Unavailable(Self::missing_kvm_message()));
        }
        if !std::path::Path::new("/dev/kvm").exists() {
            return Err(BackendError::Unavailable(Self::missing_kvm_message()));
        }
        self.inner.health_check().await
    }

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        if let Some(raw) = &ir.extensions {
            let section = raw.get("libkrun").cloned().unwrap_or_default();
            serde_json::from_value::<serde_json::Value>(section).map_err(|error| {
                BackendError::Configuration(format!("extensions.libkrun non valide: {error}"))
            })?;
        }
        self.inner.can_satisfy(ir).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_reports_microvm_capabilities() {
        let descriptor = LibkrunBackendFactory.describe();
        assert_eq!(descriptor.id, "libkrun");
        assert_eq!(
            descriptor.capabilities.isolation_level,
            IsolationLevel::MicroVM
        );
    }
}
