use agentsandbox_backend_libkrun::LibkrunBackendFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agentsandbox_sdk::plugin::serve_plugin(&LibkrunBackendFactory).await?;
    Ok(())
}
