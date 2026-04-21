use agentsandbox_backend_libkrun::LibkrunBackendFactory;
use agentsandbox_sdk::{backend::BackendFactory, error::BackendError};
use std::collections::HashMap;

#[tokio::test]
async fn conformance_suite() {
    let backend = LibkrunBackendFactory
        .create(&HashMap::new())
        .expect("la factory libkrun deve costruire il backend");

    match backend.health_check().await {
        Ok(()) => {
            let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
            report.print();
            assert!(report.all_passed(), "conformance suite fallita");
        }
        Err(BackendError::Unavailable(message)) => {
            eprintln!("skip conformance libkrun: {message}");
        }
        Err(error) => panic!("health_check libkrun inatteso: {error}"),
    }
}
