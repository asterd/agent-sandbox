use agentsandbox_backend_gvisor::GVisorBackendFactory;
use agentsandbox_sdk::{backend::BackendFactory, error::BackendError};
use std::collections::HashMap;

#[tokio::test]
async fn conformance_suite() {
    let backend = GVisorBackendFactory
        .create(&HashMap::new())
        .expect("la factory gVisor deve sempre costruire il backend");

    match backend.health_check().await {
        Ok(()) => {
            let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
            report.print();
            assert!(report.all_passed(), "conformance suite fallita");
        }
        Err(BackendError::Unavailable(message)) => {
            eprintln!("skip conformance gVisor: {message}");
        }
        Err(error) => panic!("health_check gVisor inatteso: {error}"),
    }
}
