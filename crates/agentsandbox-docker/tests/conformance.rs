//! Conformance test suite for [`DockerAdapter`].
//!
//! These tests require a reachable Docker daemon and the `alpine:3.20` image
//! to be pullable. They are marked `#[ignore]` by default so the basic
//! `cargo test` stays hermetic; run them with:
//!
//!     cargo test -p agentsandbox-docker -- --ignored --test-threads=1
//!
//! `--test-threads=1` avoids concurrent exec spam on the Docker daemon when
//! running locally.

use agentsandbox_core::conformance;
use agentsandbox_core::SandboxAdapter;
use agentsandbox_docker::DockerAdapter;

const FORCE_INTEGRATION_ENV: &str = "AGENTSANDBOX_INTEGRATION";

async fn adapter_or_skip() -> Option<DockerAdapter> {
    let adapter = DockerAdapter::new().ok()?;
    adapter.health_check().await.ok()?;
    // Warm the image up once so per-test create latency stays low.
    Some(adapter)
}

async fn adapter_or_fail() -> DockerAdapter {
    match adapter_or_skip().await {
        Some(adapter) => adapter,
        None => panic!(
            "Docker non disponibile. Avvia un daemon Docker reale oppure non eseguire \
             la suite di conformance. Per forzare il controllo end-to-end usa \
             {FORCE_INTEGRATION_ENV}=1."
        ),
    }
}

fn integration_forced() -> bool {
    std::env::var(FORCE_INTEGRATION_ENV).as_deref() == Ok("1")
}

#[tokio::test]
#[ignore = "richiede Docker in esecuzione"]
async fn create_and_destroy() {
    let adapter = if integration_forced() {
        adapter_or_fail().await
    } else {
        let Some(adapter) = adapter_or_skip().await else {
            eprintln!("skip: Docker non disponibile");
            return;
        };
        adapter
    };
    conformance::test_create_and_destroy(&adapter).await;
}

#[tokio::test]
#[ignore = "richiede Docker in esecuzione"]
async fn exec_returns_stdout() {
    let adapter = if integration_forced() {
        adapter_or_fail().await
    } else {
        let Some(adapter) = adapter_or_skip().await else {
            eprintln!("skip: Docker non disponibile");
            return;
        };
        adapter
    };
    conformance::test_exec_returns_stdout(&adapter).await;
}

#[tokio::test]
#[ignore = "richiede Docker in esecuzione"]
async fn exec_captures_stderr() {
    let adapter = if integration_forced() {
        adapter_or_fail().await
    } else {
        let Some(adapter) = adapter_or_skip().await else {
            eprintln!("skip: Docker non disponibile");
            return;
        };
        adapter
    };
    conformance::test_exec_captures_stderr(&adapter).await;
}

#[tokio::test]
#[ignore = "richiede Docker in esecuzione"]
async fn inspect_running() {
    let adapter = if integration_forced() {
        adapter_or_fail().await
    } else {
        let Some(adapter) = adapter_or_skip().await else {
            eprintln!("skip: Docker non disponibile");
            return;
        };
        adapter
    };
    conformance::test_inspect_running(&adapter).await;
}

#[tokio::test]
#[ignore = "richiede Docker in esecuzione"]
async fn destroy_nonexistent_is_ok() {
    let adapter = if integration_forced() {
        adapter_or_fail().await
    } else {
        let Some(adapter) = adapter_or_skip().await else {
            eprintln!("skip: Docker non disponibile");
            return;
        };
        adapter
    };
    conformance::test_destroy_nonexistent_is_ok(&adapter).await;
}
