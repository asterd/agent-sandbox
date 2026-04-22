use agentsandbox_backend_nsjail::NsjailBackendFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    agentsandbox_sdk::plugin::serve_plugin(&NsjailBackendFactory).await?;
    Ok(())
}
