mod factory;

pub use factory::DockerBackendFactory;

use agentsandbox_proxy::{EgressProxy, RunningEgressProxy};
use agentsandbox_sdk::{
    backend::{ExecResult, ResourceUsage, SandboxBackend, SandboxState, SandboxStatus},
    error::BackendError,
    ir::{EgressMode, SandboxIR},
};
use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, LogOutput, RemoveContainerOptions,
    StartContainerOptions,
};
use bollard::errors::Error as BollardError;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::{DeviceMapping, HostConfig, ResourcesUlimits};
use bollard::Docker;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use std::{collections::HashMap, sync::Mutex};
use tokio::time::{timeout, Duration};

const CONTAINER_NAME_PREFIX: &str = "agentsandbox-";
const OWNER_LABEL: &str = "ai.sandbox.owner";
const OWNER_LABEL_VALUE: &str = "agentsandbox";
const ID_LABEL: &str = "ai.sandbox.id";

pub struct DockerBackend {
    client: Docker,
    runtime: Option<String>,
    proxy_tasks: Mutex<HashMap<String, RunningEgressProxy>>,
}

impl DockerBackend {
    pub fn with_client(client: Docker) -> Self {
        Self {
            client,
            runtime: None,
            proxy_tasks: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_runtime(client: Docker, runtime: Option<String>) -> Self {
        Self {
            client,
            runtime,
            proxy_tasks: Mutex::new(HashMap::new()),
        }
    }

    fn container_name(sandbox_id: &str) -> String {
        format!("{CONTAINER_NAME_PREFIX}{sandbox_id}")
    }

    fn legacy_container_name(handle: &str) -> Option<String> {
        if handle.starts_with(CONTAINER_NAME_PREFIX) {
            None
        } else {
            Some(Self::container_name(handle))
        }
    }

    fn network_mode_for(ir: &SandboxIR) -> &'static str {
        if ir.egress.mode == EgressMode::None {
            return "none";
        }
        if ir.egress.mode == EgressMode::Passthrough {
            return "bridge";
        }
        if ir.egress.deny_by_default && ir.egress.allow_hostnames.is_empty() {
            "none"
        } else {
            "bridge"
        }
    }

    fn should_apply_egress_rules(ir: &SandboxIR) -> bool {
        ir.egress.mode == EgressMode::Proxy
            && ir.egress.deny_by_default
            && !ir.egress.allow_hostnames.is_empty()
    }

    fn configure_proxy_env(env: &mut Vec<String>, host_config: &mut HostConfig, proxy_port: u16) {
        let http_proxy_url = format!("http://host.docker.internal:{proxy_port}");
        let socks_proxy_url = format!("socks5h://host.docker.internal:{proxy_port}");
        env.push(format!("HTTP_PROXY={http_proxy_url}"));
        env.push(format!("HTTPS_PROXY={http_proxy_url}"));
        env.push(format!("ALL_PROXY={socks_proxy_url}"));
        env.push("NO_PROXY=127.0.0.1,localhost".to_string());

        let extra_host = "host.docker.internal:host-gateway".to_string();
        match &mut host_config.extra_hosts {
            Some(extra_hosts) => {
                if !extra_hosts.iter().any(|value| value == &extra_host) {
                    extra_hosts.push(extra_host);
                }
            }
            None => host_config.extra_hosts = Some(vec![extra_host]),
        }
    }

    fn register_proxy_task(&self, handle: String, proxy: RunningEgressProxy) {
        self.proxy_tasks.lock().unwrap().insert(handle, proxy);
    }

    fn abort_proxy_task(&self, handle: &str) {
        if let Some(proxy) = self.proxy_tasks.lock().unwrap().remove(handle) {
            proxy.abort();
        }
    }

    fn map_missing(e: BollardError, handle: &str) -> BackendError {
        match e {
            BollardError::DockerResponseServerError {
                status_code: 404, ..
            } => BackendError::NotFound(handle.to_string()),
            other => BackendError::Internal(other.to_string()),
        }
    }

    fn map_remove(e: BollardError) -> Result<(), BackendError> {
        match e {
            BollardError::DockerResponseServerError {
                status_code: 404, ..
            } => Ok(()),
            other => Err(BackendError::Internal(other.to_string())),
        }
    }

    fn parse_extensions(ir: &SandboxIR) -> Result<DockerExtensions, BackendError> {
        match &ir.extensions {
            None => Ok(DockerExtensions::default()),
            Some(raw) => {
                let section = raw.get("docker").cloned().unwrap_or_default();
                serde_json::from_value(section).map_err(|e| {
                    BackendError::Configuration(format!("extensions.docker non valide: {e}"))
                })
            }
        }
    }

    async fn create_exec_with_fallback(
        &self,
        handle: &str,
        options: CreateExecOptions<String>,
    ) -> Result<String, BackendError> {
        match self.client.create_exec(handle, options.clone()).await {
            Ok(exec) => Ok(exec.id),
            Err(BollardError::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                let Some(legacy_name) = Self::legacy_container_name(handle) else {
                    return Err(BackendError::NotFound(handle.to_string()));
                };
                self.client
                    .create_exec(&legacy_name, options)
                    .await
                    .map(|exec| exec.id)
                    .map_err(|e| Self::map_missing(e, handle))
            }
            Err(error) => Err(BackendError::Internal(error.to_string())),
        }
    }

    async fn inspect_container_with_fallback(
        &self,
        handle: &str,
    ) -> Result<bollard::models::ContainerInspectResponse, BackendError> {
        match self
            .client
            .inspect_container(handle, None::<InspectContainerOptions>)
            .await
        {
            Ok(info) => Ok(info),
            Err(BollardError::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                let Some(legacy_name) = Self::legacy_container_name(handle) else {
                    return Err(BackendError::NotFound(handle.to_string()));
                };
                self.client
                    .inspect_container(&legacy_name, None::<InspectContainerOptions>)
                    .await
                    .map_err(|e| Self::map_missing(e, handle))
            }
            Err(error) => Err(BackendError::Internal(error.to_string())),
        }
    }

    async fn remove_container_with_fallback(&self, handle: &str) -> Result<(), BackendError> {
        match self
            .client
            .remove_container(
                handle,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(BollardError::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                let Some(legacy_name) = Self::legacy_container_name(handle) else {
                    return Ok(());
                };
                match self
                    .client
                    .remove_container(
                        &legacy_name,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(error) => Self::map_remove(error),
                }
            }
            Err(error) => Self::map_remove(error),
        }
    }
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DockerExtensions {
    host_config: Option<DockerHostConfigExt>,
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DockerHostConfigExt {
    cap_add: Option<Vec<String>>,
    cap_drop: Option<Vec<String>>,
    security_opt: Option<Vec<String>>,
    privileged: Option<bool>,
    shm_size_mb: Option<u64>,
    sysctls: Option<HashMap<String, String>>,
    ulimits: Option<Vec<DockerUlimit>>,
    devices: Option<Vec<DockerDevice>>,
    binds: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct DockerUlimit {
    name: String,
    soft: u64,
    hard: u64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerDevice {
    path_on_host: String,
    path_in_container: String,
    cgroup_permissions: String,
}

#[async_trait]
impl SandboxBackend for DockerBackend {
    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        Self::parse_extensions(ir)?;
        Ok(())
    }

    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let ext = Self::parse_extensions(ir)?;

        let mut env: Vec<String> = ir.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        env.extend(ir.secret_env.iter().map(|(k, v)| format!("{k}={v}")));

        let mut labels = ir.labels.clone();
        labels.insert(OWNER_LABEL.into(), OWNER_LABEL_VALUE.into());
        labels.insert(ID_LABEL.into(), ir.id.clone());

        let mut host_config = HostConfig {
            memory: Some(i64::from(ir.memory_mb) * 1024 * 1024),
            nano_cpus: Some(i64::from(ir.cpu_millicores) * 1_000_000),
            network_mode: Some(Self::network_mode_for(ir).to_string()),
            runtime: self.runtime.clone(),
            auto_remove: Some(false),
            ..Default::default()
        };

        if let Some(hc) = ext.host_config {
            if hc.privileged == Some(true) {
                tracing::warn!(sandbox_id = %ir.id, "docker extension privileged=true");
            }
            host_config.cap_add = hc.cap_add.or(host_config.cap_add);
            host_config.cap_drop = hc.cap_drop;
            host_config.security_opt = hc.security_opt;
            host_config.privileged = hc.privileged;
            host_config.sysctls = hc.sysctls;
            host_config.binds = hc.binds;
            host_config.devices = hc.devices.map(|devices| {
                devices
                    .into_iter()
                    .map(|device| DeviceMapping {
                        path_on_host: Some(device.path_on_host),
                        path_in_container: Some(device.path_in_container),
                        cgroup_permissions: Some(device.cgroup_permissions),
                    })
                    .collect()
            });
            host_config.ulimits = hc.ulimits.map(|limits| {
                limits
                    .into_iter()
                    .map(|limit| ResourcesUlimits {
                        name: Some(limit.name),
                        soft: Some(limit.soft as i64),
                        hard: Some(limit.hard as i64),
                    })
                    .collect()
            });
            if let Some(shm_size_mb) = hc.shm_size_mb {
                host_config.shm_size = Some((shm_size_mb * 1024 * 1024) as i64);
            }
        }

        let proxy = if Self::should_apply_egress_rules(ir) {
            let proxy = EgressProxy::start(ir.id.clone(), ir.egress.allow_hostnames.clone())
                .await
                .map_err(|error| {
                    BackendError::Internal(format!("impossibile avviare egress proxy: {error}"))
                })?;
            Self::configure_proxy_env(&mut env, &mut host_config, proxy.port());
            Some(proxy)
        } else {
            None
        };

        let container_config = Config {
            image: Some(ir.image.clone()),
            env: Some(env),
            working_dir: Some(ir.working_dir.clone()),
            host_config: Some(host_config),
            labels: Some(labels),
            cmd: Some(
                ir.command
                    .clone()
                    .unwrap_or_else(|| vec!["sleep".into(), ir.ttl_seconds.to_string()]),
            ),
            ..Default::default()
        };

        let name = Self::container_name(&ir.id);
        let container = match self
            .client
            .create_container(
                Some(CreateContainerOptions {
                    name: name.as_str(),
                    platform: None,
                }),
                container_config,
            )
            .await
        {
            Ok(container) => container,
            Err(error) => {
                if let Some(proxy) = proxy {
                    proxy.abort();
                }
                return Err(BackendError::Internal(error.to_string()));
            }
        };

        if let Err(error) = self
            .client
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
        {
            if let Some(proxy) = proxy {
                proxy.abort();
            }
            let _ = self
                .client
                .remove_container(
                    &name,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
            return Err(BackendError::Internal(error.to_string()));
        }

        if let Some(proxy) = proxy {
            self.register_proxy_task(container.id.clone(), proxy);
        }

        Ok(container.id)
    }

    async fn exec(
        &self,
        handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError> {
        let start = std::time::Instant::now();
        let exec_id = self
            .create_exec_with_fallback(
                handle,
                CreateExecOptions {
                    cmd: Some(vec![
                        "sh".to_string(),
                        "-c".to_string(),
                        command.to_string(),
                    ]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?;

        let run = async {
            let mut stdout = String::new();
            let mut stderr = String::new();

            if let StartExecResults::Attached { mut output, .. } = self
                .client
                .start_exec(&exec_id, None)
                .await
                .map_err(|e| BackendError::Internal(e.to_string()))?
            {
                while let Some(chunk) = output.next().await {
                    match chunk.map_err(|e| BackendError::Internal(e.to_string()))? {
                        LogOutput::StdOut { message } => {
                            stdout.push_str(&String::from_utf8_lossy(&message));
                        }
                        LogOutput::StdErr { message } => {
                            stderr.push_str(&String::from_utf8_lossy(&message));
                        }
                        _ => {}
                    }
                }
            }

            let inspect = self
                .client
                .inspect_exec(&exec_id)
                .await
                .map_err(|e| BackendError::Internal(e.to_string()))?;

            Ok(ExecResult {
                stdout,
                stderr,
                exit_code: inspect.exit_code.unwrap_or(-1),
                duration_ms: start.elapsed().as_millis() as u64,
                resource_usage: Some(ResourceUsage {
                    cpu_user_ms: None,
                    memory_peak_mb: None,
                }),
            })
        };

        if let Some(timeout_ms) = timeout_ms {
            timeout(Duration::from_millis(timeout_ms), run)
                .await
                .map_err(|_| BackendError::Timeout(timeout_ms))?
        } else {
            run.await
        }
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        let info = self.inspect_container_with_fallback(handle).await?;

        let state = match info.state.as_ref().and_then(|state| state.running) {
            Some(true) => SandboxState::Running,
            Some(false) => SandboxState::Stopped,
            None => SandboxState::Failed("stato sconosciuto".into()),
        };

        let created_at = info
            .created
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Ok(SandboxStatus {
            sandbox_id: handle.to_string(),
            state,
            created_at,
            expires_at: created_at,
            backend_id: "docker".into(),
        })
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        self.abort_proxy_task(handle);
        if let Some(legacy_name) = Self::legacy_container_name(handle) {
            self.abort_proxy_task(&legacy_name);
        }
        self.remove_container_with_fallback(handle).await
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        self.client
            .ping()
            .await
            .map(|_| ())
            .map_err(|e| BackendError::Unavailable(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsandbox_sdk::ir::EgressIR;

    #[test]
    fn network_mode_defaults_to_none_when_deny_all() {
        let ir = SandboxIR::default();
        assert_eq!(DockerBackend::network_mode_for(&ir), "none");
    }

    #[test]
    fn network_mode_uses_bridge_when_passthrough() {
        let mut ir = SandboxIR::default();
        ir.egress.mode = EgressMode::Passthrough;
        assert_eq!(DockerBackend::network_mode_for(&ir), "bridge");
    }

    #[test]
    fn network_mode_uses_bridge_for_proxy_allowlist() {
        let ir = SandboxIR {
            egress: EgressIR {
                mode: EgressMode::Proxy,
                allow_hostnames: vec!["pypi.org".into()],
                allow_ips: Vec::new(),
                deny_by_default: true,
            },
            ..SandboxIR::default()
        };
        assert_eq!(DockerBackend::network_mode_for(&ir), "bridge");
        assert!(DockerBackend::should_apply_egress_rules(&ir));
    }

    #[test]
    fn configure_proxy_env_injects_proxy_variables_and_host_gateway() {
        let mut env = Vec::new();
        let mut host_config = HostConfig::default();

        DockerBackend::configure_proxy_env(&mut env, &mut host_config, 43123);

        assert!(env.contains(&"HTTP_PROXY=http://host.docker.internal:43123".to_string()));
        assert!(env.contains(&"HTTPS_PROXY=http://host.docker.internal:43123".to_string()));
        assert!(env.contains(&"ALL_PROXY=socks5h://host.docker.internal:43123".to_string()));
        assert!(env.contains(&"NO_PROXY=127.0.0.1,localhost".to_string()));
        assert_eq!(
            host_config.extra_hosts,
            Some(vec!["host.docker.internal:host-gateway".to_string()])
        );
    }

    #[tokio::test]
    async fn abort_proxy_task_unregisters_running_proxy() {
        let client =
            Docker::connect_with_unix("/var/run/docker.sock", 30, bollard::API_DEFAULT_VERSION)
                .unwrap();
        let backend = DockerBackend::with_client(client);
        let proxy = EgressProxy::start("sandbox-1".into(), vec!["localhost".into()])
            .await
            .unwrap();

        backend.register_proxy_task("handle-1".into(), proxy);
        assert_eq!(backend.proxy_tasks.lock().unwrap().len(), 1);

        backend.abort_proxy_task("handle-1");
        assert!(backend.proxy_tasks.lock().unwrap().is_empty());
    }

    #[test]
    fn legacy_container_name_only_applies_to_old_sandbox_ids() {
        assert_eq!(
            DockerBackend::legacy_container_name("sandbox-123").as_deref(),
            Some("agentsandbox-sandbox-123")
        );
        assert!(DockerBackend::legacy_container_name("agentsandbox-sandbox-123").is_none());
    }
}
