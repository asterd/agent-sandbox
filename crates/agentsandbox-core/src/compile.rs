//! Spec → IR compile pipeline.
//!
//! Validation rules in one place:
//! * apiVersion must be supported
//! * runtime.image or runtime.preset must be set (preset=Custom requires image)
//! * egress.allow hostnames must not be IPs, wildcards or paths
//! * secrets must resolve against the host
//!
//! Every invalid input returns a [`CompileError`]; `compile` never panics.

use crate::{
    ir::SandboxIR,
    spec::{RuntimePreset, RuntimeSpec, SandboxSpec, SecretRef, API_VERSION_V1ALPHA1},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CompileError {
    #[error("apiVersion non supportata: {0}")]
    UnsupportedApiVersion(String),
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

/// Compile a validated public spec into the backend-agnostic [`SandboxIR`].
pub fn compile(spec: SandboxSpec) -> Result<SandboxIR, CompileError> {
    if spec.api_version != API_VERSION_V1ALPHA1 {
        return Err(CompileError::UnsupportedApiVersion(spec.api_version));
    }
    if spec.kind != "Sandbox" {
        return Err(CompileError::UnsupportedKind(spec.kind));
    }

    let body = spec.spec;
    let mut ir = SandboxIR {
        image: resolve_image(&body.runtime)?,
        ..SandboxIR::default()
    };

    if let Some(wd) = body.runtime.working_dir {
        ir.working_dir = wd;
    }

    if let Some(res) = body.resources {
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

    if let Some(net) = body.network {
        for host in &net.egress.allow {
            validate_hostname(host)?;
        }
        ir.egress_allow = net.egress.allow;
        ir.deny_by_default = net.egress.deny_by_default;
    }

    if let Some(secrets) = body.secrets {
        for s in &secrets {
            let value = resolve_secret(s)?;
            ir.secret_env.push((s.name.clone(), value));
        }
    }

    if let Some(ttl) = body.ttl_seconds {
        ir.ttl_seconds = ttl;
    }

    if let Some(env) = body.runtime.env {
        ir.env = env.into_iter().collect();
        ir.env.sort_by(|a, b| a.0.cmp(&b.0));
    }

    Ok(ir)
}

fn resolve_image(runtime: &RuntimeSpec) -> Result<String, CompileError> {
    if let Some(image) = &runtime.image {
        return Ok(image.clone());
    }
    match runtime.preset {
        Some(RuntimePreset::Python) => Ok("python:3.12-slim".into()),
        Some(RuntimePreset::Node) => Ok("node:20-slim".into()),
        Some(RuntimePreset::Rust) => Ok("rust:1.77-slim".into()),
        Some(RuntimePreset::Shell) => Ok("ubuntu:24.04".into()),
        Some(RuntimePreset::Custom) => Err(CompileError::CustomPresetNeedsImage),
        None => Err(CompileError::MissingRuntime),
    }
}

fn resolve_secret(s: &SecretRef) -> Result<String, CompileError> {
    match (&s.value_from.env_ref, &s.value_from.file) {
        (Some(name), None) => {
            std::env::var(name).map_err(|_| CompileError::SecretNotFound(name.clone()))
        }
        (None, Some(path)) => std::fs::read_to_string(path)
            .map(|raw| raw.trim().to_string())
            .map_err(|_| CompileError::SecretNotFound(path.clone())),
        _ => Err(CompileError::InvalidSecretSource {
            name: s.name.clone(),
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
    use crate::spec::SandboxSpec;

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
        assert!(ir.deny_by_default, "denyByDefault deve essere true di default");
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
        // Safety: tests run single-threaded per binary in cargo test by default
        // for this crate (no multi-threaded access to shared env in this test).
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
        assert!(matches!(compile(spec), Err(CompileError::SecretNotFound(_))));
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
        let err = serde_yaml::from_str::<SandboxSpec>(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             unexpected: true\n\
             spec:\n  runtime:\n    preset: python\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("unexpected"));
    }

    #[test]
    fn test_unknown_runtime_field_is_error() {
        let err = serde_yaml::from_str::<SandboxSpec>(
            "apiVersion: sandbox.ai/v1alpha1\n\
             kind: Sandbox\n\
             metadata: {}\n\
             spec:\n  runtime:\n    preset: python\n    bogus: true\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }
}
