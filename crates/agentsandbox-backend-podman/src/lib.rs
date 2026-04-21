mod factory;

pub use factory::{default_podman_socket, PodmanBackendFactory};

use agentsandbox_sdk::{
    backend::{ExecResult, SandboxBackend, SandboxStatus},
    error::BackendError,
    ir::SandboxIR,
};
use async_trait::async_trait;

pub struct PodmanBackend {
    inner: Box<dyn SandboxBackend>,
}

impl PodmanBackend {
    pub fn new(inner: Box<dyn SandboxBackend>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl SandboxBackend for PodmanBackend {
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
        status.backend_id = "podman".into();
        Ok(status)
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        self.inner.destroy(handle).await
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        self.inner
            .health_check()
            .await
            .map_err(|error| match error {
                BackendError::Unavailable(message) => BackendError::Unavailable(format!(
                    "Podman non disponibile o socket non raggiungibile: {message}"
                )),
                other => other,
            })
    }

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        self.inner.can_satisfy(ir).await
    }

    async fn upload_file(
        &self,
        handle: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), BackendError> {
        self.inner.upload_file(handle, path, content).await
    }

    async fn download_file(&self, handle: &str, path: &str) -> Result<Vec<u8>, BackendError> {
        self.inner.download_file(handle, path).await
    }

    async fn snapshot(&self, handle: &str) -> Result<String, BackendError> {
        self.inner.snapshot(handle).await
    }

    async fn restore(&self, snapshot_id: &str, ir: &SandboxIR) -> Result<String, BackendError> {
        self.inner.restore(snapshot_id, ir).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsandbox_sdk::backend::SandboxState;
    use chrono::Utc;

    struct UnavailableBackend;

    #[async_trait]
    impl SandboxBackend for UnavailableBackend {
        async fn create(&self, _ir: &SandboxIR) -> Result<String, BackendError> {
            unreachable!()
        }

        async fn exec(
            &self,
            _handle: &str,
            _command: &str,
            _timeout_ms: Option<u64>,
        ) -> Result<ExecResult, BackendError> {
            unreachable!()
        }

        async fn status(&self, _handle: &str) -> Result<SandboxStatus, BackendError> {
            Ok(SandboxStatus {
                sandbox_id: "sandbox-1".into(),
                state: SandboxState::Running,
                created_at: Utc::now(),
                expires_at: Utc::now(),
                backend_id: "docker".into(),
            })
        }

        async fn destroy(&self, _handle: &str) -> Result<(), BackendError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), BackendError> {
            Err(BackendError::Unavailable(
                "No such file or directory".into(),
            ))
        }
    }

    #[tokio::test]
    async fn health_check_wraps_unavailable_message_for_podman() {
        let backend = PodmanBackend::new(Box::new(UnavailableBackend));
        let error = backend.health_check().await.unwrap_err();
        match error {
            BackendError::Unavailable(message) => {
                assert!(message.contains("Podman non disponibile"));
                assert!(message.contains("No such file or directory"));
            }
            other => panic!("errore inatteso: {other}"),
        }
    }

    #[tokio::test]
    async fn status_reports_podman_backend_id() {
        let backend = PodmanBackend::new(Box::new(UnavailableBackend));
        let status = backend.status("sandbox-1").await.unwrap();
        assert_eq!(status.backend_id, "podman");
    }
}
