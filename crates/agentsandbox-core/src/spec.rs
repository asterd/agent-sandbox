//! Public sandbox specifications.
//!
//! `v1alpha1` remains supported as-is. `v1beta1` is additive only and extends
//! the public contract without removing or renaming any `v1alpha1` field.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const API_VERSION_V1ALPHA1: &str = "sandbox.ai/v1alpha1";
pub const API_VERSION_V1BETA1: &str = "sandbox.ai/v1beta1";

pub type SandboxSpec = SpecV1Alpha1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecV1Alpha1 {
    pub api_version: String,
    pub kind: String,
    #[serde(default)]
    pub metadata: Metadata,
    pub spec: SandboxSpecBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpecV1Beta1 {
    pub api_version: String,
    pub kind: String,
    #[serde(default)]
    pub metadata: Metadata,
    pub spec: SandboxSpecBodyV1Beta1,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub name: Option<String>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxSpecBody {
    pub runtime: RuntimeSpec,
    pub resources: Option<ResourceSpec>,
    pub network: Option<NetworkSpec>,
    pub secrets: Option<Vec<SecretRef>>,
    pub ttl_seconds: Option<u64>,
    pub scheduling: Option<SchedulingSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxSpecBodyV1Beta1 {
    pub runtime: RuntimeSpecV1Beta1,
    pub resources: Option<ResourceSpecV1Beta1>,
    pub network: Option<NetworkSpecV1Beta1>,
    pub secrets: Option<Vec<SecretRef>>,
    pub ttl_seconds: Option<u64>,
    pub scheduling: Option<SchedulingSpecV1Beta1>,
    pub storage: Option<StorageSpec>,
    pub observability: Option<ObservabilitySpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSpec {
    pub image: Option<String>,
    pub preset: Option<RuntimePreset>,
    pub env: Option<HashMap<String, String>>,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSpecV1Beta1 {
    pub image: Option<String>,
    pub preset: Option<RuntimePreset>,
    pub version: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuntimePreset {
    Python,
    Node,
    Rust,
    Shell,
    Custom,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceSpec {
    pub cpu_millicores: Option<u32>,
    pub memory_mb: Option<u32>,
    pub disk_mb: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceSpecV1Beta1 {
    pub cpu_millicores: Option<u32>,
    pub memory_mb: Option<u32>,
    pub disk_mb: Option<u32>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSpec {
    pub egress: EgressPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSpecV1Beta1 {
    pub egress: EgressPolicyV1Beta1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressPolicy {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default = "default_true")]
    pub deny_by_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressPolicyV1Beta1 {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default = "default_true")]
    pub deny_by_default: bool,
    pub mode: Option<EgressMode>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EgressMode {
    None,
    Proxy,
    Passthrough,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretRef {
    pub name: String,
    pub value_from: SecretSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretSource {
    pub env_ref: Option<String>,
    pub file: Option<String>,
}

impl SecretSource {
    pub fn env_ref(name: impl Into<String>) -> Self {
        Self {
            env_ref: Some(name.into()),
            file: None,
        }
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self {
            env_ref: None,
            file: Some(path.into()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulingSpec {
    #[serde(default)]
    pub prefer_warm: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulingSpecV1Beta1 {
    pub backend: Option<String>,
    #[serde(default)]
    pub prefer_warm: bool,
    pub priority: Option<SchedulingPriority>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SchedulingPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageSpec {
    #[serde(default)]
    pub volumes: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservabilitySpec {
    pub audit_level: Option<AuditLevel>,
    pub metrics_enabled: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuditLevel {
    None,
    Basic,
    Full,
}
