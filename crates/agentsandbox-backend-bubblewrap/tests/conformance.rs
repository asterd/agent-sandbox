use agentsandbox_backend_bubblewrap::BubblewrapBackendFactory;
use agentsandbox_sdk::{backend::BackendFactory, error::BackendError};
use std::collections::HashMap;

#[tokio::test]
async fn conformance_suite() {
    let backend = BubblewrapBackendFactory
        .create(&HashMap::new())
        .expect("la factory Bubblewrap deve costruire il backend");

    match backend.health_check().await {
        Ok(()) => {
            let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
            report.print();
            assert!(report.all_passed(), "conformance suite fallita");
        }
        Err(BackendError::Unavailable(message)) => {
            eprintln!("skip conformance Bubblewrap: {message}");
        }
        Err(error) => panic!("health_check Bubblewrap inatteso: {error}"),
    }
}
