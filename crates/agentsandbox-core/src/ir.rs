//! Internal intermediate representation consumed by backend adapters.
//!
//! The IR is produced by [`crate::compile::compile`] from a public spec and
//! contains resolved values (no presets, no host-env lookups, no wildcards).
//! Backend adapters depend only on this type.

use crate::spec::{AuditLevel, EgressMode, SchedulingPriority};
use std::fmt;

/// Resolved, backend-agnostic sandbox definition.
///
/// `secret_env` intentionally carries already-resolved secret values; the
/// manual [`fmt::Debug`] impl redacts them so that `tracing::debug!` on an
/// `SandboxIR` never leaks a secret.
#[derive(Clone)]
pub struct SandboxIR {
    /// Opaque sandbox id (uuid v4) assigned at compile time.
    pub id: String,
    /// Fully resolved Docker image reference. Never a preset.
    pub image: String,
    /// Optional CMD override. `None` means the adapter chooses a keep-alive.
    pub command: Option<Vec<String>>,
    /// Non-secret environment variables, in insertion order.
    pub env: Vec<(String, String)>,
    /// Secret environment variables, resolved values. Redacted in Debug.
    pub secret_env: Vec<(String, String)>,
    pub cpu_millicores: u32,
    pub memory_mb: u32,
    pub disk_mb: u32,
    pub egress_allow: Vec<String>,
    pub deny_by_default: bool,
    pub egress_mode: Option<EgressMode>,
    pub ttl_seconds: u64,
    pub exec_timeout_ms: Option<u64>,
    pub working_dir: String,
    pub runtime_version: Option<String>,
    pub backend_hint: Option<String>,
    pub prefer_warm: bool,
    pub priority: Option<SchedulingPriority>,
    pub storage_volumes: Vec<serde_json::Value>,
    pub audit_level: Option<AuditLevel>,
    pub metrics_enabled: bool,
}

impl Default for SandboxIR {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            image: "python:3.12-slim".into(),
            command: None,
            env: Vec::new(),
            secret_env: Vec::new(),
            cpu_millicores: 1000,
            memory_mb: 512,
            disk_mb: 1024,
            egress_allow: Vec::new(),
            deny_by_default: true,
            egress_mode: None,
            ttl_seconds: 300,
            exec_timeout_ms: None,
            working_dir: "/workspace".into(),
            runtime_version: None,
            backend_hint: None,
            prefer_warm: false,
            priority: None,
            storage_volumes: Vec::new(),
            audit_level: None,
            metrics_enabled: false,
        }
    }
}

impl fmt::Debug for SandboxIR {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_secrets: Vec<(&str, &str)> = self
            .secret_env
            .iter()
            .map(|(k, _)| (k.as_str(), "<redacted>"))
            .collect();

        f.debug_struct("SandboxIR")
            .field("id", &self.id)
            .field("image", &self.image)
            .field("command", &self.command)
            .field("env", &self.env)
            .field("secret_env", &redacted_secrets)
            .field("cpu_millicores", &self.cpu_millicores)
            .field("memory_mb", &self.memory_mb)
            .field("disk_mb", &self.disk_mb)
            .field("egress_allow", &self.egress_allow)
            .field("deny_by_default", &self.deny_by_default)
            .field("egress_mode", &self.egress_mode)
            .field("ttl_seconds", &self.ttl_seconds)
            .field("exec_timeout_ms", &self.exec_timeout_ms)
            .field("working_dir", &self.working_dir)
            .field("runtime_version", &self.runtime_version)
            .field("backend_hint", &self.backend_hint)
            .field("prefer_warm", &self.prefer_warm)
            .field("priority", &self.priority)
            .field("storage_volumes", &self.storage_volumes)
            .field("audit_level", &self.audit_level)
            .field("metrics_enabled", &self.metrics_enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ir_has_safe_defaults() {
        let ir = SandboxIR::default();
        assert_eq!(ir.image, "python:3.12-slim");
        assert_eq!(ir.cpu_millicores, 1000);
        assert_eq!(ir.memory_mb, 512);
        assert_eq!(ir.disk_mb, 1024);
        assert_eq!(ir.ttl_seconds, 300);
        assert!(ir.deny_by_default);
        assert_eq!(ir.working_dir, "/workspace");
        assert!(ir.egress_allow.is_empty());
        assert!(ir.env.is_empty());
        assert!(ir.secret_env.is_empty());
        assert!(ir.egress_mode.is_none());
        assert!(ir.exec_timeout_ms.is_none());
        assert!(ir.runtime_version.is_none());
        assert!(ir.backend_hint.is_none());
        assert!(!ir.prefer_warm);
        assert!(ir.priority.is_none());
        assert!(ir.storage_volumes.is_empty());
        assert!(ir.audit_level.is_none());
        assert!(!ir.metrics_enabled);
        assert!(!ir.id.is_empty());
    }

    #[test]
    fn debug_redacts_secret_env_values() {
        let mut ir = SandboxIR::default();
        ir.secret_env
            .push(("API_KEY".into(), "super-secret-value".into()));
        let rendered = format!("{:?}", ir);
        assert!(rendered.contains("API_KEY"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("super-secret-value"));
    }

    #[test]
    fn debug_keeps_non_secret_env_visible() {
        let mut ir = SandboxIR::default();
        ir.env.push(("LOG_LEVEL".into(), "debug".into()));
        let rendered = format!("{:?}", ir);
        assert!(rendered.contains("LOG_LEVEL"));
        assert!(rendered.contains("debug"));
    }
}
