use agentsandbox_sdk::{
    backend::{SandboxBackend, SandboxState},
    error::BackendError,
    ir::SandboxIR,
};

pub struct ConformanceReport {
    pub results: Vec<(String, Result<(), String>)>,
}

impl ConformanceReport {
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|(_, result)| result.is_ok())
    }

    pub fn print(&self) {
        for (name, result) in &self.results {
            match result {
                Ok(()) => println!("  ok  {name}"),
                Err(message) => println!("  err {name} - {message}"),
            }
        }
        let passed = self
            .results
            .iter()
            .filter(|(_, result)| result.is_ok())
            .count();
        println!("\n  {passed}/{} test passati", self.results.len());
    }
}

impl Default for ConformanceReport {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run_all(backend: &dyn SandboxBackend) -> ConformanceReport {
    let mut report = ConformanceReport::new();
    let ir = SandboxIR::default_for_test();

    macro_rules! test {
        ($name:expr, $future:expr) => {
            report
                .results
                .push(($name.into(), $future.await.map_err(|e: String| e)));
        };
    }

    test!("health_check", health_check(backend));
    test!("create_handle_nonempty", create_handle(backend, &ir));
    test!("exec_stdout_marker", exec_stdout(backend, &ir));
    test!("exec_stderr_captured", exec_stderr(backend, &ir));
    test!("exec_nonzero_exit_code", exec_nonzero(backend, &ir));
    test!("status_running", status_running(backend, &ir));
    test!("destroy_cleans_up", destroy(backend, &ir));
    test!("destroy_idempotent", destroy_idempotent(backend, &ir));
    test!("concurrent_three", concurrent(backend, &ir, 3));

    report
}

async fn health_check(backend: &dyn SandboxBackend) -> Result<(), String> {
    backend.health_check().await.map_err(|e| e.to_string())
}

async fn create_handle(backend: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let handle = backend.create(ir).await.map_err(|e| e.to_string())?;
    if handle.is_empty() {
        return Err("handle vuoto".into());
    }
    backend.destroy(&handle).await.ok();
    Ok(())
}

async fn exec_stdout(backend: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let handle = backend.create(ir).await.map_err(|e| e.to_string())?;
    let result = backend
        .exec(&handle, "echo 'agentsandbox-conformance-ok'", None)
        .await
        .map_err(|e| e.to_string())?;
    backend.destroy(&handle).await.ok();
    if !result.stdout.contains("agentsandbox-conformance-ok") {
        return Err(format!("marker non in stdout: {:?}", result.stdout));
    }
    if result.exit_code != 0 {
        return Err(format!("exit_code inatteso: {}", result.exit_code));
    }
    Ok(())
}

async fn exec_stderr(backend: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let handle = backend.create(ir).await.map_err(|e| e.to_string())?;
    let result = backend
        .exec(&handle, "echo 'stderr-marker' >&2", None)
        .await
        .map_err(|e| e.to_string())?;
    backend.destroy(&handle).await.ok();
    if !result.stderr.contains("stderr-marker") {
        return Err(format!("marker non in stderr: {:?}", result.stderr));
    }
    Ok(())
}

async fn exec_nonzero(backend: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let handle = backend.create(ir).await.map_err(|e| e.to_string())?;
    let result = backend
        .exec(&handle, "exit 42", None)
        .await
        .map_err(|e| e.to_string())?;
    backend.destroy(&handle).await.ok();
    if result.exit_code != 42 {
        return Err(format!("atteso 42, got {}", result.exit_code));
    }
    Ok(())
}

async fn status_running(backend: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let handle = backend.create(ir).await.map_err(|e| e.to_string())?;
    let status = backend.status(&handle).await.map_err(|e| e.to_string())?;
    backend.destroy(&handle).await.ok();
    if status.state != SandboxState::Running {
        return Err(format!("atteso Running, got {:?}", status.state));
    }
    Ok(())
}

async fn destroy(backend: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let handle = backend.create(ir).await.map_err(|e| e.to_string())?;
    backend.destroy(&handle).await.map_err(|e| e.to_string())?;
    match backend.status(&handle).await {
        Err(BackendError::NotFound(_)) => Ok(()),
        Ok(status) if status.state == SandboxState::Stopped => Ok(()),
        Ok(status) => Err(format!("dopo destroy: {:?}", status.state)),
        Err(error) => Err(format!("dopo destroy errore inatteso: {error}")),
    }
}

async fn destroy_idempotent(backend: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let handle = backend.create(ir).await.map_err(|e| e.to_string())?;
    backend.destroy(&handle).await.map_err(|e| e.to_string())?;
    match backend.destroy(&handle).await {
        Ok(()) | Err(BackendError::NotFound(_)) => Ok(()),
        Err(error) => Err(format!("seconda destroy: {error}")),
    }
}

async fn concurrent(
    backend: &dyn SandboxBackend,
    ir: &SandboxIR,
    count: usize,
) -> Result<(), String> {
    let mut handles = Vec::with_capacity(count);
    for _ in 0..count {
        let mut sandbox = ir.clone();
        sandbox.id = uuid::Uuid::new_v4().to_string();
        handles.push(backend.create(&sandbox).await.map_err(|e| e.to_string())?);
    }
    for handle in handles {
        backend.destroy(&handle).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[macro_export]
macro_rules! run_conformance_suite {
    ($make_backend:expr) => {
        #[cfg(test)]
        mod conformance {
            use super::*;

            #[tokio::test]
            async fn full_suite() {
                let backend = ($make_backend)().await;
                let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
                report.print();
                assert!(report.all_passed(), "conformance suite fallita");
            }
        }
    };
}
