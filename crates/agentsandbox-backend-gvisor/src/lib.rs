mod factory;

pub use factory::GVisorBackendFactory;

use agentsandbox_sdk::{
    backend::{ExecResult, SandboxBackend, SandboxStatus},
    error::BackendError,
    ir::SandboxIR,
};
use async_trait::async_trait;
use bollard::Docker;

const GVISOR_INSTALL_URL: &str = "https://gvisor.dev/docs/user_guide/install/";

pub struct GVisorBackend {
    inner: Box<dyn SandboxBackend>,
    client: Docker,
    runtime: String,
}

impl GVisorBackend {
    pub fn new(inner: Box<dyn SandboxBackend>, client: Docker, runtime: String) -> Self {
        Self {
            inner,
            client,
            runtime,
        }
    }

    fn missing_runtime_message(runtime: &str) -> String {
        format!(
            "runtime Docker '{}' non configurato. Installa gVisor e registra il runtime runsc in Docker: {}",
            runtime, GVISOR_INSTALL_URL
        )
    }

    fn parse_extensions(ir: &SandboxIR) -> Result<GVisorExtensions, BackendError> {
        match &ir.extensions {
            None => Ok(GVisorExtensions::default()),
            Some(raw) => {
                let section = raw.get("gvisor").cloned().unwrap_or_default();
                serde_json::from_value(section).map_err(|e| {
                    BackendError::Configuration(format!("extensions.gvisor non valide: {e}"))
                })
            }
        }
    }

    fn apply_extensions(&self, ir: &SandboxIR) -> Result<SandboxIR, BackendError> {
        let extensions = Self::parse_extensions(ir)?;
        let mut effective = ir.clone();

        match extensions.network {
            Some(GVisorNetwork::Sandbox) | None => {}
            Some(GVisorNetwork::Host) => {
                effective.egress.mode = agentsandbox_sdk::ir::EgressMode::Passthrough;
                effective.egress.deny_by_default = false;
                effective.egress.allow_hostnames.clear();
                effective.egress.allow_ips.clear();
            }
            Some(GVisorNetwork::None) => {
                effective.egress.mode = agentsandbox_sdk::ir::EgressMode::None;
                effective.egress.allow_hostnames.clear();
                effective.egress.allow_ips.clear();
            }
        }

        Ok(effective)
    }
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GVisorExtensions {
    network: Option<GVisorNetwork>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum GVisorNetwork {
    Sandbox,
    Host,
    None,
}

#[async_trait]
impl SandboxBackend for GVisorBackend {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let effective = self.apply_extensions(ir)?;
        self.inner.create(&effective).await
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
        status.backend_id = "gvisor".into();
        Ok(status)
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        self.inner.destroy(handle).await
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        self.client
            .ping()
            .await
            .map_err(|e| BackendError::Unavailable(format!("Docker non raggiungibile: {e}")))?;

        let info = self.client.info().await.map_err(|e| {
            BackendError::Unavailable(format!("impossibile leggere i runtime Docker: {e}"))
        })?;

        let runtime_available = info
            .runtimes
            .as_ref()
            .map(|runtimes| runtimes.contains_key(&self.runtime))
            .unwrap_or(false);

        if runtime_available {
            Ok(())
        } else {
            Err(BackendError::Unavailable(Self::missing_runtime_message(
                &self.runtime,
            )))
        }
    }

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        let effective = self.apply_extensions(ir)?;
        self.inner.can_satisfy(&effective).await
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

    struct TestBackend;

    #[async_trait]
    impl SandboxBackend for TestBackend {
        async fn create(&self, _ir: &SandboxIR) -> Result<String, BackendError> {
            Ok("sandbox-1".into())
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
                backend_id: "docker".into(),
            })
        }

        async fn destroy(&self, _handle: &str) -> Result<(), BackendError> {
            Ok(())
        }

        async fn health_check(&self) -> Result<(), BackendError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn status_reports_gvisor_backend_id() {
        let client = Docker::connect_with_local_defaults().unwrap();
        let backend = GVisorBackend::new(Box::new(TestBackend), client, "runsc".into());
        let status = backend.status("sandbox-1").await.unwrap();
        assert_eq!(status.backend_id, "gvisor");
    }

    #[test]
    fn missing_runtime_message_mentions_runtime_and_install_url() {
        let message = GVisorBackend::missing_runtime_message("runsc-kvm");
        assert!(message.contains("runsc-kvm"));
        assert!(message.contains(GVISOR_INSTALL_URL));
    }

    #[test]
    fn parse_extensions_accepts_supported_network_override() {
        let mut ir = SandboxIR::default();
        ir.extensions = Some(serde_json::json!({
            "gvisor": {
                "network": "host"
            }
        }));

        let effective = GVisorBackend::apply_extensions(
            &GVisorBackend::new(
                Box::new(TestBackend),
                Docker::connect_with_local_defaults().unwrap(),
                "runsc".into(),
            ),
            &ir,
        )
        .unwrap();

        assert_eq!(
            effective.egress.mode,
            agentsandbox_sdk::ir::EgressMode::Passthrough
        );
        assert!(!effective.egress.deny_by_default);
    }

    #[test]
    fn parse_extensions_rejects_unknown_fields() {
        let mut ir = SandboxIR::default();
        ir.extensions = Some(serde_json::json!({
            "gvisor": {
                "platform": "systrap"
            }
        }));

        let error = GVisorBackend::parse_extensions(&ir).unwrap_err();
        match error {
            BackendError::Configuration(message) => {
                assert!(message.contains("extensions.gvisor non valide"));
            }
            other => panic!("errore inatteso: {other}"),
        }
    }
}
