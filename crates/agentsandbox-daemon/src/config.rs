use anyhow::Context;
use serde::Deserialize;
use std::{collections::HashMap, path::Path};

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
    pub mode: AuthMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    SingleUser,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BackendsSection {
    #[serde(default)]
    pub enabled: Vec<String>,
    #[serde(default)]
    pub search_dirs: Vec<String>,
    #[serde(flatten, default)]
    pub plugin_config: HashMap<String, HashMap<String, String>>,
}

impl DaemonConfig {
    pub fn listen_addr(&self) -> String {
        format!("{}:{}", self.daemon.host, self.daemon.port)
    }
}

impl BackendsSection {
    pub fn config_for(&self, backend_id: &str) -> HashMap<String, String> {
        self.plugin_config
            .get(backend_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn is_enabled(&self, backend_id: &str) -> bool {
        self.enabled.is_empty() || self.enabled.iter().any(|item| item == backend_id)
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
        .set_default("backends.enabled", Vec::<String>::new())?
        .set_default("backends.search_dirs", vec!["target/debug".to_string()])?;

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
        cfg.auth.mode = match mode.as_str() {
            "single_user" => AuthMode::SingleUser,
            "api_key" => AuthMode::ApiKey,
            _ => anyhow::bail!("AS_AUTH_MODE non valido: {mode}"),
        };
    }
    if let Ok(enabled) = std::env::var("AS_BACKENDS_ENABLED") {
        cfg.backends.enabled = enabled
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect();
    }
    if let Ok(search_dirs) = std::env::var("AS_BACKENDS_SEARCH_DIRS") {
        cfg.backends.search_dirs = search_dirs
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect();
    }

    for (key, value) in std::env::vars() {
        let Some(rest) = key.strip_prefix("AS_BACKENDS_") else {
            continue;
        };
        if rest == "ENABLED" || rest == "SEARCH_DIRS" {
            continue;
        }
        let mut parts = rest.split('_');
        let Some(backend_id) = parts.next() else {
            continue;
        };
        let config_key = parts.collect::<Vec<_>>().join("_").to_ascii_lowercase();
        if config_key.is_empty() {
            continue;
        }
        cfg.backends
            .plugin_config
            .entry(backend_id.to_ascii_lowercase())
            .or_default()
            .insert(config_key, value);
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
enabled = ["docker", "gvisor", "podman"]
search_dirs = ["target/debug", "/opt/agentsandbox/plugins"]

[backends.docker]
socket = "/tmp/docker.sock"

[backends.gvisor]
socket = "/tmp/gvisor.sock"
runtime = "runsc-kvm"

[backends.podman]
socket = "/tmp/podman.sock"
"#,
        );

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.daemon.host, "0.0.0.0");
        assert_eq!(cfg.daemon.port, 9000);
        assert_eq!(cfg.database.url, "sqlite://dev.db");
        assert_eq!(cfg.auth.mode, AuthMode::ApiKey);
        assert_eq!(
            cfg.backends.search_dirs,
            vec!["target/debug", "/opt/agentsandbox/plugins"]
        );
        assert_eq!(
            cfg.backends
                .config_for("docker")
                .get("socket")
                .map(String::as_str),
            Some("/tmp/docker.sock")
        );
        assert_eq!(
            cfg.backends
                .config_for("gvisor")
                .get("socket")
                .map(String::as_str),
            Some("/tmp/gvisor.sock")
        );
        assert_eq!(
            cfg.backends
                .config_for("gvisor")
                .get("runtime")
                .map(String::as_str),
            Some("runsc-kvm")
        );
        assert_eq!(
            cfg.backends
                .config_for("podman")
                .get("socket")
                .map(String::as_str),
            Some("/tmp/podman.sock")
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
  enabled: [docker, gvisor, podman]
  search_dirs: [target/debug, /opt/agentsandbox/plugins]
  gvisor:
    socket: /tmp/gvisor.sock
    runtime: runsc
  podman:
    socket: /tmp/podman.sock
"#,
        );

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.daemon.port, 7848);
        assert_eq!(cfg.database.url, "sqlite://yaml.db");
        assert_eq!(cfg.auth.mode, AuthMode::SingleUser);
        assert_eq!(
            cfg.backends.search_dirs,
            vec!["target/debug", "/opt/agentsandbox/plugins"]
        );
        assert_eq!(
            cfg.backends
                .config_for("gvisor")
                .get("socket")
                .map(String::as_str),
            Some("/tmp/gvisor.sock")
        );
        assert_eq!(
            cfg.backends
                .config_for("gvisor")
                .get("runtime")
                .map(String::as_str),
            Some("runsc")
        );
        assert_eq!(
            cfg.backends
                .config_for("podman")
                .get("socket")
                .map(String::as_str),
            Some("/tmp/podman.sock")
        );
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
        let _enabled = EnvGuard::set("AS_BACKENDS_ENABLED", "docker,gvisor,podman");
        let _search_dirs = EnvGuard::set(
            "AS_BACKENDS_SEARCH_DIRS",
            "target/debug,/Users/example/.local/lib/agentsandbox/plugins",
        );
        let _gvisor_socket = EnvGuard::set("AS_BACKENDS_GVISOR_SOCKET", "/env/gvisor.sock");
        let _gvisor_runtime = EnvGuard::set("AS_BACKENDS_GVISOR_RUNTIME", "runsc-debug");
        let _podman_socket = EnvGuard::set("AS_BACKENDS_PODMAN_SOCKET", "/env/podman.sock");

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.daemon.port, 9999);
        assert_eq!(cfg.database.url, "sqlite://env.db");
        assert_eq!(cfg.backends.enabled, vec!["docker", "gvisor", "podman"]);
        assert_eq!(
            cfg.backends.search_dirs,
            vec!["target/debug", "/Users/example/.local/lib/agentsandbox/plugins"]
        );
        assert_eq!(
            cfg.backends
                .config_for("gvisor")
                .get("socket")
                .map(String::as_str),
            Some("/env/gvisor.sock")
        );
        assert_eq!(
            cfg.backends
                .config_for("gvisor")
                .get("runtime")
                .map(String::as_str),
            Some("runsc-debug")
        );
        assert_eq!(
            cfg.backends
                .config_for("podman")
                .get("socket")
                .map(String::as_str),
            Some("/env/podman.sock")
        );
    }

    #[test]
    fn config_for_returns_backend_specific_socket() {
        let backends = BackendsSection {
            enabled: vec!["docker".into(), "gvisor".into(), "podman".into()],
            search_dirs: vec!["target/debug".into()],
            plugin_config: HashMap::from([
                (
                    "docker".into(),
                    HashMap::from([("socket".into(), "/docker.sock".into())]),
                ),
                (
                    "gvisor".into(),
                    HashMap::from([
                        ("socket".into(), "/gvisor.sock".into()),
                        ("runtime".into(), "runsc".into()),
                    ]),
                ),
                (
                    "podman".into(),
                    HashMap::from([("socket".into(), "/podman.sock".into())]),
                ),
            ]),
        };

        assert_eq!(
            backends
                .config_for("docker")
                .get("socket")
                .map(String::as_str),
            Some("/docker.sock")
        );
        assert_eq!(
            backends
                .config_for("gvisor")
                .get("socket")
                .map(String::as_str),
            Some("/gvisor.sock")
        );
        assert_eq!(
            backends
                .config_for("gvisor")
                .get("runtime")
                .map(String::as_str),
            Some("runsc")
        );
        assert_eq!(
            backends
                .config_for("podman")
                .get("socket")
                .map(String::as_str),
            Some("/podman.sock")
        );
    }

    #[test]
    fn enabled_filter_is_optional() {
        let backends = BackendsSection {
            enabled: Vec::new(),
            search_dirs: vec!["target/debug".into()],
            plugin_config: HashMap::new(),
        };

        assert!(backends.is_enabled("docker"));
        assert!(backends.is_enabled("custom"));
    }
}
