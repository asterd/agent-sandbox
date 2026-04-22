use agentsandbox_backend_wasmtime::WasmtimeBackendFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agentsandbox_sdk::plugin::serve_plugin(&WasmtimeBackendFactory).await?;
    Ok(())
}
