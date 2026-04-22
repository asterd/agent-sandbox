#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use agentsandbox_sdk::{
    backend::{
        BackendCapabilities, BackendDescriptor, BackendFactory, ExecResult, IsolationLevel,
        SandboxBackend, SandboxState, SandboxStatus,
    },
    error::BackendError,
    ir::SandboxIR,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{process::Command, sync::Mutex, time::timeout};

#[derive(Clone)]
struct SandboxSession {
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    ir: SandboxIR,
    workspace: PathBuf,
}

pub struct NsjailBackendFactory;

pub struct NsjailBackend {
    nsjail_path: String,
    chroot_base: PathBuf,
    sessions: Arc<Mutex<HashMap<String, SandboxSession>>>,
}

impl NsjailBackend {
    pub fn new(nsjail_path: String, chroot_base: String) -> Self {
        Self {
            nsjail_path,
            chroot_base: PathBuf::from(chroot_base),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn parse_extensions(ir: &SandboxIR) -> Result<NsjailExtensions, BackendError> {
        match &ir.extensions {
            None => Ok(NsjailExtensions::default()),
            Some(raw) => {
                let section = raw.get("nsjail").cloned().unwrap_or_default();
                serde_json::from_value(section).map_err(|error| {
                    BackendError::Configuration(format!("extensions.nsjail non valide: {error}"))
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

    fn workspace_dir(&self, id: &str) -> PathBuf {
        self.chroot_base.join(id)
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.chroot_base.join(".snapshots")
    }

    fn resolve_guest_path(workspace: &Path, path: &str) -> Result<PathBuf, BackendError> {
        let mut relative = PathBuf::new();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(part) => relative.push(part),
                Component::CurDir | Component::RootDir => {}
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(BackendError::Configuration(
                        "path file non valido o traversal non consentito".into(),
                    ))
                }
            }
        }
        if relative.as_os_str().is_empty() {
            return Err(BackendError::Configuration("path file vuoto".into()));
        }
        Ok(workspace.join(relative))
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), BackendError> {
        std::fs::create_dir_all(dst)
            .map_err(|error| BackendError::Internal(format!("mkdir snapshot: {error}")))?;
        for entry in std::fs::read_dir(src)
            .map_err(|error| BackendError::Internal(format!("read_dir snapshot: {error}")))?
        {
            let entry = entry
                .map_err(|error| BackendError::Internal(format!("read_dir entry: {error}")))?;
            let file_type = entry
                .file_type()
                .map_err(|error| BackendError::Internal(format!("file_type: {error}")))?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if file_type.is_dir() {
                Self::copy_dir_recursive(&src_path, &dst_path)?;
            } else if file_type.is_file() {
                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        BackendError::Internal(format!("mkdir parent: {error}"))
                    })?;
                }
                std::fs::copy(&src_path, &dst_path)
                    .map_err(|error| BackendError::Internal(format!("copy file: {error}")))?;
            }
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn build_command(
        &self,
        session: &SandboxSession,
        command: &str,
        extensions: &NsjailExtensions,
    ) -> Command {
        let mut cmd = Command::new(&self.nsjail_path);
        cmd.arg("--quiet")
            .arg("--mode")
            .arg("o")
            .arg("--cwd")
            .arg(&session.workspace)
            .arg("--time_limit")
            .arg(session.ir.ttl_seconds.to_string())
            .arg("--rlimit_as")
            .arg((session.ir.memory_mb * 2).to_string())
            .arg("--rlimit_cpu")
            .arg(((session.ir.cpu_millicores / 1000).max(1)).to_string());

        if session.ir.egress.mode == agentsandbox_sdk::ir::EgressMode::None {
            cmd.arg("--disable_clone_newnet");
        }
        if let Some(value) = extensions.rlimit_nofile {
            cmd.arg("--rlimit_nofile").arg(value.to_string());
        }
        if let Some(value) = extensions.rlimit_nproc {
            cmd.arg("--rlimit_nproc").arg(value.to_string());
        }
        if let Some(value) = extensions.cgroup_mem_max {
            cmd.arg("--cgroup_mem_max").arg(value.to_string());
        }
        for bind in &extensions.bindmount_ro {
            cmd.arg("--bindmount_ro").arg(bind);
        }
        cmd.arg("--").arg("/bin/sh").arg("-lc").arg(command);
        cmd
    }

    #[cfg(not(target_os = "linux"))]
    fn unsupported_error() -> BackendError {
        BackendError::Unavailable(
            "nsjail e' supportato solo su Linux con namespace/cgroup disponibili".into(),
        )
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NsjailExtensions {
    seccomp_policy: Option<String>,
    rlimit_nofile: Option<u64>,
    rlimit_nproc: Option<u64>,
    #[serde(default)]
    bindmount_ro: Vec<String>,
    cgroup_mem_max: Option<u64>,
}

impl BackendFactory for NsjailBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "nsjail",
            display_name: "nsjail (Google)",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: false,
                isolation_level: IsolationLevel::Process,
                supported_presets: vec!["python", "node", "rust", "shell"],
                rootless: false,
                snapshot_restore: true,
            },
            extensions_schema: Some(include_str!("../schema/extensions.schema.json")),
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let nsjail_path = config
            .get("nsjail_path")
            .cloned()
            .unwrap_or_else(|| "nsjail".into());
        let chroot_base = config.get("chroot_base").cloned().unwrap_or_else(|| {
            std::env::temp_dir()
                .join("agentsandbox-nsjail")
                .to_string_lossy()
                .into_owned()
        });
        Ok(Box::new(NsjailBackend::new(nsjail_path, chroot_base)))
    }
}

#[async_trait]
impl SandboxBackend for NsjailBackend {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let _ = Self::parse_extensions(ir)?;
        std::fs::create_dir_all(&self.chroot_base)
            .map_err(|error| BackendError::Internal(format!("chroot base mkdir: {error}")))?;
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
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (handle, command, timeout_ms);
            return Err(Self::unsupported_error());
        }

        #[cfg(target_os = "linux")]
        {
            let session = self.session(handle).await?;
            let extensions = Self::parse_extensions(&session.ir)?;
            let effective_timeout = timeout_ms.unwrap_or(session.ir.timeout_ms);
            let start = std::time::Instant::now();
            let child = self
                .build_command(&session, command, &extensions)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .envs(session.ir.env.iter().map(|(k, v)| (k, v)))
                .envs(session.ir.secret_env.iter().map(|(k, v)| (k, v)))
                .spawn()
                .map_err(|error| BackendError::Internal(format!("spawn nsjail: {error}")))?;

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
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        let session = self.session(handle).await?;
        Ok(SandboxStatus {
            sandbox_id: handle.to_string(),
            state: SandboxState::Running,
            created_at: session.created_at,
            expires_at: session.expires_at,
            backend_id: "nsjail".into(),
        })
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        if let Some(session) = self.sessions.lock().await.remove(handle) {
            let _ = std::fs::remove_dir_all(session.workspace);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        #[cfg(not(target_os = "linux"))]
        {
            return Err(Self::unsupported_error());
        }

        #[cfg(target_os = "linux")]
        {
            let output = Command::new(&self.nsjail_path)
                .arg("--help")
                .output()
                .await
                .map_err(|error| {
                    BackendError::Unavailable(format!(
                        "nsjail non trovato in '{}': {error}",
                        self.nsjail_path
                    ))
                })?;
            if output.stdout.is_empty() && output.stderr.is_empty() {
                return Err(BackendError::Unavailable("nsjail non risponde".into()));
            }
            Ok(())
        }
    }

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        let _ = Self::parse_extensions(ir)?;
        Ok(())
    }

    async fn upload_file(
        &self,
        handle: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), BackendError> {
        let session = self.session(handle).await?;
        let guest_path = Self::resolve_guest_path(&session.workspace, path)?;
        if let Some(parent) = guest_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| BackendError::Internal(format!("mkdir upload parent: {error}")))?;
        }
        std::fs::write(&guest_path, content)
            .map_err(|error| BackendError::Internal(format!("write upload file: {error}")))?;
        Ok(())
    }

    async fn download_file(&self, handle: &str, path: &str) -> Result<Vec<u8>, BackendError> {
        let session = self.session(handle).await?;
        let guest_path = Self::resolve_guest_path(&session.workspace, path)?;
        std::fs::read(&guest_path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => BackendError::NotFound(path.into()),
            _ => BackendError::Internal(format!("read download file: {error}")),
        })
    }

    async fn snapshot(&self, handle: &str) -> Result<String, BackendError> {
        let session = self.session(handle).await?;
        let snapshot_id = format!(
            "nsjail-{}-{}",
            handle,
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let snapshot_path = self.snapshots_dir().join(&snapshot_id);
        Self::copy_dir_recursive(&session.workspace, &snapshot_path)?;
        Ok(snapshot_id)
    }

    async fn restore(&self, snapshot_id: &str, ir: &SandboxIR) -> Result<String, BackendError> {
        let _ = Self::parse_extensions(ir)?;
        std::fs::create_dir_all(&self.chroot_base)
            .map_err(|error| BackendError::Internal(format!("chroot base mkdir: {error}")))?;
        let snapshot_path = self.snapshots_dir().join(snapshot_id);
        if !snapshot_path.exists() {
            return Err(BackendError::NotFound(snapshot_id.into()));
        }
        let workspace = self.workspace_dir(&ir.id);
        if workspace.exists() {
            let _ = std::fs::remove_dir_all(&workspace);
        }
        Self::copy_dir_recursive(&snapshot_path, &workspace)?;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_reports_capabilities() {
        let descriptor = NsjailBackendFactory.describe();
        assert_eq!(descriptor.id, "nsjail");
        assert!(descriptor.capabilities.memory_hard_limit);
        assert!(descriptor.capabilities.cpu_hard_limit);
    }

    #[tokio::test]
    async fn destroy_removes_workspace() {
        let backend = NsjailBackend::new(
            "nsjail".into(),
            std::env::temp_dir()
                .join("agentsandbox-nsjail-test")
                .to_string_lossy()
                .into_owned(),
        );
        let ir = SandboxIR::default_for_test();
        let handle = backend.create(&ir).await.unwrap();
        let workspace = backend.workspace_dir(&handle);
        assert!(workspace.exists());
        backend.destroy(&handle).await.unwrap();
        assert!(!workspace.exists());
    }

    #[tokio::test]
    async fn upload_download_and_snapshot_roundtrip() {
        let backend = NsjailBackend::new(
            "nsjail".into(),
            std::env::temp_dir()
                .join("agentsandbox-nsjail-test-capabilities")
                .to_string_lossy()
                .into_owned(),
        );
        let ir = SandboxIR::default_for_test();
        let handle = backend.create(&ir).await.unwrap();
        backend
            .upload_file(&handle, "nested/input.txt", b"hello")
            .await
            .unwrap();
        let content = backend
            .download_file(&handle, "nested/input.txt")
            .await
            .unwrap();
        assert_eq!(content, b"hello");

        let snapshot_id = backend.snapshot(&handle).await.unwrap();
        backend.destroy(&handle).await.unwrap();

        let restored_ir = SandboxIR {
            id: "restored-nsjail".into(),
            ..SandboxIR::default_for_test()
        };
        let restored = backend.restore(&snapshot_id, &restored_ir).await.unwrap();
        let restored_content = backend
            .download_file(&restored, "nested/input.txt")
            .await
            .unwrap();
        assert_eq!(restored_content, b"hello");
        backend.destroy(&restored).await.unwrap();
    }
}
