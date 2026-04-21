//! Spec -> IR compile pipeline.

use crate::{
    ir::SandboxIR,
    schema::validate_raw,
    spec::{
        NetworkSpec, NetworkSpecV1Beta1, ResourceSpec, ResourceSpecV1Beta1, RuntimePreset,
        SandboxSpec, SecretRef, SpecV1Beta1, API_VERSION_V1ALPHA1, API_VERSION_V1BETA1,
    },
};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecVersion {
    V1Alpha1,
    V1Beta1,
}

impl SpecVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecVersion::V1Alpha1 => API_VERSION_V1ALPHA1,
            SpecVersion::V1Beta1 => API_VERSION_V1BETA1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

impl ValidationIssue {
    fn from_schema_path(path: String, message: String) -> Self {
        Self { path, message }
    }
}

impl<'a> From<jsonschema::ValidationError<'a>> for ValidationIssue {
    fn from(value: jsonschema::ValidationError<'a>) -> Self {
        let raw_path = value.instance_path.to_string();
        let path = if raw_path.is_empty() {
            "/".to_string()
        } else if raw_path.starts_with('/') {
            raw_path
        } else {
            format!("/{}", raw_path)
        };
        Self::from_schema_path(path, value.to_string())
    }
}

#[derive(Error, Debug)]
pub enum CompileError {
    #[error("apiVersion mancante")]
    MissingApiVersion,
    #[error("apiVersion non supportata: {0}")]
    UnsupportedApiVersion(String),
    #[error("payload non valido: {0}")]
    ParseError(String),
    #[error("spec {version:?} non valida")]
    SchemaValidation {
        version: SpecVersion,
        issues: Vec<ValidationIssue>,
    },
    #[error("kind non supportato: {0}")]
    UnsupportedKind(String),
    #[error("runtime.preset e runtime.image non possono essere entrambi assenti")]
    MissingRuntime,
    #[error("runtime.preset=Custom richiede runtime.image esplicita")]
    CustomPresetNeedsImage,
    #[error("secret {0} non trovato nell'ambiente host")]
    SecretNotFound(String),
    #[error("secret {name}: valueFrom deve contenere esattamente uno tra envRef o file")]
    InvalidSecretSource { name: String },
    #[error("egress allow contiene hostname non valido: {0}")]
    InvalidHostname(String),
}

pub fn detect_version(raw: &Value) -> Result<SpecVersion, CompileError> {
    match raw.get("apiVersion").and_then(Value::as_str) {
        Some(API_VERSION_V1ALPHA1) => Ok(SpecVersion::V1Alpha1),
        Some(API_VERSION_V1BETA1) => Ok(SpecVersion::V1Beta1),
        Some(v) => Err(CompileError::UnsupportedApiVersion(v.to_string())),
        None => Err(CompileError::MissingApiVersion),
    }
}

pub fn compile_any(raw: &str) -> Result<SandboxIR, CompileError> {
    let value: Value = serde_json::from_str(raw)
        .or_else(|_| serde_yaml::from_str::<Value>(raw))
        .map_err(|e| CompileError::ParseError(e.to_string()))?;
    compile_value(value)
}

pub fn compile_value(value: Value) -> Result<SandboxIR, CompileError> {
    let version = detect_version(&value)?;
    validate_raw(version, &value)?;

    match version {
        SpecVersion::V1Alpha1 => {
            let spec: SandboxSpec = serde_json::from_value(value)
                .map_err(|e| CompileError::ParseError(e.to_string()))?;
            compile_v1alpha1(spec)
        }
        SpecVersion::V1Beta1 => {
            let spec: SpecV1Beta1 = serde_json::from_value(value)
                .map_err(|e| CompileError::ParseError(e.to_string()))?;
            compile_v1beta1(spec)
        }
    }
}

pub fn compile(spec: SandboxSpec) -> Result<SandboxIR, CompileError> {
    compile_v1alpha1(spec)
}

fn compile_v1alpha1(spec: SandboxSpec) -> Result<SandboxIR, CompileError> {
    if spec.api_version != API_VERSION_V1ALPHA1 {
        return Err(CompileError::UnsupportedApiVersion(spec.api_version));
    }
    if spec.kind != "Sandbox" {
        return Err(CompileError::UnsupportedKind(spec.kind));
    }

    let body = spec.spec;
    let mut ir = SandboxIR {
        image: resolve_image(&body.runtime.image, body.runtime.preset, None)?,
        ..SandboxIR::default()
    };

    if let Some(wd) = body.runtime.working_dir {
        ir.working_dir = wd;
    }

    apply_resources(&mut ir, body.resources);
    apply_network_alpha(&mut ir, body.network)?;
    apply_secrets(&mut ir, body.secrets)?;
    apply_runtime_env(&mut ir, body.runtime.env);

    if let Some(ttl) = body.ttl_seconds {
        ir.ttl_seconds = ttl;
    }
    if let Some(scheduling) = body.scheduling {
        ir.prefer_warm = scheduling.prefer_warm;
    }

    Ok(ir)
}

fn compile_v1beta1(spec: SpecV1Beta1) -> Result<SandboxIR, CompileError> {
    if spec.api_version != API_VERSION_V1BETA1 {
        return Err(CompileError::UnsupportedApiVersion(spec.api_version));
    }
    if spec.kind != "Sandbox" {
        return Err(CompileError::UnsupportedKind(spec.kind));
    }

    let body = spec.spec;
    let mut ir = SandboxIR {
        image: resolve_image(
            &body.runtime.image,
            body.runtime.preset,
            body.runtime.version.as_deref(),
        )?,
        runtime_version: body.runtime.version,
        ..SandboxIR::default()
    };

    if let Some(wd) = body.runtime.working_dir {
        ir.working_dir = wd;
    }

    apply_resources_v1beta1(&mut ir, body.resources);
    apply_network_v1beta1(&mut ir, body.network)?;
    apply_secrets(&mut ir, body.secrets)?;
    apply_runtime_env(&mut ir, body.runtime.env);

    if let Some(ttl) = body.ttl_seconds {
        ir.ttl_seconds = ttl;
    }
    if let Some(scheduling) = body.scheduling {
        ir.backend_hint = scheduling.backend;
        ir.prefer_warm = scheduling.prefer_warm;
        ir.priority = scheduling.priority;
    }
    if let Some(storage) = body.storage {
        ir.storage_volumes = storage.volumes;
    }
    if let Some(observability) = body.observability {
        ir.audit_level = observability.audit_level;
        ir.metrics_enabled = observability.metrics_enabled.unwrap_or(false);
    }

    Ok(ir)
}

fn apply_resources(ir: &mut SandboxIR, resources: Option<ResourceSpec>) {
    if let Some(res) = resources {
        if let Some(v) = res.cpu_millicores {
            ir.cpu_millicores = v;
        }
        if let Some(v) = res.memory_mb {
            ir.memory_mb = v;
        }
        if let Some(v) = res.disk_mb {
            ir.disk_mb = v;
        }
    }
}

fn apply_resources_v1beta1(ir: &mut SandboxIR, resources: Option<ResourceSpecV1Beta1>) {
    if let Some(res) = resources {
        if let Some(v) = res.cpu_millicores {
            ir.cpu_millicores = v;
        }
        if let Some(v) = res.memory_mb {
            ir.memory_mb = v;
        }
        if let Some(v) = res.disk_mb {
            ir.disk_mb = v;
        }
        ir.exec_timeout_ms = res.timeout_ms;
    }
}

fn apply_network_alpha(
    ir: &mut SandboxIR,
    network: Option<NetworkSpec>,
) -> Result<(), CompileError> {
    if let Some(net) = network {
        for host in &net.egress.allow {
            validate_hostname(host)?;
        }
        ir.egress_allow = net.egress.allow;
        ir.deny_by_default = net.egress.deny_by_default;
    }
    Ok(())
}

fn apply_network_v1beta1(
    ir: &mut SandboxIR,
    network: Option<NetworkSpecV1Beta1>,
) -> Result<(), CompileError> {
    if let Some(net) = network {
        for host in &net.egress.allow {
            validate_hostname(host)?;
        }
        ir.egress_allow = net.egress.allow;
        ir.deny_by_default = net.egress.deny_by_default;
        ir.egress_mode = net.egress.mode;
    }
    Ok(())
}

fn apply_runtime_env(ir: &mut SandboxIR, env: Option<std::collections::HashMap<String, String>>) {
    if let Some(env) = env {
        ir.env = env.into_iter().collect();
        ir.env.sort_by(|a, b| a.0.cmp(&b.0));
    }
}

fn apply_secrets(ir: &mut SandboxIR, secrets: Option<Vec<SecretRef>>) -> Result<(), CompileError> {
    if let Some(secrets) = secrets {
        for secret in &secrets {
            let value = resolve_secret(secret)?;
            ir.secret_env.push((secret.name.clone(), value));
        }
    }
    Ok(())
}

fn resolve_image(
    image: &Option<String>,
    preset: Option<RuntimePreset>,
    version: Option<&str>,
) -> Result<String, CompileError> {
    if let Some(image) = image {
        return Ok(image.clone());
    }

    match (preset, version) {
        (Some(RuntimePreset::Python), Some(version)) => Ok(format!("python:{version}-slim")),
        (Some(RuntimePreset::Node), Some(version)) => Ok(format!("node:{version}-slim")),
        (Some(RuntimePreset::Rust), Some(version)) => Ok(format!("rust:{version}-slim")),
        (Some(RuntimePreset::Shell), Some(version)) => Ok(format!("ubuntu:{version}")),
        (Some(RuntimePreset::Python), None) => Ok("python:3.12-slim".into()),
        (Some(RuntimePreset::Node), None) => Ok("node:20-slim".into()),
        (Some(RuntimePreset::Rust), None) => Ok("rust:1.77-slim".into()),
        (Some(RuntimePreset::Shell), None) => Ok("ubuntu:24.04".into()),
        (Some(RuntimePreset::Custom), _) => Err(CompileError::CustomPresetNeedsImage),
        (None, _) => Err(CompileError::MissingRuntime),
    }
}

fn resolve_secret(secret: &SecretRef) -> Result<String, CompileError> {
    match (&secret.value_from.env_ref, &secret.value_from.file) {
        (Some(name), None) => {
            std::env::var(name).map_err(|_| CompileError::SecretNotFound(name.clone()))
        }
        (None, Some(path)) => std::fs::read_to_string(path)
            .map(|raw| raw.trim().to_string())
            .map_err(|_| CompileError::SecretNotFound(path.clone())),
        _ => Err(CompileError::InvalidSecretSource {
            name: secret.name.clone(),
        }),
    }
}

fn validate_hostname(host: &str) -> Result<(), CompileError> {
    let invalid = host.is_empty()
        || host.contains('/')
        || host.contains('*')
        || host.contains(' ')
        || host.parse::<std::net::IpAddr>().is_ok();
    if invalid {
        return Err(CompileError::InvalidHostname(host.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{
        AuditLevel, EgressMode, SandboxSpec, SchedulingPriority, API_VERSION_V1BETA1,
    };

    fn parse(yaml: &str) -> SandboxSpec {
        serde_yaml::from_str(yaml).expect("YAML di test deve essere valido")
    }

    fn minimal_spec(preset: &str) -> SandboxSpec {
        parse(&format!(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata:\n  name: test\n\
             spec:\n  runtime:\n    preset: {}\n",
            preset
        ))
    }

    #[test]
    fn detects_both_supported_versions() {
        let alpha = serde_json::json!({ "apiVersion": "sandbox.ai/v1alpha1" });
        let beta = serde_json::json!({ "apiVersion": API_VERSION_V1BETA1 });
        assert_eq!(detect_version(&alpha).unwrap(), SpecVersion::V1Alpha1);
        assert_eq!(detect_version(&beta).unwrap(), SpecVersion::V1Beta1);
    }

    #[test]
    fn compile_any_accepts_v1alpha1_json() {
        let raw = r#"{
          "apiVersion":"sandbox.ai/v1alpha1",
          "kind":"Sandbox",
          "metadata":{},
          "spec":{"runtime":{"preset":"python"}}
        }"#;
        let ir = compile_any(raw).unwrap();
        assert_eq!(ir.image, "python:3.12-slim");
    }

    #[test]
    fn compile_any_accepts_v1beta1_yaml() {
        let raw = r#"
apiVersion: sandbox.ai/v1beta1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
    version: "3.11"
  resources:
    timeoutMs: 45000
  network:
    egress:
      allow: ["pypi.org"]
      denyByDefault: true
      mode: proxy
  scheduling:
    backend: docker
    preferWarm: true
    priority: high
  storage:
    volumes: []
  observability:
    auditLevel: full
    metricsEnabled: true
"#;
        let ir = compile_any(raw).unwrap();
        assert_eq!(ir.image, "python:3.11-slim");
        assert_eq!(ir.exec_timeout_ms, Some(45000));
        assert_eq!(ir.egress_mode, Some(EgressMode::Proxy));
        assert_eq!(ir.backend_hint.as_deref(), Some("docker"));
        assert!(ir.prefer_warm);
        assert_eq!(ir.priority, Some(SchedulingPriority::High));
        assert_eq!(ir.audit_level, Some(AuditLevel::Full));
        assert!(ir.metrics_enabled);
    }

    #[test]
    fn schema_validation_collects_field_errors() {
        let raw = serde_json::json!({
            "apiVersion": "sandbox.ai/v1beta1",
            "kind": "Sandbox",
            "metadata": {},
            "spec": {
                "runtime": { "preset": "python" },
                "resources": { "cpuMillicores": -1 },
                "network": { "egress": { "mode": "bogus" } }
            }
        });
        let err = compile_value(raw).unwrap_err();
        match err {
            CompileError::SchemaValidation { issues, .. } => {
                assert!(issues.len() >= 2);
                assert!(issues
                    .iter()
                    .any(|issue| issue.path.contains("cpuMillicores")));
                assert!(issues.iter().any(|issue| issue.path.contains("mode")));
            }
            other => panic!("errore inatteso: {other:?}"),
        }
    }

    #[test]
    fn test_python_preset_resolves_image() {
        let ir = compile(minimal_spec("python")).unwrap();
        assert_eq!(ir.image, "python:3.12-slim");
    }

    #[test]
    fn test_node_preset_resolves_image() {
        let ir = compile(minimal_spec("node")).unwrap();
        assert_eq!(ir.image, "node:20-slim");
    }

    #[test]
    fn test_rust_preset_resolves_image() {
        let ir = compile(minimal_spec("rust")).unwrap();
        assert_eq!(ir.image, "rust:1.77-slim");
    }

    #[test]
    fn test_shell_preset_resolves_image() {
        let ir = compile(minimal_spec("shell")).unwrap();
        assert_eq!(ir.image, "ubuntu:24.04");
    }

    #[test]
    fn test_custom_preset_without_image_is_error() {
        let spec = minimal_spec("custom");
        assert!(matches!(
            compile(spec),
            Err(CompileError::CustomPresetNeedsImage)
        ));
    }

    #[test]
    fn test_explicit_image_overrides_preset() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n    image: my-registry/foo:42\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(ir.image, "my-registry/foo:42");
    }

    #[test]
    fn test_missing_runtime_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime: {}\n",
        );
        assert!(matches!(compile(spec), Err(CompileError::MissingRuntime)));
    }

    #[test]
    fn test_wrong_api_version_is_error() {
        let mut spec = minimal_spec("python");
        spec.api_version = "sandbox.ai/v2".into();
        assert!(matches!(
            compile(spec),
            Err(CompileError::UnsupportedApiVersion(_))
        ));
    }

    #[test]
    fn test_wrong_kind_is_error() {
        let mut spec = minimal_spec("python");
        spec.kind = "Pod".into();
        assert!(matches!(
            compile(spec),
            Err(CompileError::UnsupportedKind(_))
        ));
    }

    #[test]
    fn test_default_ttl_is_300() {
        let ir = compile(minimal_spec("python")).unwrap();
        assert_eq!(ir.ttl_seconds, 300);
    }

    #[test]
    fn test_custom_ttl_is_applied() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n  ttlSeconds: 900\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(ir.ttl_seconds, 900);
    }

    #[test]
    fn test_resources_override_defaults() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  resources:\n    cpuMillicores: 2000\n    memoryMb: 2048\n    diskMb: 4096\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(ir.cpu_millicores, 2000);
        assert_eq!(ir.memory_mb, 2048);
        assert_eq!(ir.disk_mb, 4096);
    }

    #[test]
    fn test_ip_in_egress_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  network:\n    egress:\n      allow: [\"1.2.3.4\"]\n      denyByDefault: true\n",
        );
        assert!(matches!(
            compile(spec),
            Err(CompileError::InvalidHostname(_))
        ));
    }

    #[test]
    fn test_wildcard_in_egress_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  network:\n    egress:\n      allow: [\"*.example.com\"]\n      denyByDefault: true\n",
        );
        assert!(matches!(
            compile(spec),
            Err(CompileError::InvalidHostname(_))
        ));
    }

    #[test]
    fn test_path_in_egress_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  network:\n    egress:\n      allow: [\"example.com/api\"]\n      denyByDefault: true\n",
        );
        assert!(matches!(
            compile(spec),
            Err(CompileError::InvalidHostname(_))
        ));
    }

    #[test]
    fn test_valid_egress_is_accepted() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  network:\n    egress:\n      allow: [\"pypi.org\", \"files.pythonhosted.org\"]\n      denyByDefault: true\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(ir.egress_allow, vec!["pypi.org", "files.pythonhosted.org"]);
        assert!(ir.deny_by_default);
    }

    #[test]
    fn test_egress_deny_by_default_defaults_true() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  network:\n    egress:\n      allow: [\"pypi.org\"]\n",
        );
        let ir = compile(spec).unwrap();
        assert!(ir.deny_by_default);
    }

    #[test]
    fn test_env_is_propagated_sorted() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n    env:\n      FOO: bar\n      BAZ: qux\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(
            ir.env,
            vec![
                ("BAZ".to_string(), "qux".to_string()),
                ("FOO".to_string(), "bar".to_string())
            ]
        );
    }

    #[test]
    fn test_secret_from_env_is_resolved() {
        let var_name = "AGENTSANDBOX_TEST_SECRET_OK";
        std::env::set_var(var_name, "deadbeef");

        let spec = parse(&format!(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {{}}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  secrets:\n    - name: MY_SECRET\n      valueFrom:\n        envRef: \"{}\"\n",
            var_name
        ));
        let ir = compile(spec).unwrap();
        assert_eq!(ir.secret_env, vec![("MY_SECRET".into(), "deadbeef".into())]);
        std::env::remove_var(var_name);
    }

    #[test]
    fn test_secret_with_both_sources_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  secrets:\n    - name: BAD\n      valueFrom:\n        envRef: X\n        file: /tmp/y\n",
        );
        assert!(matches!(
            compile(spec),
            Err(CompileError::InvalidSecretSource { .. })
        ));
    }

    #[test]
    fn test_secret_with_no_sources_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  secrets:\n    - name: BAD\n      valueFrom: {}\n",
        );
        assert!(matches!(
            compile(spec),
            Err(CompileError::InvalidSecretSource { .. })
        ));
    }

    #[test]
    fn test_secret_missing_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  secrets:\n    - name: MY_SECRET\n      valueFrom:\n        envRef: AGENTSANDBOX_DEFINITELY_MISSING_XYZ\n",
        );
        assert!(matches!(
            compile(spec),
            Err(CompileError::SecretNotFound(_))
        ));
    }

    #[test]
    fn test_working_dir_override_applied() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n    workingDir: /sandbox\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(ir.working_dir, "/sandbox");
    }

    #[test]
    fn test_unknown_top_level_field_is_error() {
        let err = compile_any(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             unexpected: true\n\
             spec:\n  runtime:\n    preset: python\n",
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::SchemaValidation { .. }));
    }

    #[test]
    fn test_unknown_runtime_field_is_error() {
        let err = compile_any(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n    bogus: true\n",
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::SchemaValidation { .. }));
    }
}
