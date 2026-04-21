use anyhow::Context;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DaemonConfig {
    pub daemon: DaemonSection,
    pub database: DatabaseSection,
    pub auth: AuthSection,
    pub backends: BackendsSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DaemonSection {
    pub host: String,
    pub port: u16,
    pub log_level: String,
    pub log_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DatabaseSection {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuthSection {
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BackendsSection {
    pub enabled: Vec<String>,
    #[serde(default)]
    pub docker: DockerBackendSection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DockerBackendSection {
    pub socket: Option<String>,
}

impl DaemonConfig {
    pub fn listen_addr(&self) -> String {
        format!("{}:{}", self.daemon.host, self.daemon.port)
    }
}

impl BackendsSection {
    pub fn config_for(&self, backend_id: &str) -> std::collections::HashMap<String, String> {
        match backend_id {
            "docker" => {
                let mut config = std::collections::HashMap::new();
                if let Some(socket) = &self.docker.socket {
                    config.insert("socket".into(), socket.clone());
                }
                config
            }
            _ => std::collections::HashMap::new(),
        }
    }
}

pub fn load_config(path: &str) -> anyhow::Result<DaemonConfig> {
    let mut builder = config::Config::builder()
        .set_default("daemon.host", "127.0.0.1")?
        .set_default("daemon.port", 7847)?
        .set_default("daemon.log_level", "info")?
        .set_default("daemon.log_format", "text")?
        .set_default("database.url", "sqlite://agentsandbox.db")?
        .set_default("auth.mode", "single_user")?
        .set_default("backends.enabled", vec!["docker"])?;

    let config_path = Path::new(path);
    if config_path.exists() {
        let format = match config_path.extension().and_then(|ext| ext.to_str()) {
            Some("yaml" | "yml") => config::FileFormat::Yaml,
            _ => config::FileFormat::Toml,
        };
        builder = builder.add_source(config::File::new(path, format));
    }

    let cfg = builder.build()?;
    let mut parsed: DaemonConfig = cfg.try_deserialize()?;
    apply_env_overrides(&mut parsed)?;
    Ok(parsed)
}

fn apply_env_overrides(cfg: &mut DaemonConfig) -> anyhow::Result<()> {
    if let Ok(host) = std::env::var("AS_DAEMON_HOST") {
        cfg.daemon.host = host;
    }
    if let Ok(port) = std::env::var("AS_DAEMON_PORT") {
        cfg.daemon.port = port
            .parse()
            .with_context(|| format!("AS_DAEMON_PORT non valido: {port}"))?;
    }
    if let Ok(level) = std::env::var("AS_DAEMON_LOG_LEVEL") {
        cfg.daemon.log_level = level;
    }
    if let Ok(format) = std::env::var("AS_DAEMON_LOG_FORMAT") {
        cfg.daemon.log_format = format;
    }
    if let Ok(url) = std::env::var("AS_DATABASE_URL") {
        cfg.database.url = url;
    }
    if let Ok(mode) = std::env::var("AS_AUTH_MODE") {
        cfg.auth.mode = mode;
    }
    if let Ok(enabled) = std::env::var("AS_BACKENDS_ENABLED") {
        cfg.backends.enabled = enabled
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect();
    }
    if let Ok(socket) = std::env::var("AS_BACKENDS_DOCKER_SOCKET") {
        cfg.backends.docker.socket = Some(socket);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn clear_env(key: &'static str) -> EnvGuard {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        EnvGuard { key, original }
    }

    fn temp_file(ext: &str, body: &str) -> String {
        let name = format!(
            "agentsandbox-config-{}.{ext}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(name);
        fs::write(&path, body).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn loads_toml_config() {
        let _guard = env_lock().lock().unwrap();
        let _port = clear_env("AS_DAEMON_PORT");
        let _db = clear_env("AS_DATABASE_URL");
        let _enabled = clear_env("AS_BACKENDS_ENABLED");
        let path = temp_file(
            "toml",
            r#"
[daemon]
host = "0.0.0.0"
port = 9000
log_level = "debug"
log_format = "json"

[database]
url = "sqlite://dev.db"

[auth]
mode = "api_key"

[backends]
enabled = ["docker"]

[backends.docker]
socket = "/tmp/docker.sock"
"#,
        );

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.daemon.host, "0.0.0.0");
        assert_eq!(cfg.daemon.port, 9000);
        assert_eq!(cfg.database.url, "sqlite://dev.db");
        assert_eq!(
            cfg.backends.docker.socket.as_deref(),
            Some("/tmp/docker.sock")
        );
    }

    #[test]
    fn loads_yaml_config() {
        let _guard = env_lock().lock().unwrap();
        let _port = clear_env("AS_DAEMON_PORT");
        let _db = clear_env("AS_DATABASE_URL");
        let _enabled = clear_env("AS_BACKENDS_ENABLED");
        let path = temp_file(
            "yaml",
            r#"
daemon:
  host: 127.0.0.1
  port: 7848
database:
  url: sqlite://yaml.db
auth:
  mode: single_user
backends:
  enabled: [docker]
"#,
        );

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.daemon.port, 7848);
        assert_eq!(cfg.database.url, "sqlite://yaml.db");
    }

    #[test]
    fn env_overrides_take_precedence() {
        let _guard = env_lock().lock().unwrap();
        let path = temp_file(
            "toml",
            r#"
[daemon]
host = "127.0.0.1"
port = 7847
log_level = "info"
log_format = "text"

[database]
url = "sqlite://file.db"

[auth]
mode = "single_user"

[backends]
enabled = ["docker"]
"#,
        );
        let _port = EnvGuard::set("AS_DAEMON_PORT", "9999");
        let _db = EnvGuard::set("AS_DATABASE_URL", "sqlite://env.db");
        let _enabled = EnvGuard::set("AS_BACKENDS_ENABLED", "docker,noop");

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.daemon.port, 9999);
        assert_eq!(cfg.database.url, "sqlite://env.db");
        assert_eq!(cfg.backends.enabled, vec!["docker", "noop"]);
    }
}
