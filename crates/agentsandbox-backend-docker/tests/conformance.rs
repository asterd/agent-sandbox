use agentsandbox_backend_docker::DockerBackendFactory;
use agentsandbox_sdk::{backend::BackendFactory, error::BackendError};
use std::collections::HashMap;

async fn backend_can_create_default_test_image(
    backend: &dyn agentsandbox_sdk::backend::SandboxBackend,
) -> Result<(), BackendError> {
    let handle = backend
        .create(&agentsandbox_sdk::ir::SandboxIR::default_for_test())
        .await?;
    backend.destroy(&handle).await.ok();
    Ok(())
}

#[tokio::test]
async fn conformance_suite() {
    let backend = DockerBackendFactory
        .create(&HashMap::new())
        .expect("la factory Docker deve sempre costruire il backend");

    match backend.health_check().await {
        Ok(()) => {
            match backend_can_create_default_test_image(backend.as_ref()).await {
                Ok(()) => {}
                Err(BackendError::Internal(message))
                    if message.contains("No such image: python:3.12-slim") =>
                {
                    eprintln!("skip conformance Docker: immagine di test mancante ({message})");
                    return;
                }
                Err(error) => panic!("preflight Docker inatteso: {error}"),
            }
            let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
            report.print();
            assert!(report.all_passed(), "conformance suite fallita");
        }
        Err(BackendError::Unavailable(message)) => {
            eprintln!("skip conformance Docker: {message}");
        }
        Err(error) => panic!("health_check Docker inatteso: {error}"),
    }
}
