//! Spec -> IR compile pipeline.

use crate::{
    schema::validate_raw,
    spec::{NetworkSpec, ResourceSpec, RuntimePreset, SandboxSpec, SecretRef, API_VERSION_V1},
};
use agentsandbox_sdk::ir::{AuditLevel, EgressIR, EgressMode, SandboxIR, SchedulingPriority};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecVersion {
    V1,
}

impl SpecVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecVersion::V1 => API_VERSION_V1,
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
    #[error("apiVersion non supportata: {0}. L'unica versione valida e' '{API_VERSION_V1}'")]
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
    #[error("extensions richiede scheduling.backend esplicito")]
    ExtensionsRequireExplicitBackend,
    #[error("extensions.{0} non consentita: {1}")]
    ExtensionForbidden(String, String),
}

pub fn detect_version(raw: &Value) -> Result<SpecVersion, CompileError> {
    match raw.get("apiVersion").and_then(Value::as_str) {
        Some(API_VERSION_V1) => Ok(SpecVersion::V1),
        Some(v) => Err(CompileError::UnsupportedApiVersion(v.to_string())),
        None => Err(CompileError::MissingApiVersion),
    }
}

pub fn compile_any(raw: &str) -> Result<SandboxIR, CompileError> {
    let raw = raw.trim();
    let value: Value = if raw.starts_with('{') {
        serde_json::from_str(raw).map_err(|e| CompileError::ParseError(e.to_string()))?
    } else {
        serde_yaml::from_str(raw).map_err(|e| CompileError::ParseError(e.to_string()))?
    };

    compile_value(value)
}

pub fn compile_value(value: Value) -> Result<SandboxIR, CompileError> {
    let version = detect_version(&value)?;
    validate_raw(version, &value)?;

    let spec: SandboxSpec =
        serde_json::from_value(value).map_err(|e| CompileError::ParseError(e.to_string()))?;
    compile(spec)
}

pub fn compile(spec: SandboxSpec) -> Result<SandboxIR, CompileError> {
    if spec.api_version != API_VERSION_V1 {
        return Err(CompileError::UnsupportedApiVersion(spec.api_version));
    }
    if spec.kind != "Sandbox" {
        return Err(CompileError::UnsupportedKind(spec.kind));
    }

    let crate::spec::SandboxSpecBody {
        runtime,
        resources,
        network,
        secrets,
        ttl_seconds,
        scheduling,
        extensions,
        storage,
        observability,
    } = spec.spec;

    let mut ir = SandboxIR {
        image: resolve_image(&runtime.image, runtime.preset, runtime.version.as_deref())?,
        runtime_version: runtime.version,
        labels: spec.metadata.labels.unwrap_or_default(),
        ..SandboxIR::default()
    };

    if let Some(wd) = runtime.working_dir {
        ir.working_dir = wd;
    }

    if let Some(ext) = extensions {
        let backend = scheduling
            .as_ref()
            .and_then(|cfg| cfg.backend.as_deref())
            .ok_or(CompileError::ExtensionsRequireExplicitBackend)?;
        validate_extensions_safety(&ext, backend)?;
        ir.extensions = Some(ext);
    }

    apply_resources(&mut ir, resources);
    apply_network(&mut ir, network)?;
    apply_secrets(&mut ir, secrets)?;
    apply_runtime_env(&mut ir, runtime.env);

    if let Some(ttl) = ttl_seconds {
        ir.ttl_seconds = ttl;
    }
    if let Some(scheduling) = scheduling {
        ir.backend_hint = scheduling.backend;
        ir.prefer_warm = scheduling.prefer_warm;
        ir.priority = scheduling.priority.map(map_priority);
    }
    if let Some(storage) = storage {
        ir.storage_volumes = storage.volumes;
    }
    if let Some(observability) = observability {
        ir.audit_level = observability.audit_level.map(map_audit_level);
        ir.metrics_enabled = observability.metrics_enabled.unwrap_or(false);
    }

    Ok(ir)
}

fn validate_extensions_safety(ext: &Value, backend: &str) -> Result<(), CompileError> {
    let forbidden = match backend {
        "docker" | "podman" => vec![
            ("/docker/hostConfig/networkMode", "usa spec.network.egress"),
            ("/docker/name", "gestito internamente"),
            ("/podman/hostConfig/networkMode", "usa spec.network.egress"),
            ("/podman/name", "gestito internamente"),
        ],
        "firecracker" => vec![("/firecracker/vsock", "riservato al canale exec interno")],
        _ => vec![],
    };

    for (path, reason) in forbidden {
        if ext.pointer(path).is_some() {
            return Err(CompileError::ExtensionForbidden(
                path.trim_start_matches('/').replace('/', "."),
                reason.into(),
            ));
        }
    }

    Ok(())
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
        if let Some(v) = res.timeout_ms {
            ir.timeout_ms = v;
        }
    }
}

fn apply_network(ir: &mut SandboxIR, network: Option<NetworkSpec>) -> Result<(), CompileError> {
    if let Some(net) = network {
        for host in &net.egress.allow {
            validate_hostname(host)?;
        }
        ir.egress = EgressIR {
            mode: net
                .egress
                .mode
                .map(map_egress_mode)
                .unwrap_or(EgressMode::Proxy),
            allow_hostnames: net.egress.allow,
            allow_ips: Vec::new(),
            deny_by_default: net.egress.deny_by_default,
        };
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

fn map_egress_mode(mode: crate::spec::EgressMode) -> EgressMode {
    match mode {
        crate::spec::EgressMode::None => EgressMode::None,
        crate::spec::EgressMode::Proxy => EgressMode::Proxy,
        crate::spec::EgressMode::Passthrough => EgressMode::Passthrough,
    }
}

fn map_priority(priority: crate::spec::SchedulingPriority) -> SchedulingPriority {
    match priority {
        crate::spec::SchedulingPriority::Low => SchedulingPriority::Low,
        crate::spec::SchedulingPriority::Normal => SchedulingPriority::Normal,
        crate::spec::SchedulingPriority::High => SchedulingPriority::High,
    }
}

fn map_audit_level(level: crate::spec::AuditLevel) -> AuditLevel {
    match level {
        crate::spec::AuditLevel::None => AuditLevel::None,
        crate::spec::AuditLevel::Basic => AuditLevel::Basic,
        crate::spec::AuditLevel::Full => AuditLevel::Full,
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

    fn parse(yaml: &str) -> SandboxSpec {
        serde_yaml::from_str(yaml).expect("YAML di test deve essere valido")
    }

    fn minimal_spec(preset: &str) -> SandboxSpec {
        parse(&format!(
            "apiVersion: sandbox.ai/v1\n\
             kind: Sandbox\n\
             metadata:\n  name: test\n\
             spec:\n  runtime:\n    preset: {}\n",
            preset
        ))
    }

    #[test]
    fn detects_supported_version() {
        let raw = serde_json::json!({ "apiVersion": "sandbox.ai/v1" });
        assert_eq!(detect_version(&raw).unwrap(), SpecVersion::V1);
    }

    #[test]
    fn compile_any_accepts_v1_json() {
        let raw = r#"{
          "apiVersion":"sandbox.ai/v1",
          "kind":"Sandbox",
          "metadata":{},
          "spec":{"runtime":{"preset":"python"}}
        }"#;
        let ir = compile_any(raw).unwrap();
        assert_eq!(ir.image, "python:3.12-slim");
    }

    #[test]
    fn compile_any_accepts_v1_yaml() {
        let raw = r#"
apiVersion: sandbox.ai/v1
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
        assert_eq!(ir.timeout_ms, 45000);
        assert_eq!(ir.egress.mode, EgressMode::Proxy);
        assert_eq!(ir.backend_hint.as_deref(), Some("docker"));
        assert!(ir.prefer_warm);
        assert_eq!(ir.priority, Some(SchedulingPriority::High));
        assert_eq!(ir.audit_level, Some(AuditLevel::Full));
        assert!(ir.metrics_enabled);
    }

    #[test]
    fn compile_any_preserves_podman_backend_hint() {
        let raw = r#"
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  scheduling:
    backend: podman
"#;
        let ir = compile_any(raw).unwrap();
        assert_eq!(ir.backend_hint.as_deref(), Some("podman"));
    }

    #[test]
    fn compile_any_preserves_gvisor_backend_hint() {
        let raw = r#"
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  scheduling:
    backend: gvisor
"#;
        let ir = compile_any(raw).unwrap();
        assert_eq!(ir.backend_hint.as_deref(), Some("gvisor"));
    }

    #[test]
    fn compile_rejects_extensions_without_explicit_backend() {
        let raw = r#"
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  extensions:
    docker:
      hostConfig:
        capAdd: ["NET_ADMIN"]
"#;
        assert!(matches!(
            compile_any(raw),
            Err(CompileError::ExtensionsRequireExplicitBackend)
        ));
    }

    #[test]
    fn compile_rejects_forbidden_docker_extension() {
        let raw = r#"
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  scheduling:
    backend: docker
  extensions:
    docker:
      hostConfig:
        networkMode: host
"#;
        assert!(matches!(
            compile_any(raw),
            Err(CompileError::ExtensionForbidden(path, _))
            if path == "docker.hostConfig.networkMode"
        ));
    }

    #[test]
    fn schema_validation_collects_field_errors() {
        let raw = serde_json::json!({
            "apiVersion": "sandbox.ai/v1",
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  resources:\n    cpuMillicores: 2000\n    memoryMb: 2048\n    diskMb: 4096\n    timeoutMs: 1234\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(ir.cpu_millicores, 2000);
        assert_eq!(ir.memory_mb, 2048);
        assert_eq!(ir.disk_mb, 4096);
        assert_eq!(ir.timeout_ms, 1234);
    }

    #[test]
    fn test_ip_in_egress_is_error() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  network:\n    egress:\n      allow: [\"pypi.org\", \"files.pythonhosted.org\"]\n      denyByDefault: true\n      mode: proxy\n",
        );
        let ir = compile(spec).unwrap();
        assert_eq!(
            ir.egress.allow_hostnames,
            vec!["pypi.org", "files.pythonhosted.org"]
        );
        assert!(ir.egress.deny_by_default);
        assert_eq!(ir.egress.mode, EgressMode::Proxy);
    }

    #[test]
    fn test_egress_deny_by_default_defaults_true() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n\
               \n  network:\n    egress:\n      allow: [\"pypi.org\"]\n",
        );
        let ir = compile(spec).unwrap();
        assert!(ir.egress.deny_by_default);
    }

    #[test]
    fn test_env_is_propagated_sorted() {
        let spec = parse(
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
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
            "apiVersion: sandbox.ai/v1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n    bogus: true\n",
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::SchemaValidation { .. }));
    }
}
