use agentsandbox_backend_wasmtime::WasmtimeBackendFactory;
use agentsandbox_sdk::backend::BackendFactory;
use std::collections::HashMap;

#[tokio::test]
async fn conformance_suite() {
    let backend = WasmtimeBackendFactory
        .create(&HashMap::new())
        .expect("la factory Wasmtime deve costruire il backend");
    let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
    report.print();
    assert!(report.all_passed(), "conformance suite fallita");
}
