//! Internal intermediate representation consumed by backend adapters.
//!
//! The IR is produced by [`crate::compile::compile`] from a public spec and
//! contains resolved values (no presets, no host-env lookups, no wildcards).
//! Backend adapters depend only on this type.

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
    pub ttl_seconds: u64,
    pub working_dir: String,
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
            ttl_seconds: 300,
            working_dir: "/workspace".into(),
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
            .field("ttl_seconds", &self.ttl_seconds)
            .field("working_dir", &self.working_dir)
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
