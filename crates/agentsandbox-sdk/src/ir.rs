use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SchedulingPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuditLevel {
    None,
    Basic,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EgressMode {
    None,
    Proxy,
    Passthrough,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EgressIR {
    pub mode: EgressMode,
    pub allow_hostnames: Vec<String>,
    pub allow_ips: Vec<String>,
    pub deny_by_default: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SandboxIR {
    pub id: String,
    pub image: String,
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    #[serde(skip_serializing)]
    pub secret_env: Vec<(String, String)>,
    pub cpu_millicores: u32,
    pub memory_mb: u32,
    pub disk_mb: u32,
    pub egress: EgressIR,
    pub ttl_seconds: u64,
    pub timeout_ms: u64,
    pub working_dir: String,
    pub labels: HashMap<String, String>,
    pub backend_hint: Option<String>,
    pub extensions: Option<serde_json::Value>,
    pub runtime_version: Option<String>,
    pub prefer_warm: bool,
    pub priority: Option<SchedulingPriority>,
    pub storage_volumes: Vec<serde_json::Value>,
    pub audit_level: Option<AuditLevel>,
    pub metrics_enabled: bool,
}

impl SandboxIR {
    pub fn default_for_test() -> Self {
        Self {
            image: "python:3.12-slim".into(),
            ..Self::default()
        }
    }
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
            egress: EgressIR {
                mode: EgressMode::Proxy,
                allow_hostnames: Vec::new(),
                allow_ips: Vec::new(),
                deny_by_default: true,
            },
            ttl_seconds: 300,
            timeout_ms: 30_000,
            working_dir: "/workspace".into(),
            labels: HashMap::new(),
            backend_hint: None,
            extensions: None,
            runtime_version: None,
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
            .field("egress", &self.egress)
            .field("ttl_seconds", &self.ttl_seconds)
            .field("timeout_ms", &self.timeout_ms)
            .field("working_dir", &self.working_dir)
            .field("labels", &self.labels)
            .field("backend_hint", &self.backend_hint)
            .field("extensions", &self.extensions)
            .field("runtime_version", &self.runtime_version)
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
        assert_eq!(ir.timeout_ms, 30_000);
        assert!(ir.egress.deny_by_default);
        assert_eq!(ir.egress.mode, EgressMode::Proxy);
        assert_eq!(ir.working_dir, "/workspace");
        assert!(ir.env.is_empty());
        assert!(ir.secret_env.is_empty());
    }

    #[test]
    fn debug_redacts_secret_env_values() {
        let mut ir = SandboxIR::default();
        ir.secret_env
            .push(("API_KEY".into(), "super-secret-value".into()));
        let rendered = format!("{ir:?}");
        assert!(rendered.contains("API_KEY"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("super-secret-value"));
    }
}
