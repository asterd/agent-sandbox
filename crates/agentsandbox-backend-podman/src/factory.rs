use crate::PodmanBackend;
use agentsandbox_sdk::{
    backend::{
        BackendCapabilities, BackendDescriptor, BackendFactory, IsolationLevel, SandboxBackend,
    },
    error::BackendError,
};
use std::collections::HashMap;

pub struct PodmanBackendFactory;

impl BackendFactory for PodmanBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "podman",
            display_name: "Podman (rootless)",
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
                rootless: true,
                snapshot_restore: false,
            },
            extensions_schema: agentsandbox_backend_docker::DockerBackendFactory
                .describe()
                .extensions_schema,
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let mut docker_config = HashMap::new();
        let socket = config
            .get("socket")
            .cloned()
            .unwrap_or_else(default_podman_socket);
        docker_config.insert("socket".into(), socket);

        let inner = agentsandbox_backend_docker::DockerBackendFactory.create(&docker_config)?;
        Ok(Box::new(PodmanBackend::new(inner)))
    }
}

pub fn default_podman_socket() -> String {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let runtime_dir = runtime_dir.trim();
        if !runtime_dir.is_empty() {
            return format!("{runtime_dir}/podman/podman.sock");
        }
    }

    if let Ok(uid) = std::env::var("UID") {
        let uid = uid.trim();
        if !uid.is_empty() {
            return format!("/run/user/{uid}/podman/podman.sock");
        }
    }

    "/run/podman/podman.sock".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        fn clear(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn default_socket_prefers_xdg_runtime_dir() {
        let _guard = env_lock().lock().unwrap();
        let _xdg = EnvGuard::set("XDG_RUNTIME_DIR", "/tmp/runtime-user");
        let _uid = EnvGuard::set("UID", "1234");
        assert_eq!(
            default_podman_socket(),
            "/tmp/runtime-user/podman/podman.sock"
        );
    }

    #[test]
    fn default_socket_falls_back_to_uid_when_runtime_dir_missing() {
        let _guard = env_lock().lock().unwrap();
        let _xdg = EnvGuard::clear("XDG_RUNTIME_DIR");
        let _uid = EnvGuard::set("UID", "1234");
        assert_eq!(default_podman_socket(), "/run/user/1234/podman/podman.sock");
    }

    #[test]
    fn default_socket_uses_rootful_socket_as_last_resort() {
        let _guard = env_lock().lock().unwrap();
        let _xdg = EnvGuard::clear("XDG_RUNTIME_DIR");
        let _uid = EnvGuard::clear("UID");
        assert_eq!(default_podman_socket(), "/run/podman/podman.sock");
    }

    #[test]
    fn descriptor_marks_backend_as_rootless() {
        let descriptor = PodmanBackendFactory.describe();
        assert_eq!(descriptor.id, "podman");
        assert!(descriptor.capabilities.rootless);
        assert_eq!(
            descriptor.extensions_schema,
            Some(include_str!(
                "../../agentsandbox-backend-docker/schema/extensions.schema.json"
            ))
        );
    }
}
