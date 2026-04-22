use agentsandbox_backend_docker::DockerBackendFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agentsandbox_sdk::plugin::serve_plugin(&DockerBackendFactory).await?;
    Ok(())
}
