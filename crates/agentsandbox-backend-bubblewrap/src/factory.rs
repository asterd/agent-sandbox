use crate::BubblewrapBackend;
use agentsandbox_sdk::{
    backend::{
        BackendCapabilities, BackendDescriptor, BackendFactory, IsolationLevel, SandboxBackend,
    },
    error::BackendError,
};
use std::collections::HashMap;

pub struct BubblewrapBackendFactory;

pub fn default_rootfs_base() -> String {
    std::env::temp_dir()
        .join("agentsandbox-bubblewrap")
        .to_string_lossy()
        .into_owned()
}

impl BackendFactory for BubblewrapBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "bubblewrap",
            display_name: if cfg!(target_os = "macos") {
                "sandbox-exec (macOS Seatbelt)"
            } else {
                "Bubblewrap"
            },
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: false,
                cpu_hard_limit: false,
                persistent_storage: false,
                self_contained: false,
                isolation_level: IsolationLevel::Process,
                supported_presets: vec!["python", "node", "rust", "shell"],
                rootless: true,
                snapshot_restore: false,
            },
            extensions_schema: Some(include_str!("../schema/extensions.schema.json")),
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let bwrap_path = config
            .get("bwrap_path")
            .cloned()
            .unwrap_or_else(|| "bwrap".to_string());
        let rootfs_base = config
            .get("rootfs_base")
            .cloned()
            .unwrap_or_else(default_rootfs_base);
        Ok(Box::new(BubblewrapBackend::new(bwrap_path, rootfs_base)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_reports_process_isolation() {
        let descriptor = BubblewrapBackendFactory.describe();
        assert_eq!(descriptor.id, "bubblewrap");
        assert_eq!(
            descriptor.capabilities.isolation_level,
            IsolationLevel::Process
        );
        assert!(descriptor.capabilities.rootless);
        assert_eq!(
            descriptor.extensions_schema,
            Some(include_str!("../schema/extensions.schema.json"))
        );
    }
}
