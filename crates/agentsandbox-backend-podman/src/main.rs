use agentsandbox_backend_podman::PodmanBackendFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agentsandbox_sdk::plugin::serve_plugin(&PodmanBackendFactory).await?;
    Ok(())
}
