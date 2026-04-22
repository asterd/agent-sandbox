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
    config::{load_config, AuthMode, DaemonConfig},
    metrics::Metrics,
    reaper,
    registry::BackendRegistry,
    router,
    state::AppState,
    store,
};
use sha2::{Digest, Sha256};
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
    validate_startup(&cfg, &db).await?;
    store::reconcile_concurrent_usage(&db).await?;
    persist_runtime_metadata(&db, &cfg).await?;

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

async fn validate_startup(cfg: &DaemonConfig, db: &sqlx::SqlitePool) -> anyhow::Result<()> {
    if cfg.limits.max_ttl_seconds == 0 {
        anyhow::bail!("limits.max_ttl_seconds deve essere > 0");
    }
    if cfg.limits.default_timeout_ms == 0 {
        anyhow::bail!("limits.default_timeout_ms deve essere > 0");
    }
    if cfg.limits.max_file_bytes == 0 {
        anyhow::bail!("limits.max_file_bytes deve essere > 0");
    }
    if cfg.security.require_api_key_non_local
        && !is_local_host(&cfg.daemon.host)
        && cfg.auth.mode != AuthMode::ApiKey
    {
        anyhow::bail!(
            "auth.mode=api_key e' obbligatorio fuori da localhost quando security.require_api_key_non_local=true"
        );
    }
    if cfg.auth.mode == AuthMode::ApiKey {
        let active_tenants: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM tenants WHERE enabled = 1")
                .fetch_one(db)
                .await?;
        if active_tenants.0 == 0 {
            anyhow::bail!(
                "auth.mode=api_key richiede almeno un tenant attivo; eseguire scripts/bootstrap_tenant.sh prima dell'avvio"
            );
        }
    }
    Ok(())
}

fn is_local_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

async fn persist_runtime_metadata(db: &sqlx::SqlitePool, cfg: &DaemonConfig) -> anyhow::Result<()> {
    let config_json = serde_json::to_vec(cfg)?;
    let config_hash = format!("{:x}", Sha256::digest(config_json));
    store::set_runtime_metadata(db, "daemon_version", env!("CARGO_PKG_VERSION")).await?;
    store::set_runtime_metadata(db, "config_profile", &cfg.profile).await?;
    store::set_runtime_metadata(db, "config_hash", &config_hash).await?;
    store::set_runtime_metadata(db, "config_path", &cfg.source_path).await?;
    store::set_runtime_metadata(db, "schema_version", "004_internal_service").await?;
    Ok(())
}
