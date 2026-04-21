//! Integration tests for the Docker egress proxy flow introduced in Phase F.
//!
//! These tests require a real Docker daemon plus outbound access to pull the
//! Python image and reach the internet from the host. They are ignored by
//! default:
//!
//!     cargo test -p agentsandbox-backend-docker --test egress_proxy -- --ignored --test-threads=1

use agentsandbox_backend_docker::DockerBackend;
use agentsandbox_sdk::{
    backend::SandboxBackend,
    ir::{EgressIR, EgressMode, SandboxIR},
};
use bollard::Docker;

const FORCE_INTEGRATION_ENV: &str = "AGENTSANDBOX_INTEGRATION";

async fn client_or_skip() -> Option<Docker> {
    let docker = Docker::connect_with_local_defaults().ok()?;
    docker.ping().await.ok()?;
    Some(docker)
}

async fn docker_or_skip() -> Option<Docker> {
    if std::env::var(FORCE_INTEGRATION_ENV).as_deref() == Ok("1") {
        return Some(client_or_skip().await.expect("Docker non disponibile"));
    }

    let docker = client_or_skip().await;
    if docker.is_none() {
        eprintln!("skip: Docker non disponibile");
    }
    docker
}

fn proxy_ir(allow_hostnames: Vec<String>) -> SandboxIR {
    SandboxIR {
        image: "python:3.12-slim".into(),
        command: Some(vec!["sleep".into(), "120".into()]),
        egress: EgressIR {
            mode: EgressMode::Proxy,
            allow_hostnames,
            allow_ips: Vec::new(),
            deny_by_default: true,
        },
        ..SandboxIR::default()
    }
}

fn offline_ir() -> SandboxIR {
    SandboxIR {
        image: "python:3.12-slim".into(),
        command: Some(vec!["sleep".into(), "120".into()]),
        egress: EgressIR {
            mode: EgressMode::None,
            allow_hostnames: Vec::new(),
            allow_ips: Vec::new(),
            deny_by_default: true,
        },
        ..SandboxIR::default()
    }
}

#[tokio::test]
#[ignore = "richiede Docker + rete esterna"]
async fn proxy_allowlist_allows_pip_install() {
    let Some(docker) = docker_or_skip().await else {
        return;
    };
    let backend = DockerBackend::with_client(docker);
    let handle = backend
        .create(&proxy_ir(vec![
            "pypi.org".into(),
            "files.pythonhosted.org".into(),
        ]))
        .await
        .expect("create sandbox");

    let result = backend
        .exec(
            &handle,
            "python -m pip install --disable-pip-version-check --no-cache-dir idna==3.7",
            Some(120_000),
        )
        .await
        .expect("exec pip install");

    backend.destroy(&handle).await.expect("destroy sandbox");

    if result.exit_code != 0
        && (result.stderr.contains("CERTIFICATE_VERIFY_FAILED")
            || result.stderr.contains("self-signed certificate")
            || result.stdout.contains("CERTIFICATE_VERIFY_FAILED")
            || result.stdout.contains("self-signed certificate"))
    {
        eprintln!(
            "skip: ambiente con TLS MITM/non trusted CA nel guest. stdout={} stderr={}",
            result.stdout, result.stderr
        );
        return;
    }

    assert_eq!(
        result.exit_code, 0,
        "pip install doveva riuscire. stdout={} stderr={}",
        result.stdout, result.stderr
    );
}

#[tokio::test]
#[ignore = "richiede Docker + rete esterna"]
async fn proxy_blocks_non_allowlisted_hosts() {
    let Some(docker) = docker_or_skip().await else {
        return;
    };
    let backend = DockerBackend::with_client(docker);
    let handle = backend
        .create(&proxy_ir(vec!["pypi.org".into()]))
        .await
        .expect("create sandbox");

    let result = backend
        .exec(
            &handle,
            "python -c \"import urllib.request; urllib.request.urlopen('https://example.com', timeout=5)\"",
            Some(30_000),
        )
        .await
        .expect("exec blocked request");

    backend.destroy(&handle).await.expect("destroy sandbox");

    assert_ne!(
        result.exit_code, 0,
        "example.com doveva essere bloccato. stdout={} stderr={}",
        result.stdout, result.stderr
    );
}

#[tokio::test]
#[ignore = "richiede Docker + rete esterna"]
async fn egress_none_keeps_the_guest_offline() {
    let Some(docker) = docker_or_skip().await else {
        return;
    };
    let backend = DockerBackend::with_client(docker);
    let handle = backend.create(&offline_ir()).await.expect("create sandbox");

    let result = backend
        .exec(
            &handle,
            "python -c \"import urllib.request; urllib.request.urlopen('https://pypi.org', timeout=5)\"",
            Some(30_000),
        )
        .await
        .expect("exec offline request");

    backend.destroy(&handle).await.expect("destroy sandbox");

    assert_ne!(
        result.exit_code, 0,
        "network.egress.mode=none doveva bloccare la rete. stdout={} stderr={}",
        result.stdout, result.stderr
    );
}
