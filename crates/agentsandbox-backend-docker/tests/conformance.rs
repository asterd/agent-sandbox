use agentsandbox_backend_docker::DockerBackendFactory;
use agentsandbox_sdk::backend::BackendFactory;
use std::collections::HashMap;

async fn make_backend() -> Box<dyn agentsandbox_sdk::backend::SandboxBackend> {
    DockerBackendFactory
        .create(&HashMap::new())
        .expect("Docker deve essere disponibile per i test di conformance")
}

agentsandbox_conformance::run_conformance_suite!(make_backend);
