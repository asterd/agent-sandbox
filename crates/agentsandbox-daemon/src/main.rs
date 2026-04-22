//! AgentSandbox daemon entry point.
//!
//! Boot sequence:
//!   1. tracing subscriber (env-filter driven)
//!   2. open (or create) the SQLite DB and run migrations
//!   3. discover installed backend plugins and health-check them
//!   4. spawn the TTL reaper
//!   5. serve the v1 API on 127.0.0.1:7847

use std::sync::Arc;

use agentsandbox_daemon::{
    config::load_config, metrics::Metrics, reaper, registry::BackendRegistry, router,
    state::AppState,
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

    let registry = BackendRegistry::discover(&cfg.backends).await;
    if registry.list_available().is_empty() {
        tracing::warn!("nessun backend plugin disponibile all'avvio");
    }

    let addr = cfg.listen_addr();
    let state = Arc::new(AppState {
        db,
        config: cfg,
        registry: Arc::new(registry),
        metrics: Metrics::new(),
    });

    let reaper_state = state.clone();
    tokio::spawn(async move { reaper::run(reaper_state).await });

    let app = router::build(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("daemon in ascolto su http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
