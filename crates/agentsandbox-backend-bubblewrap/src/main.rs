use agentsandbox_backend_bubblewrap::BubblewrapBackendFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agentsandbox_sdk::plugin::serve_plugin(&BubblewrapBackendFactory).await?;
    Ok(())
}
