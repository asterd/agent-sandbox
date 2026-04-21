#[tokio::main]
async fn main() {
    let socket_path = std::env::var("AGENTSANDBOX_SOCKET")
        .unwrap_or_else(|_| "/tmp/agentsandbox.sock".to_string());

    if let Err(error) = agentsandbox_agent::run_server(&socket_path).await {
        eprintln!("agentsandbox-agent fatal: {error}");
        std::process::exit(1);
    }
}
