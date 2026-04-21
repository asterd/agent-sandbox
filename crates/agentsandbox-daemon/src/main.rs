//! AgentSandbox daemon entry point.
//!
//! Boot sequence:
//!   1. tracing subscriber (env-filter driven)
//!   2. open (or create) the SQLite DB and run migrations
//!   3. initialize the backend registry and health-check plugins
//!   4. spawn the TTL reaper
//!   5. serve the v1 API on 127.0.0.1:7847

use std::sync::Arc;

use agentsandbox_backend_docker::DockerBackendFactory;
use agentsandbox_daemon::{
    config::load_config, reaper, registry::BackendRegistry, router, state::AppState,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

const DEFAULT_CONFIG_PATH: &str = "agentsandbox.toml";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::var("AS_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.into());
    let cfg = load_config(&config_path)?;
    let env_filter = format!("{},tower_http=info", cfg.daemon.log_level);

    let subscriber = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_new(env_filter)
            .unwrap_or_else(|_| "agentsandbox=info,tower_http=info".into()),
    );
    if cfg.daemon.log_format == "json" {
        subscriber.json().init();
    } else {
        subscriber.init();
    }

    let options = SqliteConnectOptions::from_str(&cfg.database.url)?.create_if_missing(true);
    let db = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let mut registry = BackendRegistry::new();
    for backend_id in &cfg.backends.enabled {
        let backend_config = cfg.backends.config_for(backend_id);
        match backend_id.as_str() {
            "docker" => {
                let factory = DockerBackendFactory;
                registry.register(&factory);
                registry.initialize(&factory, &backend_config).await;
            }
            other => {
                tracing::warn!(backend_id = %other, "backend non riconosciuto");
            }
        }
    }

    if registry.list_available().is_empty() {
        anyhow::bail!("nessun backend disponibile");
    }

    let state = Arc::new(AppState {
        db,
        registry: Arc::new(registry),
    });

    let reaper_state = state.clone();
    tokio::spawn(async move { reaper::run(reaper_state).await });

    let app = router::build(state);
    let addr = cfg.listen_addr();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("daemon in ascolto su http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
