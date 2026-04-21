//! Conformance test helpers for [`SandboxAdapter`] implementations.
//!
//! This module is gated behind the `conformance` feature so that test-only
//! helpers don't bloat production builds. Each adapter crate enables the
//! feature from its `[dev-dependencies]` and calls the helpers from its own
//! `#[tokio::test]` functions, typically with `#[ignore]` so they only run
//! when a real backend is available.
//!
//! The helpers are independent: each one creates, uses and destroys its own
//! sandbox so failures stay local.

use crate::adapter::{SandboxAdapter, SandboxStatus};
use crate::ir::SandboxIR;

fn shell_ir() -> SandboxIR {
    SandboxIR {
        image: "alpine:3.20".into(),
        ttl_seconds: 60,
        ..SandboxIR::default()
    }
}

pub async fn test_create_and_destroy(adapter: &dyn SandboxAdapter) {
    let ir = shell_ir();
    let id = adapter.create(&ir).await.expect("create deve funzionare");
    assert_eq!(id, ir.id, "l'id restituito deve coincidere con ir.id");
    adapter.destroy(&id).await.expect("destroy deve funzionare");
}

pub async fn test_exec_returns_stdout(adapter: &dyn SandboxAdapter) {
    let ir = shell_ir();
    let id = adapter.create(&ir).await.unwrap();
    let result = adapter
        .exec(&id, "echo 'hello conformance'")
        .await
        .expect("exec deve tornare ExecResult");
    assert!(
        result.stdout.contains("hello conformance"),
        "stdout inatteso: {:?}",
        result.stdout
    );
    assert_eq!(result.exit_code, 0);
    adapter.destroy(&id).await.unwrap();
}

pub async fn test_exec_captures_stderr(adapter: &dyn SandboxAdapter) {
    let ir = shell_ir();
    let id = adapter.create(&ir).await.unwrap();
    let result = adapter.exec(&id, "echo 'err' >&2; exit 1").await.unwrap();
    assert!(result.stderr.contains("err"), "stderr vuoto: {:?}", result);
    assert_eq!(result.exit_code, 1);
    adapter.destroy(&id).await.unwrap();
}

pub async fn test_inspect_running(adapter: &dyn SandboxAdapter) {
    let ir = shell_ir();
    let id = adapter.create(&ir).await.unwrap();
    let info = adapter.inspect(&id).await.unwrap();
    assert_eq!(info.status, SandboxStatus::Running);
    assert_eq!(info.sandbox_id, id);
    adapter.destroy(&id).await.unwrap();
}

pub async fn test_destroy_nonexistent_is_ok(adapter: &dyn SandboxAdapter) {
    adapter
        .destroy("sandbox-che-non-esiste-xyzxyz")
        .await
        .expect("destroy di una sandbox inesistente deve essere idempotente");
}
