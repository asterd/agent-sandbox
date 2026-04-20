//! AgentSandbox daemon entry point.
//!
//! Boot sequence:
//!   1. tracing subscriber (env-filter driven)
//!   2. open (or create) the SQLite DB and run migrations
//!   3. connect to the backend adapter and health-check it
//!   4. spawn the TTL reaper
//!   5. serve the v1 API on 127.0.0.1:7847

use std::sync::Arc;

use agentsandbox_core::SandboxAdapter;
use agentsandbox_daemon::{reaper, router, state::AppState};
use agentsandbox_docker::DockerAdapter;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

const DEFAULT_ADDR: &str = "127.0.0.1:7847";
const DEFAULT_DB_URL: &str = "sqlite://agentsandbox.db";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agentsandbox=info,tower_http=info".into()),
        )
        .init();

    let db_url = std::env::var("AGENTSANDBOX_DB").unwrap_or_else(|_| DEFAULT_DB_URL.into());
    let addr = std::env::var("AGENTSANDBOX_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.into());

    let options = SqliteConnectOptions::from_str(&db_url)?.create_if_missing(true);
    let db = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let adapter = Arc::new(DockerAdapter::new()?);
    adapter.health_check().await?;

    let state = Arc::new(AppState {
        db,
        adapter: adapter.clone(),
    });

    let reaper_state = state.clone();
    tokio::spawn(async move { reaper::run(reaper_state).await });

    let app = router::build(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("daemon in ascolto su http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
