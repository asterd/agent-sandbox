use agentsandbox_backend_gvisor::GVisorBackendFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agentsandbox_sdk::plugin::serve_plugin(&GVisorBackendFactory).await?;
    Ok(())
}
