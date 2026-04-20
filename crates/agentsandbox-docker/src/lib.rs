//! Docker backend adapter for AgentSandbox.
//!
//! Implements [`agentsandbox_core::SandboxAdapter`] on top of the Docker
//! Engine API via `bollard`. The adapter is intentionally thin: it maps an
//! [`SandboxIR`] to a long-lived `sleep` container and forwards `exec` calls
//! to `docker exec`. Container handles never leak out — the daemon only ever
//! sees the IR id.

use agentsandbox_core::adapter::{
    AdapterError, ExecResult, SandboxAdapter, SandboxInfo, SandboxStatus,
};
use agentsandbox_core::ir::SandboxIR;
use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, LogOutput,
    RemoveContainerOptions, StartContainerOptions,
};
use bollard::errors::Error as BollardError;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::HostConfig;
use bollard::Docker;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use std::collections::HashMap;

/// Prefix applied to every container name and label so we can recognise our
/// own containers when enumerating the Docker daemon.
const CONTAINER_NAME_PREFIX: &str = "agentsandbox-";
const OWNER_LABEL: &str = "ai.sandbox.owner";
const OWNER_LABEL_VALUE: &str = "agentsandbox";
const ID_LABEL: &str = "ai.sandbox.id";

pub struct DockerAdapter {
    client: Docker,
}

impl DockerAdapter {
    /// Connect to the local Docker daemon using platform defaults
    /// (unix socket on *nix, named pipe on Windows).
    pub fn new() -> Result<Self, AdapterError> {
        let client = Docker::connect_with_local_defaults()
            .map_err(|e| AdapterError::BackendUnavailable(e.to_string()))?;
        Ok(Self { client })
    }

    /// Wrap an existing `bollard::Docker` (used in tests that inject a mock
    /// or alternate connection).
    pub fn with_client(client: Docker) -> Self {
        Self { client }
    }

    fn container_name(sandbox_id: &str) -> String {
        format!("{CONTAINER_NAME_PREFIX}{sandbox_id}")
    }

    /// Pick a Docker network mode for this IR.
    ///
    /// * `deny_by_default=true` + no allow list → `none` (fully offline).
    /// * `deny_by_default=false` OR any allow list → `bridge` with a loud
    ///   warning, because v1alpha1 does NOT yet enforce the allowlist.
    ///   Fase 6 replaces this with a real L4 filter; do not silently
    ///   degrade — the whole point of egress rules is defence in depth.
    fn network_mode_for(ir: &SandboxIR) -> &'static str {
        if ir.deny_by_default && ir.egress_allow.is_empty() {
            "none"
        } else {
            tracing::warn!(
                sandbox_id = %ir.id,
                allow = ?ir.egress_allow,
                deny_by_default = ir.deny_by_default,
                "egress allowlist non ancora applicata in v1alpha1-docker: \
                 la sandbox avrà accesso di rete non filtrato. Vedi Fase 6."
            );
            "bridge"
        }
    }

    fn map_inspect_err(e: BollardError, sandbox_id: &str) -> AdapterError {
        match e {
            BollardError::DockerResponseServerError {
                status_code: 404, ..
            } => AdapterError::NotFound(sandbox_id.to_string()),
            other => AdapterError::Internal(other.to_string()),
        }
    }

    /// `bollard` returns a 404 for missing containers. We treat that as a
    /// successful destroy so callers can use this method idempotently.
    fn swallow_not_found(e: BollardError) -> Result<(), AdapterError> {
        match e {
            BollardError::DockerResponseServerError {
                status_code: 404, ..
            } => Ok(()),
            other => Err(AdapterError::Internal(other.to_string())),
        }
    }
}

#[async_trait]
impl SandboxAdapter for DockerAdapter {
    async fn create(&self, ir: &SandboxIR) -> Result<String, AdapterError> {
        // Flatten env + secret_env: Docker has a single ENV concept. Secrets
        // are already resolved; they never appear in logs because SandboxIR
        // redacts them in Debug.
        let mut env: Vec<String> = ir
            .env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        env.extend(ir.secret_env.iter().map(|(k, v)| format!("{k}={v}")));

        let mut labels = HashMap::new();
        labels.insert(OWNER_LABEL.to_string(), OWNER_LABEL_VALUE.to_string());
        labels.insert(ID_LABEL.to_string(), ir.id.clone());

        let host_config = HostConfig {
            memory: Some(i64::from(ir.memory_mb) * 1024 * 1024),
            nano_cpus: Some(i64::from(ir.cpu_millicores) * 1_000_000),
            network_mode: Some(Self::network_mode_for(ir).to_string()),
            auto_remove: Some(false),
            ..Default::default()
        };

        let config = Config {
            image: Some(ir.image.clone()),
            env: Some(env),
            working_dir: Some(ir.working_dir.clone()),
            host_config: Some(host_config),
            labels: Some(labels),
            // Keep the container alive for at most ttl_seconds. The daemon's
            // reaper enforces TTL independently; this is the backstop.
            cmd: Some(vec!["sleep".to_string(), ir.ttl_seconds.to_string()]),
            ..Default::default()
        };

        let name = Self::container_name(&ir.id);
        let container = self
            .client
            .create_container(
                Some(CreateContainerOptions {
                    name: name.as_str(),
                    platform: None,
                }),
                config,
            )
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        self.client
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        Ok(ir.id.clone())
    }

    async fn exec(&self, sandbox_id: &str, command: &str) -> Result<ExecResult, AdapterError> {
        let name = Self::container_name(sandbox_id);
        let start = std::time::Instant::now();

        let exec = self
            .client
            .create_exec(
                &name,
                CreateExecOptions {
                    cmd: Some(vec!["sh", "-c", command]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| Self::map_inspect_err(e, sandbox_id))?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let StartExecResults::Attached { mut output, .. } = self
            .client
            .start_exec(&exec.id, None)
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?
        {
            while let Some(chunk) = output.next().await {
                let msg = chunk.map_err(|e| AdapterError::Internal(e.to_string()))?;
                match msg {
                    LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    // StdIn never surfaces on exec; Console appears only when
                    // a TTY is attached — we don't attach one.
                    _ => {}
                }
            }
        }

        let inspect = self
            .client
            .inspect_exec(&exec.id)
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        let exit_code = inspect.exit_code.unwrap_or(-1);

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn inspect(&self, sandbox_id: &str) -> Result<SandboxInfo, AdapterError> {
        let name = Self::container_name(sandbox_id);
        let info = self
            .client
            .inspect_container(&name, None::<InspectContainerOptions>)
            .await
            .map_err(|e| Self::map_inspect_err(e, sandbox_id))?;

        let status = match info.state.as_ref().and_then(|s| s.running) {
            Some(true) => SandboxStatus::Running,
            Some(false) => SandboxStatus::Stopped,
            None => SandboxStatus::Error("stato sconosciuto".into()),
        };

        // Docker reports `created` as an RFC3339 timestamp. If parsing fails
        // we fall back to `now()` but never propagate the error: created_at
        // is informational here — the daemon's SQLite row is authoritative.
        let created_at = info
            .created
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Ok(SandboxInfo {
            sandbox_id: sandbox_id.to_string(),
            status,
            created_at,
            expires_at: created_at,
        })
    }

    async fn destroy(&self, sandbox_id: &str) -> Result<(), AdapterError> {
        let name = Self::container_name(sandbox_id);
        match self
            .client
            .remove_container(
                &name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => Self::swallow_not_found(e),
        }
    }

    fn backend_name(&self) -> &'static str {
        "docker"
    }

    async fn health_check(&self) -> Result<(), AdapterError> {
        self.client
            .ping()
            .await
            .map(|_| ())
            .map_err(|e| AdapterError::BackendUnavailable(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_mode_defaults_to_none_when_deny_all() {
        let ir = SandboxIR::default();
        assert_eq!(DockerAdapter::network_mode_for(&ir), "none");
    }

    #[test]
    fn network_mode_is_bridge_when_allow_list_present() {
        let ir = SandboxIR {
            egress_allow: vec!["pypi.org".into()],
            ..SandboxIR::default()
        };
        assert_eq!(DockerAdapter::network_mode_for(&ir), "bridge");
    }

    #[test]
    fn network_mode_is_bridge_when_deny_by_default_false() {
        let ir = SandboxIR {
            deny_by_default: false,
            ..SandboxIR::default()
        };
        assert_eq!(DockerAdapter::network_mode_for(&ir), "bridge");
    }

    #[test]
    fn container_name_uses_prefix() {
        assert_eq!(
            DockerAdapter::container_name("abcd-1234"),
            "agentsandbox-abcd-1234"
        );
    }
}
