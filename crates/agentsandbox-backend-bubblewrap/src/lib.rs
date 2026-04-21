#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod factory;

pub use factory::{default_rootfs_base, BubblewrapBackendFactory};

use agentsandbox_sdk::{
    backend::{ExecResult, SandboxBackend, SandboxState, SandboxStatus},
    error::BackendError,
    ir::{EgressMode, SandboxIR},
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
};
use tempfile::NamedTempFile;
use tokio::{process::Command, sync::Mutex, time::timeout};

#[derive(Clone)]
struct SandboxSession {
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    ir: SandboxIR,
    workspace: PathBuf,
}

pub struct BubblewrapBackend {
    bwrap_path: String,
    rootfs_base: PathBuf,
    sessions: Arc<Mutex<HashMap<String, SandboxSession>>>,
}

impl BubblewrapBackend {
    pub fn new(bwrap_path: String, rootfs_base: String) -> Self {
        Self {
            bwrap_path,
            rootfs_base: PathBuf::from(rootfs_base),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn parse_extensions(ir: &SandboxIR) -> Result<BubblewrapExtensions, BackendError> {
        match &ir.extensions {
            None => Ok(BubblewrapExtensions::default()),
            Some(raw) => {
                let section = raw.get("bubblewrap").cloned().unwrap_or_default();
                serde_json::from_value(section).map_err(|error| {
                    BackendError::Configuration(format!(
                        "extensions.bubblewrap non valide: {error}"
                    ))
                })
            }
        }
    }

    async fn session(&self, handle: &str) -> Result<SandboxSession, BackendError> {
        self.sessions
            .lock()
            .await
            .get(handle)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(handle.to_string()))
    }

    fn env_pairs(ir: &SandboxIR) -> impl Iterator<Item = (&str, &str)> {
        ir.env
            .iter()
            .chain(ir.secret_env.iter())
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    fn workspace_dir(&self, id: &str) -> PathBuf {
        self.rootfs_base.join(id)
    }

    #[cfg(target_os = "linux")]
    fn build_linux_command(
        &self,
        session: &SandboxSession,
        command: &str,
        extensions: &BubblewrapExtensions,
    ) -> Result<Command, BackendError> {
        let mut cmd = Command::new(&self.bwrap_path);
        cmd.arg("--die-with-parent")
            .arg("--new-session")
            .arg("--unshare-pid")
            .arg("--unshare-uts")
            .arg("--hostname")
            .arg(format!("sandbox-{}", &session.ir.id[..8]))
            .arg("--proc")
            .arg("/proc")
            .arg("--dev")
            .arg("/dev")
            .arg("--tmpfs")
            .arg("/tmp")
            .arg("--ro-bind")
            .arg("/")
            .arg("/")
            .arg("--bind")
            .arg(&session.workspace)
            .arg(&session.ir.working_dir)
            .arg("--chdir")
            .arg(&session.ir.working_dir);

        if matches!(session.ir.egress.mode, EgressMode::None)
            || (session.ir.egress.mode == EgressMode::Proxy
                && session.ir.egress.deny_by_default
                && session.ir.egress.allow_hostnames.is_empty())
        {
            cmd.arg("--unshare-net");
        }

        for [host, guest] in &extensions.ro_bind {
            cmd.arg("--ro-bind").arg(host).arg(guest);
        }
        for [host, guest] in &extensions.rw_bind {
            cmd.arg("--bind").arg(host).arg(guest);
        }
        for arg in &extensions.extra_args {
            cmd.arg(arg);
        }
        for (key, value) in Self::env_pairs(&session.ir) {
            cmd.arg("--setenv").arg(key).arg(value);
        }
        cmd.arg("--").arg("/bin/sh").arg("-lc").arg(command);
        Ok(cmd)
    }

    #[cfg(target_os = "macos")]
    fn build_macos_command(
        &self,
        session: &SandboxSession,
        command: &str,
        _extensions: &BubblewrapExtensions,
    ) -> Result<(Command, Option<NamedTempFile>), BackendError> {
        let policy = if matches!(session.ir.egress.mode, EgressMode::None)
            || (session.ir.egress.mode == EgressMode::Proxy
                && session.ir.egress.deny_by_default
                && session.ir.egress.allow_hostnames.is_empty())
        {
            "(version 1)\n(allow default)\n(deny network*)\n"
        } else {
            "(version 1)\n(allow default)\n"
        };

        let mut policy_file = NamedTempFile::new()
            .map_err(|error| BackendError::Internal(format!("policy temp file: {error}")))?;
        std::io::Write::write_all(&mut policy_file, policy.as_bytes())
            .map_err(|error| BackendError::Internal(format!("policy write: {error}")))?;

        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-f")
            .arg(policy_file.path())
            .arg("/bin/sh")
            .arg("-lc")
            .arg(command)
            .current_dir(&session.workspace);
        for (key, value) in Self::env_pairs(&session.ir) {
            cmd.env(key, value);
        }
        Ok((cmd, Some(policy_file)))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn unsupported_platform_error() -> BackendError {
        BackendError::Unavailable(
            "bubblewrap e sandbox-exec sono supportati solo su Linux o macOS".into(),
        )
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BubblewrapExtensions {
    #[serde(default)]
    ro_bind: Vec<[String; 2]>,
    #[serde(default)]
    rw_bind: Vec<[String; 2]>,
    #[serde(default)]
    gpu_access: bool,
    #[serde(default)]
    extra_args: Vec<String>,
}

#[async_trait]
impl SandboxBackend for BubblewrapBackend {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let extensions = Self::parse_extensions(ir)?;
        if cfg!(target_os = "macos")
            && (!extensions.ro_bind.is_empty()
                || !extensions.rw_bind.is_empty()
                || extensions.gpu_access
                || !extensions.extra_args.is_empty())
        {
            return Err(BackendError::NotSupported(
                "su macOS bubblewrap supporta solo il profilo base senza mount extra".into(),
            ));
        }

        std::fs::create_dir_all(&self.rootfs_base)
            .map_err(|error| BackendError::Internal(format!("rootfs_base mkdir: {error}")))?;

        let workspace = self.workspace_dir(&ir.id);
        std::fs::create_dir_all(&workspace)
            .map_err(|error| BackendError::Internal(format!("workspace mkdir: {error}")))?;

        let now = Utc::now();
        self.sessions.lock().await.insert(
            ir.id.clone(),
            SandboxSession {
                created_at: now,
                expires_at: now + Duration::seconds(ir.ttl_seconds as i64),
                ir: ir.clone(),
                workspace,
            },
        );
        Ok(ir.id.clone())
    }

    async fn exec(
        &self,
        handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError> {
        let session = self.session(handle).await?;
        let extensions = Self::parse_extensions(&session.ir)?;
        let effective_timeout = timeout_ms.unwrap_or(session.ir.timeout_ms);
        let start = std::time::Instant::now();

        #[cfg(target_os = "linux")]
        let child = self
            .build_linux_command(&session, command, &extensions)?
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| BackendError::Internal(format!("spawn bubblewrap: {error}")))?;

        #[cfg(target_os = "macos")]
        let (child, _guard) = {
            let (mut command_builder, guard) =
                self.build_macos_command(&session, command, &extensions)?;
            (
                command_builder
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| {
                        BackendError::Internal(format!("spawn sandbox-exec: {error}"))
                    })?,
                guard,
            )
        };

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(Self::unsupported_platform_error());

        let output = timeout(
            std::time::Duration::from_millis(effective_timeout),
            child.wait_with_output(),
        )
        .await;

        match output {
            Ok(Ok(result)) => Ok(ExecResult {
                stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
                exit_code: result.status.code().unwrap_or(-1) as i64,
                duration_ms: start.elapsed().as_millis() as u64,
                resource_usage: None,
            }),
            Ok(Err(error)) => Err(BackendError::Internal(error.to_string())),
            Err(_) => Err(BackendError::Timeout(effective_timeout)),
        }
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        let session = self.session(handle).await?;
        Ok(SandboxStatus {
            sandbox_id: handle.to_string(),
            state: SandboxState::Running,
            created_at: session.created_at,
            expires_at: session.expires_at,
            backend_id: "bubblewrap".into(),
        })
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        let session = self.sessions.lock().await.remove(handle);
        if let Some(session) = session {
            let _ = std::fs::remove_dir_all(session.workspace);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        #[cfg(target_os = "linux")]
        {
            let output = Command::new(&self.bwrap_path)
                .arg("--version")
                .output()
                .await
                .map_err(|error| {
                    BackendError::Unavailable(format!(
                        "bwrap non trovato in '{}': {error}",
                        self.bwrap_path
                    ))
                })?;
            if !output.status.success() {
                return Err(BackendError::Unavailable(
                    "bwrap --version ha fallito".into(),
                ));
            }

            let userns_enabled = std::fs::read_to_string("/proc/sys/kernel/unprivileged_userns_clone")
                .map(|value| value.trim() != "0")
                .unwrap_or(true);
            if !userns_enabled {
                return Err(BackendError::Unavailable(
                    "user namespaces disabilitati: abilita kernel.unprivileged_userns_clone=1"
                        .into(),
                ));
            }
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        {
            let output = Command::new("sandbox-exec")
                .arg("-p")
                .arg("(version 1) (allow default)")
                .arg("/usr/bin/true")
                .output()
                .await
                .map_err(|error| {
                    BackendError::Unavailable(format!("sandbox-exec non disponibile: {error}"))
                })?;
            if output.status.success() {
                return Ok(());
            }
            return Err(BackendError::Unavailable(
                "sandbox-exec test fallito".into(),
            ));
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        Err(Self::unsupported_platform_error())
    }

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        let _ = Self::parse_extensions(ir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn destroy_removes_workspace() {
        let backend = BubblewrapBackend::new("bwrap".into(), default_rootfs_base());
        let ir = SandboxIR::default_for_test();
        let handle = backend.create(&ir).await.unwrap();
        let workspace = backend.workspace_dir(&handle);
        assert!(workspace.exists());
        backend.destroy(&handle).await.unwrap();
        assert!(!workspace.exists());
    }

    #[test]
    fn invalid_extensions_are_rejected() {
        let mut ir = SandboxIR::default_for_test();
        ir.extensions = Some(serde_json::json!({
            "bubblewrap": {
                "unknown": true
            }
        }));
        let error = BubblewrapBackend::parse_extensions(&ir).unwrap_err();
        assert!(error.to_string().contains("extensions.bubblewrap"));
    }
}
