use agentsandbox_backend_podman::PodmanBackendFactory;
use agentsandbox_sdk::{backend::BackendFactory, error::BackendError};
use std::collections::HashMap;

#[tokio::test]
async fn conformance_suite() {
    let backend = PodmanBackendFactory
        .create(&HashMap::new())
        .expect("la factory Podman deve sempre costruire il backend");

    match backend.health_check().await {
        Ok(()) => {
            let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
            report.print();
            assert!(report.all_passed(), "conformance suite fallita");
        }
        Err(BackendError::Unavailable(message)) => {
            eprintln!("skip conformance Podman: {message}");
        }
        Err(error) => panic!("health_check Podman inatteso: {error}"),
    }
}
