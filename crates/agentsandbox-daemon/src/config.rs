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
    pub enabled: Vec<String>,
    #[serde(default)]
    pub bubblewrap: BubblewrapBackendSection,
    #[serde(default)]
    pub docker: DockerBackendSection,
    #[serde(default)]
    pub gvisor: GVisorBackendSection,
    #[serde(default)]
    pub libkrun: LibkrunBackendSection,
    #[serde(default)]
    pub nsjail: NsjailBackendSection,
    #[serde(default)]
    pub podman: PodmanBackendSection,
    #[serde(default)]
    pub wasmtime: WasmtimeBackendSection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BubblewrapBackendSection {
    pub agent_path: Option<String>,
    pub bwrap_path: Option<String>,
    pub rootfs_base: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DockerBackendSection {
    pub socket: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct GVisorBackendSection {
    pub socket: Option<String>,
    pub runtime: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PodmanBackendSection {
    pub socket: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct NsjailBackendSection {
    pub agent_path: Option<String>,
    pub chroot_base: Option<String>,
    pub nsjail_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct WasmtimeBackendSection {
    pub node_wasm_path: Option<String>,
    pub python_wasm_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct LibkrunBackendSection {
    pub runtime: Option<String>,
    pub socket: Option<String>,
    pub rootfs_dir: Option<String>,
}

impl DaemonConfig {
    pub fn listen_addr(&self) -> String {
        format!("{}:{}", self.daemon.host, self.daemon.port)
    }
}

impl BackendsSection {
    pub fn config_for(&self, backend_id: &str) -> std::collections::HashMap<String, String> {
        match backend_id {
            "bubblewrap" => {
                let mut config = std::collections::HashMap::new();
                if let Some(path) = &self.bubblewrap.bwrap_path {
                    config.insert("bwrap_path".into(), path.clone());
                }
                if let Some(path) = &self.bubblewrap.rootfs_base {
                    config.insert("rootfs_base".into(), path.clone());
                }
                if let Some(path) = &self.bubblewrap.agent_path {
                    config.insert("agent_path".into(), path.clone());
                }
                config
            }
            "docker" => {
                let mut config = std::collections::HashMap::new();
                if let Some(socket) = &self.docker.socket {
                    config.insert("socket".into(), socket.clone());
                }
                config
            }
            "podman" => {
                let mut config = std::collections::HashMap::new();
                if let Some(socket) = &self.podman.socket {
                    config.insert("socket".into(), socket.clone());
                }
                config
            }
            "nsjail" => {
                let mut config = std::collections::HashMap::new();
                if let Some(path) = &self.nsjail.nsjail_path {
                    config.insert("nsjail_path".into(), path.clone());
                }
                if let Some(path) = &self.nsjail.chroot_base {
                    config.insert("chroot_base".into(), path.clone());
                }
                if let Some(path) = &self.nsjail.agent_path {
                    config.insert("agent_path".into(), path.clone());
                }
                config
            }
            "wasmtime" => {
                let mut config = std::collections::HashMap::new();
                if let Some(path) = &self.wasmtime.python_wasm_path {
                    config.insert("python_wasm_path".into(), path.clone());
                }
                if let Some(path) = &self.wasmtime.node_wasm_path {
                    config.insert("node_wasm_path".into(), path.clone());
                }
                config
            }
            "libkrun" => {
                let mut config = std::collections::HashMap::new();
                if let Some(socket) = &self.libkrun.socket {
                    config.insert("socket".into(), socket.clone());
                }
                if let Some(runtime) = &self.libkrun.runtime {
                    config.insert("runtime".into(), runtime.clone());
                }
                if let Some(rootfs_dir) = &self.libkrun.rootfs_dir {
                    config.insert("rootfs_dir".into(), rootfs_dir.clone());
                }
                config
            }
            "gvisor" => {
                let mut config = std::collections::HashMap::new();
                if let Some(socket) = &self.gvisor.socket {
                    config.insert("socket".into(), socket.clone());
                }
                if let Some(runtime) = &self.gvisor.runtime {
                    config.insert("runtime".into(), runtime.clone());
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
    if let Ok(path) = std::env::var("AS_BACKENDS_BUBBLEWRAP_BWRAP_PATH") {
        cfg.backends.bubblewrap.bwrap_path = Some(path);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_BUBBLEWRAP_ROOTFS_BASE") {
        cfg.backends.bubblewrap.rootfs_base = Some(path);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_BUBBLEWRAP_AGENT_PATH") {
        cfg.backends.bubblewrap.agent_path = Some(path);
    }
    if let Ok(socket) = std::env::var("AS_BACKENDS_DOCKER_SOCKET") {
        cfg.backends.docker.socket = Some(socket);
    }
    if let Ok(socket) = std::env::var("AS_BACKENDS_GVISOR_SOCKET") {
        cfg.backends.gvisor.socket = Some(socket);
    }
    if let Ok(runtime) = std::env::var("AS_BACKENDS_GVISOR_RUNTIME") {
        cfg.backends.gvisor.runtime = Some(runtime);
    }
    if let Ok(socket) = std::env::var("AS_BACKENDS_PODMAN_SOCKET") {
        cfg.backends.podman.socket = Some(socket);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_NSJAIL_PATH") {
        cfg.backends.nsjail.nsjail_path = Some(path);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_NSJAIL_CHROOT_BASE") {
        cfg.backends.nsjail.chroot_base = Some(path);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_NSJAIL_AGENT_PATH") {
        cfg.backends.nsjail.agent_path = Some(path);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_WASMTIME_PYTHON_WASM_PATH") {
        cfg.backends.wasmtime.python_wasm_path = Some(path);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_WASMTIME_NODE_WASM_PATH") {
        cfg.backends.wasmtime.node_wasm_path = Some(path);
    }
    if let Ok(socket) = std::env::var("AS_BACKENDS_LIBKRUN_SOCKET") {
        cfg.backends.libkrun.socket = Some(socket);
    }
    if let Ok(runtime) = std::env::var("AS_BACKENDS_LIBKRUN_RUNTIME") {
        cfg.backends.libkrun.runtime = Some(runtime);
    }
    if let Ok(path) = std::env::var("AS_BACKENDS_LIBKRUN_ROOTFS_DIR") {
        cfg.backends.libkrun.rootfs_dir = Some(path);
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
            cfg.backends.docker.socket.as_deref(),
            Some("/tmp/docker.sock")
        );
        assert_eq!(
            cfg.backends.gvisor.socket.as_deref(),
            Some("/tmp/gvisor.sock")
        );
        assert_eq!(cfg.backends.gvisor.runtime.as_deref(), Some("runsc-kvm"));
        assert_eq!(
            cfg.backends.podman.socket.as_deref(),
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
            cfg.backends.gvisor.socket.as_deref(),
            Some("/tmp/gvisor.sock")
        );
        assert_eq!(cfg.backends.gvisor.runtime.as_deref(), Some("runsc"));
        assert_eq!(
            cfg.backends.podman.socket.as_deref(),
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
        let _gvisor_socket = EnvGuard::set("AS_BACKENDS_GVISOR_SOCKET", "/env/gvisor.sock");
        let _gvisor_runtime = EnvGuard::set("AS_BACKENDS_GVISOR_RUNTIME", "runsc-debug");
        let _podman_socket = EnvGuard::set("AS_BACKENDS_PODMAN_SOCKET", "/env/podman.sock");

        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.daemon.port, 9999);
        assert_eq!(cfg.database.url, "sqlite://env.db");
        assert_eq!(cfg.backends.enabled, vec!["docker", "gvisor", "podman"]);
        assert_eq!(
            cfg.backends.gvisor.socket.as_deref(),
            Some("/env/gvisor.sock")
        );
        assert_eq!(cfg.backends.gvisor.runtime.as_deref(), Some("runsc-debug"));
        assert_eq!(
            cfg.backends.podman.socket.as_deref(),
            Some("/env/podman.sock")
        );
    }

    #[test]
    fn config_for_returns_backend_specific_socket() {
        let backends = BackendsSection {
            enabled: vec!["docker".into(), "gvisor".into(), "podman".into()],
            bubblewrap: BubblewrapBackendSection::default(),
            docker: DockerBackendSection {
                socket: Some("/docker.sock".into()),
            },
            gvisor: GVisorBackendSection {
                socket: Some("/gvisor.sock".into()),
                runtime: Some("runsc".into()),
            },
            libkrun: LibkrunBackendSection::default(),
            nsjail: NsjailBackendSection::default(),
            podman: PodmanBackendSection {
                socket: Some("/podman.sock".into()),
            },
            wasmtime: WasmtimeBackendSection::default(),
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
}
