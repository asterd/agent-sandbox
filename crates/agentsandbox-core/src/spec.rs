//! Public sandbox specification (`sandbox.ai/v1alpha1`).
//!
//! This module is the public contract between agents/SDKs and the daemon.
//! Anything not expressed here is not part of the API and should not be relied
//! upon. Adding or removing a field is a spec-version change.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The current supported API version string.
pub const API_VERSION_V1ALPHA1: &str = "sandbox.ai/v1alpha1";

/// Root document: a sandbox specification in YAML or JSON form.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSpec {
    /// API version string, e.g. `"sandbox.ai/v1alpha1"`.
    pub api_version: String,
    /// Kind, always `"Sandbox"` in v1alpha1.
    pub kind: String,
    #[serde(default)]
    pub metadata: Metadata,
    pub spec: SandboxSpecBody,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub name: Option<String>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSpecBody {
    pub runtime: RuntimeSpec,
    pub resources: Option<ResourceSpec>,
    pub network: Option<NetworkSpec>,
    pub secrets: Option<Vec<SecretRef>>,
    pub ttl_seconds: Option<u64>,
    pub scheduling: Option<SchedulingSpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSpec {
    /// Explicit docker image. When set, overrides `preset`.
    pub image: Option<String>,
    pub preset: Option<RuntimePreset>,
    /// Non-secret environment variables forwarded to the guest.
    pub env: Option<HashMap<String, String>>,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuntimePreset {
    /// `python:3.12-slim`
    Python,
    /// `node:20-slim`
    Node,
    /// `rust:1.77-slim`
    Rust,
    /// `ubuntu:24.04`
    Shell,
    /// Requires an explicit `runtime.image`.
    Custom,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpec {
    /// CPU budget in millicores. Default: 1000 (= 1 CPU).
    pub cpu_millicores: Option<u32>,
    /// Memory limit in MiB. Default: 512.
    pub memory_mb: Option<u32>,
    /// Ephemeral disk limit in MiB. Default: 1024.
    pub disk_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSpec {
    pub egress: EgressPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressPolicy {
    /// Allowed egress hostnames (no wildcards, no IPs).
    #[serde(default)]
    pub allow: Vec<String>,
    /// Deny all egress not in `allow`. Defaults to `true`.
    #[serde(default = "default_true")]
    pub deny_by_default: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Guest-side env var name the secret is bound to.
    pub name: String,
    pub value_from: SecretSource,
}

/// Where the secret value is sourced from on the host.
///
/// Secret values are resolved by the daemon and never appear in the public
/// spec payload or in any log output.
///
/// Represented as a map with exactly one key (`envRef` or `file`):
///
/// ```yaml
/// valueFrom:
///   envRef: MY_HOST_ENV
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretSource {
    /// Read the secret from a host environment variable.
    pub env_ref: Option<String>,
    /// Read the secret from a file on the host.
    pub file: Option<String>,
}

impl SecretSource {
    /// Build an `envRef` source.
    pub fn env_ref(name: impl Into<String>) -> Self {
        Self {
            env_ref: Some(name.into()),
            file: None,
        }
    }

    /// Build a `file` source.
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            env_ref: None,
            file: Some(path.into()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulingSpec {
    /// Prefer reusing a warm sandbox pool. Ignored in v1alpha1.
    #[serde(default)]
    pub prefer_warm: bool,
}
