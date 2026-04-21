# AgentSandbox — Documento di Continuazione
> **Documento autoconsistente per continuare lo sviluppo.**
> Prerequisito: implementazione completa di v1alpha1 (Fasi 0-8) + Fase A di v1stable.
> **Parti dalla Fase B di questo documento.**

---

## Stato dell'arte — cosa esiste già

Prima di iniziare qualsiasi nuova fase, verifica che questi elementi siano presenti
e funzionanti nel tuo repository:

```
agentsandbox/
├── crates/
│   ├── agentsandbox-core/        ✅ spec parser, IR, compile pipeline
│   └── agentsandbox-daemon/      ✅ HTTP API, SQLite, TTL reaper
│   └── agentsandbox-docker/      ✅ Docker adapter (NON ancora plugin)
├── sdks/
│   ├── python/                   ✅ SDK Python funzionante
│   └── typescript/               ✅ SDK TypeScript funzionante
└── examples/
    ├── python-code-review-agent/ ✅
    └── ts-dependency-auditor/    ✅
```

**Checklist di verifica pre-partenza:**
- [ ] `cargo test --workspace` — tutti i test passano
- [ ] `cargo run -p agentsandbox-daemon` — daemon si avvia senza errori
- [ ] Esempio code-review-agent funziona end-to-end
- [ ] La spec v1beta1 (da Fase A) è accettata dal compile pipeline

---

## Correzioni da applicare PRIMA di iniziare Fase B
**Stima: 2-3 ore — non saltare questo step**

Queste correzioni allineano il codice esistente alle decisioni architetturali finali.
Sono piccole ma bloccanti per tutto ciò che segue.

### Fix 1 — Versione spec: da v1beta1 a v1

La Fase A ha introdotto `sandbox.ai/v1beta1`. Durante lo sviluppo pre-stable
non serve retrocompatibilità. Porta tutto a `sandbox.ai/v1`.

```rust
// crates/agentsandbox-core/src/compile.rs
// PRIMA:
if spec.api_version != "sandbox.ai/v1alpha1"
    && spec.api_version != "sandbox.ai/v1beta1" { ... }

// DOPO:
if spec.api_version != "sandbox.ai/v1" {
    return Err(CompileError::UnsupportedVersion(
        format!("'{}'. L'unica versione valida è 'sandbox.ai/v1'", spec.api_version)
    ));
}
```

```yaml
# Aggiorna tutti i file di test, fixture e documentazione:
# s/sandbox.ai\/v1alpha1/sandbox.ai\/v1/g
# s/sandbox.ai\/v1beta1/sandbox.ai\/v1/g
```

```json
// spec/sandbox.v1.schema.json — rinomina da qualsiasi nome precedente
// Aggiorna il campo apiVersion nell'enum:
{ "enum": ["sandbox.ai/v1"] }
```

### Fix 2 — Configurazione daemon: aggiungi supporto YAML

Il daemon legge solo TOML oggi. Aggiungi supporto YAML con il crate `config`.

```toml
# Cargo.toml workspace — aggiungi:
config = { version = "0.14", features = ["toml", "yaml"] }
```

```rust
// crates/agentsandbox-daemon/src/config.rs
// Sostituisci il parsing manuale con:

use config::{Config, File, FileFormat, Environment};

pub fn load_config(path: &str) -> anyhow::Result<DaemonConfig> {
    let format = if path.ends_with(".yaml") || path.ends_with(".yml") {
        FileFormat::Yaml
    } else {
        FileFormat::Toml  // default
    };

    let cfg = Config::builder()
        .add_source(File::new(path, format))
        // Override da env: AS_DAEMON_PORT, AS_DATABASE_URL, ecc.
        .add_source(Environment::with_prefix("AS").separator("_"))
        // Defaults
        .set_default("daemon.host", "127.0.0.1")?
        .set_default("daemon.port", 7847)?
        .set_default("daemon.log_level", "info")?
        .set_default("daemon.log_format", "text")?
        .set_default("database.url", "sqlite://agentsandbox.db")?
        .set_default("auth.mode", "single_user")?
        .set_default("backends.enabled", vec!["docker"])?
        .build()?;

    Ok(cfg.try_deserialize()?)
}
```

```toml
# agentsandbox.toml — struttura target (crea o aggiorna)
[daemon]
host = "127.0.0.1"
port = 7847
log_level = "info"
log_format = "text"   # "text" | "json"

[database]
url = "sqlite://agentsandbox.db"

[auth]
mode = "single_user"  # "single_user" | "api_key"

[backends]
enabled = ["docker"]

[backends.docker]
socket = "/var/run/docker.sock"
```

### Fix 3 — Spec parser: aggiungi JSON esplicito

Il parser probabilmente accetta solo YAML oggi. Assicurati che JSON sia accettato.

```rust
// crates/agentsandbox-core/src/parse.rs
pub fn parse_spec(input: &str) -> Result<SandboxSpec, ParseError> {
    let input = input.trim();
    if input.starts_with('{') {
        serde_json::from_str(input).map_err(|e| ParseError::Json(e.to_string()))
    } else {
        serde_yaml::from_str(input).map_err(|e| ParseError::Yaml(e.to_string()))
    }
}
```

### Fix 4 — Aggiorna i test esistenti

```bash
# Aggiorna tutti i riferimenti alla versione nelle test fixture:
find . -name "*.rs" -o -name "*.yaml" -o -name "*.json" | \
  xargs grep -l "v1alpha1\|v1beta1" | \
  xargs sed -i 's/sandbox\.ai\/v1alpha1/sandbox.ai\/v1/g; s/sandbox\.ai\/v1beta1/sandbox.ai\/v1/g'

# Verifica che non rimanga nulla:
grep -r "v1alpha1\|v1beta1" . --include="*.rs" --include="*.yaml" --include="*.json"
# Output atteso: nessun risultato
```

**Criterio di completamento pre-Fase B:**
- [ ] `cargo test --workspace` passa con la nuova versione `sandbox.ai/v1`
- [ ] Il daemon accetta configurazione `.toml` e `.yaml`
- [ ] `curl -X POST localhost:7847/v1/sandboxes -d '{"apiVersion":"sandbox.ai/v1",...}'` — funziona
- [ ] `curl -X POST localhost:7847/v1/sandboxes` con body YAML — funziona

---

## FASE B — Plugin Architecture
**Stima: 4-5 giorni**
**Questa è la fase più importante: tutto il resto dipende da essa.**

L'obiettivo è trasformare il Docker adapter hardcoded in un sistema dove
ogni backend è un plugin intercambiabile. Docker diventa il primo plugin,
identico architetturalmente a qualsiasi backend che un contributor esterno potrebbe scrivere.

### B.1 — Nuovo crate: `agentsandbox-sdk`

Questo crate contiene il trait pubblico. È la **sola** dipendenza che un
backend esterno deve avere sul progetto.

```toml
# crates/agentsandbox-sdk/Cargo.toml
[package]
name = "agentsandbox-sdk"
version = "0.1.0"
edition = "2021"

# IMPORTANTE: nessuna dipendenza da Docker, SQLx, Axum.
# Questo crate deve essere usabile da chiunque voglia scrivere un backend.
[dependencies]
async-trait = "0.1"
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
thiserror   = "1"
chrono      = { version = "0.4", features = ["serde"] }
uuid        = { version = "1", features = ["v4"] }
```

```rust
// crates/agentsandbox-sdk/src/lib.rs
pub mod backend;
pub mod ir;
pub mod error;

pub const BACKEND_TRAIT_VERSION: &str = "1";
```

```rust
// crates/agentsandbox-sdk/src/ir.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxIR {
    pub id: String,
    pub image: String,
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    #[serde(skip_serializing)]  // mai nei log, mai fuori dal processo
    pub secret_env: Vec<(String, String)>,
    pub cpu_millicores: u32,
    pub memory_mb: u32,
    pub disk_mb: u32,
    pub egress: EgressIR,
    pub ttl_seconds: u64,
    pub timeout_ms: u64,
    pub working_dir: String,
    pub labels: HashMap<String, String>,
    pub backend_hint: Option<String>,
    pub extensions: Option<serde_json::Value>,
}

impl SandboxIR {
    /// IR minimale per la conformance suite. Non usare in produzione.
    pub fn default_for_test() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            image: "python:3.12-slim".into(),
            command: None,
            env: vec![],
            secret_env: vec![],
            cpu_millicores: 500,
            memory_mb: 256,
            disk_mb: 512,
            egress: EgressIR {
                mode: EgressMode::None,
                allow_hostnames: vec![],
                allow_ips: vec![],
                deny_by_default: true,
            },
            ttl_seconds: 60,
            timeout_ms: 30_000,
            working_dir: "/workspace".into(),
            labels: HashMap::new(),
            backend_hint: None,
            extensions: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressIR {
    pub mode: EgressMode,
    pub allow_hostnames: Vec<String>,
    pub allow_ips: Vec<String>,
    pub deny_by_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EgressMode { None, Proxy, Passthrough }
```

```rust
// crates/agentsandbox-sdk/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum BackendError {
    #[error("sandbox non trovata: {0}")]
    NotFound(String),

    #[error("backend non disponibile: {0}")]
    Unavailable(String),

    #[error("risorse insufficienti: {0}")]
    ResourceExhausted(String),

    #[error("operazione non supportata: {0}")]
    NotSupported(String),

    #[error("timeout dopo {0}ms")]
    Timeout(u64),

    #[error("configurazione non valida: {0}")]
    Configuration(String),

    #[error("errore interno: {0}")]
    Internal(String),
}
```

```rust
// crates/agentsandbox-sdk/src/backend.rs
use async_trait::async_trait;
use std::collections::HashMap;
use crate::{ir::SandboxIR, error::BackendError, BACKEND_TRAIT_VERSION};

#[derive(Debug, Clone)]
pub struct BackendDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub version: &'static str,
    pub trait_version: &'static str,
    pub capabilities: BackendCapabilities,
    /// JSON Schema delle extensions. None = extensions non supportate.
    pub extensions_schema: Option<&'static str>,
}

#[derive(Debug, Clone, Default)]
pub struct BackendCapabilities {
    pub network_isolation: bool,
    pub memory_hard_limit: bool,
    pub cpu_hard_limit: bool,
    pub persistent_storage: bool,
    pub self_contained: bool,
    pub isolation_level: IsolationLevel,
    pub supported_presets: Vec<&'static str>,
    pub rootless: bool,
    pub snapshot_restore: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum IsolationLevel {
    #[default]
    Process,
    Container,
    KernelSandbox,  // gVisor
    MicroVM,        // Firecracker
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
    pub resource_usage: Option<ResourceUsage>,
}

#[derive(Debug, Clone)]
pub struct ResourceUsage {
    pub cpu_user_ms: Option<u64>,
    pub memory_peak_mb: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SandboxStatus {
    pub sandbox_id: String,
    pub state: SandboxState,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub backend_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SandboxState {
    Creating,
    Running,
    Stopped,
    Failed(String),
    Expired,
}

/// Factory — crea istanze del backend dalla configurazione del daemon.
pub trait BackendFactory: Send + Sync {
    fn describe(&self) -> BackendDescriptor;
    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError>;
}

/// Il contratto principale. Ogni backend implementa questo trait.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError>;

    async fn exec(
        &self,
        handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError>;

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError>;

    /// Idempotente: destroy su sandbox già distrutta → Ok(())
    async fn destroy(&self, handle: &str) -> Result<(), BackendError>;

    async fn health_check(&self) -> Result<(), BackendError>;

    /// Validazione pre-creazione — fail fast prima di allocare risorse.
    /// Override per validare extensions e capability mismatch.
    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        if ir.extensions.is_some() {
            return Err(BackendError::NotSupported(
                "questo backend non supporta extensions".into()
            ));
        }
        Ok(())
    }

    // Opzionali — default: BackendError::NotSupported
    async fn upload_file(&self, handle: &str, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let _ = (handle, path, content);
        Err(BackendError::NotSupported("upload_file".into()))
    }

    async fn download_file(&self, handle: &str, path: &str) -> Result<Vec<u8>, BackendError> {
        let _ = (handle, path);
        Err(BackendError::NotSupported("download_file".into()))
    }

    async fn snapshot(&self, handle: &str) -> Result<String, BackendError> {
        let _ = handle;
        Err(BackendError::NotSupported("snapshot".into()))
    }

    async fn restore(&self, snapshot_id: &str, ir: &SandboxIR) -> Result<String, BackendError> {
        let _ = (snapshot_id, ir);
        Err(BackendError::NotSupported("restore".into()))
    }
}
```

### B.2 — Nuovo crate: `agentsandbox-conformance`

```toml
# crates/agentsandbox-conformance/Cargo.toml
[package]
name = "agentsandbox-conformance"
version = "0.1.0"

[dependencies]
agentsandbox-sdk = { path = "../agentsandbox-sdk" }
tokio = { version = "1", features = ["full"] }
```

```rust
// crates/agentsandbox-conformance/src/lib.rs
use agentsandbox_sdk::{
    backend::{SandboxBackend, SandboxState},
    ir::SandboxIR,
    error::BackendError,
};

pub struct ConformanceReport {
    pub results: Vec<(String, Result<(), String>)>,
}

impl ConformanceReport {
    pub fn new() -> Self { Self { results: vec![] } }

    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|(_, r)| r.is_ok())
    }

    pub fn print(&self) {
        for (name, result) in &self.results {
            match result {
                Ok(_)    => println!("  ✅ {}", name),
                Err(msg) => println!("  ❌ {} — {}", name, msg),
            }
        }
        let passed = self.results.iter().filter(|(_, r)| r.is_ok()).count();
        println!("\n  {}/{} test passati", passed, self.results.len());
    }
}

pub async fn run_all(backend: &dyn SandboxBackend) -> ConformanceReport {
    let mut r = ConformanceReport::new();
    let ir = SandboxIR::default_for_test();

    macro_rules! test {
        ($name:expr, $fut:expr) => {
            r.results.push(($name.into(), $fut.await.map_err(|e: String| e)));
        };
    }

    test!("health_check",              health_check(backend));
    test!("create_handle_nonempty",    create_handle(backend, &ir));
    test!("exec_stdout_marker",        exec_stdout(backend, &ir));
    test!("exec_stderr_captured",      exec_stderr(backend, &ir));
    test!("exec_nonzero_exit_code",    exec_nonzero(backend, &ir));
    test!("status_running",            status_running(backend, &ir));
    test!("destroy_cleans_up",         destroy(backend, &ir));
    test!("destroy_idempotent",        destroy_idempotent(backend, &ir));
    test!("concurrent_three",          concurrent(backend, &ir, 3));

    r
}

async fn health_check(b: &dyn SandboxBackend) -> Result<(), String> {
    b.health_check().await.map_err(|e| e.to_string())
}

async fn create_handle(b: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let h = b.create(ir).await.map_err(|e| e.to_string())?;
    if h.is_empty() { return Err("handle vuoto".into()); }
    b.destroy(&h).await.ok();
    Ok(())
}

async fn exec_stdout(b: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let h = b.create(ir).await.map_err(|e| e.to_string())?;
    let r = b.exec(&h, "echo 'agentsandbox-conformance-ok'", None)
        .await.map_err(|e| e.to_string())?;
    b.destroy(&h).await.ok();
    if !r.stdout.contains("agentsandbox-conformance-ok") {
        return Err(format!("marker non in stdout: {:?}", r.stdout));
    }
    if r.exit_code != 0 { return Err(format!("exit_code: {}", r.exit_code)); }
    Ok(())
}

async fn exec_stderr(b: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let h = b.create(ir).await.map_err(|e| e.to_string())?;
    let r = b.exec(&h, "echo 'stderr-marker' >&2", None)
        .await.map_err(|e| e.to_string())?;
    b.destroy(&h).await.ok();
    if !r.stderr.contains("stderr-marker") {
        return Err(format!("marker non in stderr: {:?}", r.stderr));
    }
    Ok(())
}

async fn exec_nonzero(b: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let h = b.create(ir).await.map_err(|e| e.to_string())?;
    let r = b.exec(&h, "exit 42", None).await.map_err(|e| e.to_string())?;
    b.destroy(&h).await.ok();
    if r.exit_code != 42 { return Err(format!("atteso 42, got {}", r.exit_code)); }
    Ok(())
}

async fn status_running(b: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let h = b.create(ir).await.map_err(|e| e.to_string())?;
    let s = b.status(&h).await.map_err(|e| e.to_string())?;
    b.destroy(&h).await.ok();
    if s.state != SandboxState::Running {
        return Err(format!("atteso Running, got {:?}", s.state));
    }
    Ok(())
}

async fn destroy(b: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let h = b.create(ir).await.map_err(|e| e.to_string())?;
    b.destroy(&h).await.map_err(|e| e.to_string())?;
    match b.status(&h).await {
        Err(BackendError::NotFound(_)) => Ok(()),
        Ok(s) if s.state == SandboxState::Stopped => Ok(()),
        Ok(s) => Err(format!("dopo destroy: {:?}", s.state)),
        Err(e) => Err(format!("dopo destroy errore inatteso: {}", e)),
    }
}

async fn destroy_idempotent(b: &dyn SandboxBackend, ir: &SandboxIR) -> Result<(), String> {
    let h = b.create(ir).await.map_err(|e| e.to_string())?;
    b.destroy(&h).await.map_err(|e| e.to_string())?;
    match b.destroy(&h).await {
        Ok(_) | Err(BackendError::NotFound(_)) => Ok(()),
        Err(e) => Err(format!("seconda destroy: {}", e)),
    }
}

async fn concurrent(b: &dyn SandboxBackend, ir: &SandboxIR, n: usize) -> Result<(), String> {
    // Crea n sandbox sequenzialmente (il trait non è Clone)
    let mut handles = vec![];
    for _ in 0..n {
        handles.push(b.create(ir).await.map_err(|e| e.to_string())?);
    }
    for h in handles {
        b.destroy(&h).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Macro che genera i #[tokio::test] per ogni test della suite.
#[macro_export]
macro_rules! run_conformance_suite {
    ($make_backend:expr) => {
        #[cfg(test)]
        mod conformance {
            use super::*;

            #[tokio::test]
            async fn full_suite() {
                let backend = ($make_backend)().await;
                let report = agentsandbox_conformance::run_all(backend.as_ref()).await;
                report.print();
                assert!(report.all_passed(), "conformance suite fallita — vedi output sopra");
            }
        }
    };
}
```

### B.3 — Refactor Docker adapter come plugin

Rinomina il crate esistente da `agentsandbox-docker` a `agentsandbox-backend-docker`
e adattalo al nuovo trait.

```toml
# crates/agentsandbox-backend-docker/Cargo.toml
[package]
name = "agentsandbox-backend-docker"
version = "0.1.0"

[dependencies]
agentsandbox-sdk = { path = "../agentsandbox-sdk" }
bollard    = "0.16"
async-trait = "0.1"
tokio      = { version = "1", features = ["full"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
tracing    = "1"
futures    = "0.3"

[dev-dependencies]
agentsandbox-conformance = { path = "../agentsandbox-conformance" }
tokio = { version = "1", features = ["full"] }
```

```rust
// crates/agentsandbox-backend-docker/src/factory.rs
use agentsandbox_sdk::backend::*;
use std::collections::HashMap;

pub struct DockerBackendFactory;

impl BackendFactory for DockerBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "docker",
            display_name: "Docker",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: false,
                isolation_level: IsolationLevel::Container,
                supported_presets: vec!["python", "node", "rust", "shell"],
                rootless: false,
                snapshot_restore: false,
            },
            extensions_schema: Some(include_str!("../schema/extensions.schema.json")),
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, agentsandbox_sdk::error::BackendError> {
        use agentsandbox_sdk::error::BackendError;
        let socket = config
            .get("socket")
            .map(|s| s.as_str())
            .unwrap_or("/var/run/docker.sock");

        let client = bollard::Docker::connect_with_unix(
            socket, 30, bollard::API_DEFAULT_VERSION,
        ).map_err(|e| BackendError::Unavailable(e.to_string()))?;

        Ok(Box::new(crate::DockerBackend { client }))
    }
}
```

```rust
// crates/agentsandbox-backend-docker/src/lib.rs
mod factory;
pub use factory::DockerBackendFactory;

use agentsandbox_sdk::{
    backend::*,
    ir::{SandboxIR, EgressMode},
    error::BackendError,
};
use async_trait::async_trait;
use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, StartContainerOptions,
        RemoveContainerOptions,
    },
    exec::{CreateExecOptions, StartExecResults},
    models::HostConfig,
};
use futures::StreamExt;

pub struct DockerBackend {
    pub(crate) client: Docker,
}

impl DockerBackend {
    fn container_name(sandbox_id: &str) -> String {
        format!("agentsandbox-{}", sandbox_id)
    }

    fn network_mode(ir: &SandboxIR) -> String {
        match ir.egress.mode {
            EgressMode::None => "none".into(),
            EgressMode::Proxy | EgressMode::Passthrough => {
                if ir.egress.mode == EgressMode::Passthrough {
                    tracing::warn!(
                        sandbox_id = %ir.id,
                        "egress mode=passthrough: nessun filtro di rete applicato"
                    );
                }
                "bridge".into()
            }
        }
    }

    fn parse_extensions(ir: &SandboxIR) -> Result<DockerExtensions, BackendError> {
        match &ir.extensions {
            None => Ok(DockerExtensions::default()),
            Some(raw) => {
                let section = raw.get("docker")
                    .cloned()
                    .unwrap_or_default();
                serde_json::from_value::<DockerExtensions>(section)
                    .map_err(|e| BackendError::Configuration(
                        format!("extensions.docker non valide: {}", e)
                    ))
            }
        }
    }
}

// Extensions schema (con deny_unknown_fields — errore su campi sconosciuti)
#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DockerExtensions {
    host_config: Option<DockerHostConfigExt>,
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DockerHostConfigExt {
    cap_add: Option<Vec<String>>,
    cap_drop: Option<Vec<String>>,
    security_opt: Option<Vec<String>>,
    privileged: Option<bool>,
    shm_size_mb: Option<u64>,
    sysctls: Option<std::collections::HashMap<String, String>>,
    ulimits: Option<Vec<DockerUlimit>>,
    devices: Option<Vec<DockerDevice>>,
    binds: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct DockerUlimit { name: String, soft: u64, hard: u64 }

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerDevice {
    path_on_host: String,
    path_in_container: String,
    cgroup_permissions: String,
}

#[async_trait]
impl SandboxBackend for DockerBackend {
    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        // Valida extensions prima di creare qualsiasi risorsa
        Self::parse_extensions(ir)?;
        Ok(())
    }

    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let ext = Self::parse_extensions(ir)?;

        let mut env: Vec<String> = ir.env.iter()
            .chain(ir.secret_env.iter())
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        let mut host_config = HostConfig {
            memory: Some((ir.memory_mb as i64) * 1024 * 1024),
            nano_cpus: Some((ir.cpu_millicores as i64) * 1_000_000),
            network_mode: Some(Self::network_mode(ir)),
            auto_remove: Some(false),
            ..Default::default()
        };

        // Applica extensions dopo i valori base
        if let Some(hc) = &ext.host_config {
            if hc.privileged == Some(true) {
                tracing::warn!(
                    sandbox_id = %ir.id,
                    "extensions: privileged=true — la sandbox ha accesso privilegiato all'host"
                );
            }
            host_config.cap_add = hc.cap_add.clone();
            host_config.cap_drop = hc.cap_drop.clone();
            host_config.security_opt = hc.security_opt.clone();
            host_config.privileged = hc.privileged;
            if let Some(mb) = hc.shm_size_mb {
                host_config.shm_size = Some((mb * 1024 * 1024) as i64);
            }
        }

        let config = Config {
            image: Some(ir.image.clone()),
            env: Some(env),
            working_dir: Some(ir.working_dir.clone()),
            host_config: Some(host_config),
            cmd: Some(vec!["sleep".into(), ir.ttl_seconds.to_string()]),
            ..Default::default()
        };

        let container = self.client
            .create_container(
                Some(CreateContainerOptions {
                    name: Self::container_name(&ir.id),
                    platform: None,
                }),
                config,
            )
            .await
            .map_err(|e| BackendError::Internal(e.to_string()))?;

        self.client
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| BackendError::Internal(e.to_string()))?;

        Ok(ir.id.clone())
    }

    async fn exec(
        &self,
        handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError> {
        let container = Self::container_name(handle);
        let start = std::time::Instant::now();

        let exec = self.client
            .create_exec(&container, CreateExecOptions {
                cmd: Some(vec!["sh", "-c", command]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            })
            .await
            .map_err(|e| BackendError::Internal(e.to_string()))?;

        let mut stdout = String::new();
        let mut stderr_buf = String::new();

        if let StartExecResults::Attached { mut output, .. } = self.client
            .start_exec(&exec.id, None)
            .await
            .map_err(|e| BackendError::Internal(e.to_string()))?
        {
            while let Some(chunk) = output.next().await {
                use bollard::container::LogOutput;
                match chunk.map_err(|e| BackendError::Internal(e.to_string()))? {
                    LogOutput::StdOut { message } => stdout.push_str(&String::from_utf8_lossy(&message)),
                    LogOutput::StdErr { message } => stderr_buf.push_str(&String::from_utf8_lossy(&message)),
                    _ => {}
                }
            }
        }

        let inspect = self.client.inspect_exec(&exec.id).await
            .map_err(|e| BackendError::Internal(e.to_string()))?;

        Ok(ExecResult {
            stdout,
            stderr: stderr_buf,
            exit_code: inspect.exit_code.unwrap_or(-1),
            duration_ms: start.elapsed().as_millis() as u64,
            resource_usage: None,
        })
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        let container = Self::container_name(handle);
        let info = self.client
            .inspect_container(&container, None)
            .await
            .map_err(|_| BackendError::NotFound(handle.to_string()))?;

        let state = match info.state.and_then(|s| s.running) {
            Some(true)  => SandboxState::Running,
            Some(false) => SandboxState::Stopped,
            None        => SandboxState::Failed("stato sconosciuto".into()),
        };

        let now = chrono::Utc::now();
        Ok(SandboxStatus {
            sandbox_id: handle.to_string(),
            state,
            created_at: now,
            expires_at: now,
            backend_id: "docker".into(),
        })
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        let container = Self::container_name(handle);
        match self.client
            .remove_container(&container, Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }))
            .await
        {
            Ok(_) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(()),
            Err(e) => Err(BackendError::Internal(e.to_string())),
        }
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        self.client.ping().await
            .map_err(|e| BackendError::Unavailable(e.to_string()))
    }
}
```

```json
// crates/agentsandbox-backend-docker/schema/extensions.schema.json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Docker Backend Extensions",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "docker": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "hostConfig": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "capAdd":      { "type": "array", "items": { "type": "string" } },
            "capDrop":     { "type": "array", "items": { "type": "string" } },
            "securityOpt": { "type": "array", "items": { "type": "string" } },
            "privileged":  { "type": "boolean" },
            "shmSizeMb":   { "type": "integer", "minimum": 1 },
            "sysctls":     { "type": "object", "additionalProperties": { "type": "string" } },
            "binds":       { "type": "array", "items": { "type": "string" } },
            "ulimits": {
              "type": "array",
              "items": {
                "type": "object",
                "required": ["name", "soft", "hard"],
                "additionalProperties": false,
                "properties": {
                  "name": { "type": "string" },
                  "soft": { "type": "integer" },
                  "hard": { "type": "integer" }
                }
              }
            },
            "devices": {
              "type": "array",
              "items": {
                "type": "object",
                "required": ["pathOnHost", "pathInContainer", "cgroupPermissions"],
                "additionalProperties": false,
                "properties": {
                  "pathOnHost":        { "type": "string" },
                  "pathInContainer":   { "type": "string" },
                  "cgroupPermissions": { "type": "string" }
                }
              }
            }
          }
        }
      }
    }
  }
}
```

```rust
// crates/agentsandbox-backend-docker/tests/conformance.rs
use agentsandbox_backend_docker::DockerBackendFactory;
use agentsandbox_sdk::backend::BackendFactory;
use std::collections::HashMap;

async fn make_backend() -> Box<dyn agentsandbox_sdk::backend::SandboxBackend> {
    DockerBackendFactory.create(&HashMap::new())
        .expect("Docker deve essere disponibile per i test di conformance")
}

agentsandbox_conformance::run_conformance_suite!(make_backend);
```

### B.4 — Backend Registry nel daemon

```rust
// crates/agentsandbox-daemon/src/registry.rs
use agentsandbox_sdk::backend::{BackendFactory, BackendDescriptor, SandboxBackend};
use agentsandbox_sdk::ir::SandboxIR;
use std::collections::HashMap;
use std::sync::Arc;

pub struct BackendRegistry {
    descriptors: HashMap<String, BackendDescriptor>,
    instances: HashMap<String, Arc<dyn SandboxBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
            instances: HashMap::new(),
        }
    }

    pub fn register(&mut self, factory: &dyn BackendFactory) {
        let desc = factory.describe();
        tracing::info!(
            backend_id = %desc.id,
            version = %desc.version,
            trait_version = %desc.trait_version,
            "backend registrato"
        );
        self.descriptors.insert(desc.id.to_string(), desc);
    }

    pub async fn initialize(
        &mut self,
        factory: &dyn BackendFactory,
        config: &HashMap<String, String>,
    ) {
        let desc = factory.describe();
        match factory.create(config) {
            Ok(backend) => {
                match backend.health_check().await {
                    Ok(_) => {
                        tracing::info!(backend_id = %desc.id, "backend healthy");
                        self.instances.insert(desc.id.to_string(), Arc::from(backend));
                    }
                    Err(e) => {
                        tracing::warn!(
                            backend_id = %desc.id,
                            error = %e,
                            "backend health check fallito — non disponibile"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    backend_id = %desc.id,
                    error = %e,
                    "backend inizializzazione fallita — non disponibile"
                );
            }
        }
    }

    pub fn select(&self, ir: &SandboxIR) -> Result<Arc<dyn SandboxBackend>, RegistryError> {
        // Regola 1: backend hint esplicito dalla spec
        if let Some(hint) = &ir.backend_hint {
            return self.instances
                .get(hint)
                .cloned()
                .ok_or_else(|| RegistryError::RequestedUnavailable(hint.clone()));
        }

        // Regola 2: primo backend disponibile
        // (future versioni useranno le capabilities per il matching)
        self.instances.values().next().cloned()
            .ok_or(RegistryError::NoneAvailable)
    }

    pub fn list_available(&self) -> Vec<&BackendDescriptor> {
        self.descriptors.values()
            .filter(|d| self.instances.contains_key(d.id))
            .collect()
    }

    pub fn get_extensions_schema(&self, backend_id: &str) -> Option<&'static str> {
        self.descriptors.get(backend_id)
            .and_then(|d| d.extensions_schema)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("nessun backend disponibile")]
    NoneAvailable,
    #[error("backend '{0}' richiesto ma non disponibile")]
    RequestedUnavailable(String),
}
```

```rust
// crates/agentsandbox-daemon/src/main.rs — aggiorna startup
use agentsandbox_backend_docker::DockerBackendFactory;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = std::env::args().nth(1)
        .unwrap_or_else(|| "agentsandbox.toml".into());
    let config = config::load_config(&config_path)?;

    // Logging strutturato
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(&config.daemon.log_level);
    if config.daemon.log_format == "json" {
        subscriber.json().init();
    } else {
        subscriber.init();
    }

    let db = sqlx::SqlitePool::connect(&config.database.url).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let mut registry = BackendRegistry::new();

    // Registra backend in base a backends.enabled nel config
    for backend_id in &config.backends.enabled {
        let backend_config = config.backends.configs
            .get(backend_id)
            .cloned()
            .unwrap_or_default();

        match backend_id.as_str() {
            "docker" => {
                let factory = DockerBackendFactory;
                registry.register(&factory);
                registry.initialize(&factory, &backend_config).await;
            }
            // Fase C: "podman" => { ... }
            // Fase D: "gvisor" => { ... }
            // Fase E: "firecracker" => { ... }
            other => {
                tracing::warn!("backend '{}' non riconosciuto — ignorato", other);
            }
        }
    }

    if registry.list_available().is_empty() {
        anyhow::bail!("nessun backend disponibile — controlla la configurazione");
    }

    // ... resto del setup axum invariato, passa registry nello AppState
    Ok(())
}
```

### B.5 — Aggiorna Cargo.toml workspace

```toml
# Cargo.toml (root) — aggiungi i nuovi crate al workspace
[workspace]
members = [
    "crates/agentsandbox-sdk",
    "crates/agentsandbox-conformance",
    "crates/agentsandbox-core",
    "crates/agentsandbox-daemon",
    "crates/agentsandbox-backend-docker",
    # Aggiunti nelle fasi successive:
    # "crates/agentsandbox-backend-podman",
    # "crates/agentsandbox-backend-gvisor",
    # "crates/agentsandbox-backend-firecracker",
    # "crates/agentsandbox-proxy",
]
```

### B.6 — Aggiorna `agentsandbox-core` per dipendere dall'SDK

```toml
# crates/agentsandbox-core/Cargo.toml — aggiungi:
[dependencies]
agentsandbox-sdk = { path = "../agentsandbox-sdk" }
```

```rust
// crates/agentsandbox-core/src/compile.rs — usa SandboxIR dall'SDK
use agentsandbox_sdk::ir::{SandboxIR, EgressIR, EgressMode as IREgressMode};
// Rimuovi il SandboxIR locale se ne avevi uno nel core
```

### B.7 — Criteri di completamento Fase B

- [ ] `cargo check --workspace` passa
- [ ] `cargo test -p agentsandbox-sdk` — nessun errore
- [ ] `cargo test -p agentsandbox-conformance` — compile ok (nessun backend specifico)
- [ ] `cargo test -p agentsandbox-backend-docker conformance` — tutti i test passano
- [ ] `docker ps` dopo i test — zero container `agentsandbox-*` rimasti vivi
- [ ] Il daemon si avvia e `GET /v1/backends` ritorna Docker
- [ ] `cargo check -p agentsandbox-sdk` NON dipende da bollard, sqlx, axum
- [ ] Il native handle (container ID) non è mai nella response HTTP

---

## FASE C — Backend Podman
**Stima: 1 giorno**

Podman è compatibile con l'API Docker. Riusa il Docker backend con socket diverso.

```toml
# crates/agentsandbox-backend-podman/Cargo.toml
[package]
name = "agentsandbox-backend-podman"
version = "0.1.0"

[dependencies]
agentsandbox-sdk            = { path = "../agentsandbox-sdk" }
agentsandbox-backend-docker = { path = "../agentsandbox-backend-docker" }
async-trait = "0.1"
tracing = "1"

[dev-dependencies]
agentsandbox-conformance = { path = "../agentsandbox-conformance" }
tokio = { version = "1", features = ["full"] }
```

```rust
// crates/agentsandbox-backend-podman/src/lib.rs
use agentsandbox_sdk::backend::*;
use agentsandbox_sdk::error::BackendError;
use std::collections::HashMap;

pub struct PodmanBackendFactory;

impl BackendFactory for PodmanBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "podman",
            display_name: "Podman (rootless)",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: false,
                isolation_level: IsolationLevel::Container,
                supported_presets: vec!["python", "node", "rust", "shell"],
                rootless: true,   // differenza principale vs Docker
                snapshot_restore: false,
            },
            // Podman accetta le stesse extensions Docker (API compatibile)
            extensions_schema: agentsandbox_backend_docker::DockerBackendFactory
                .describe().extensions_schema,
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        // Podman espone un socket compatibile Docker.
        // Default rootless: /run/user/{uid}/podman/podman.sock
        let socket = config.get("socket")
            .cloned()
            .unwrap_or_else(default_podman_socket);

        // Riusa DockerBackendFactory con il socket Podman
        agentsandbox_backend_docker::DockerBackendFactory
            .create(&HashMap::from([("socket".into(), socket)]))
    }
}

fn default_podman_socket() -> String {
    // Rootless default
    if let Ok(uid) = std::env::var("UID") {
        return format!("/run/user/{}/podman/podman.sock", uid);
    }
    // Fallback rootful
    "/run/podman/podman.sock".into()
}
```

```rust
// crates/agentsandbox-backend-podman/tests/conformance.rs
use agentsandbox_backend_podman::PodmanBackendFactory;
use agentsandbox_sdk::backend::BackendFactory;
use std::collections::HashMap;

async fn make_backend() -> Box<dyn agentsandbox_sdk::backend::SandboxBackend> {
    PodmanBackendFactory.create(&HashMap::new())
        .expect("Podman deve essere disponibile")
}

agentsandbox_conformance::run_conformance_suite!(make_backend);
```

**Aggiungi al daemon `main.rs`:**
```rust
"podman" => {
    let factory = agentsandbox_backend_podman::PodmanBackendFactory;
    registry.register(&factory);
    registry.initialize(&factory, &backend_config).await;
}
```

### Criteri di completamento Fase C

- [ ] `cargo test -p agentsandbox-backend-podman conformance` con Podman running — passa
- [ ] `health_check()` ritorna `Unavailable` con messaggio se Podman non è installato
- [ ] `scheduling.backend: podman` nella spec usa Podman

---

## FASE D — Backend gVisor
**Stima: 2 giorni | Solo Linux**

gVisor intercetta le syscall a livello userspace — isolamento più forte di Docker standard,
senza richiedere KVM.

```toml
# crates/agentsandbox-backend-gvisor/Cargo.toml
[package]
name = "agentsandbox-backend-gvisor"
version = "0.1.0"

[dependencies]
agentsandbox-sdk            = { path = "../agentsandbox-sdk" }
agentsandbox-backend-docker = { path = "../agentsandbox-backend-docker" }
async-trait = "0.1"
bollard = "0.16"
tracing = "1"

[dev-dependencies]
agentsandbox-conformance = { path = "../agentsandbox-conformance" }
tokio = { version = "1", features = ["full"] }
```

```rust
// crates/agentsandbox-backend-gvisor/src/lib.rs
use agentsandbox_sdk::backend::*;
use agentsandbox_sdk::error::BackendError;
use std::collections::HashMap;

pub struct GVisorBackendFactory;

impl BackendFactory for GVisorBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "gvisor",
            display_name: "gVisor (runsc)",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                self_contained: false,
                isolation_level: IsolationLevel::KernelSandbox,
                supported_presets: vec!["python", "node", "shell"],
                rootless: false,
                snapshot_restore: false,
                ..Default::default()
            },
            extensions_schema: Some(include_str!("../schema/extensions.schema.json")),
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let socket = config.get("socket").cloned()
            .unwrap_or_else(|| "/var/run/docker.sock".into());
        let runtime = config.get("runtime").cloned()
            .unwrap_or_else(|| "runsc".into());

        let client = bollard::Docker::connect_with_unix(
            &socket, 30, bollard::API_DEFAULT_VERSION,
        ).map_err(|e| BackendError::Unavailable(e.to_string()))?;

        Ok(Box::new(GVisorBackend { client, runtime }))
    }
}

pub struct GVisorBackend {
    client: bollard::Docker,
    runtime: String,
}

// GVisorBackend è identico a DockerBackend con una differenza:
// HostConfig.runtime = Some(self.runtime.clone())
// Implementa il trait delegando a DockerBackend con override su create().
#[async_trait::async_trait]
impl SandboxBackend for GVisorBackend {
    async fn health_check(&self) -> Result<(), BackendError> {
        // 1. Docker raggiungibile?
        self.client.ping().await
            .map_err(|e| BackendError::Unavailable(format!("Docker: {}", e)))?;

        // 2. runtime runsc disponibile?
        let runtimes = self.client.info().await
            .map_err(|e| BackendError::Unavailable(e.to_string()))?
            .runtimes
            .unwrap_or_default();

        if !runtimes.contains_key(&self.runtime) {
            return Err(BackendError::Unavailable(format!(
                "runtime '{}' non trovato in Docker. \
                 Installa gVisor e configura il runtime: \
                 https://gvisor.dev/docs/user_guide/install/",
                self.runtime
            )));
        }
        Ok(())
    }

    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
        use bollard::models::HostConfig;
        use agentsandbox_sdk::ir::EgressMode;

        let env: Vec<String> = ir.env.iter()
            .chain(ir.secret_env.iter())
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        let host_config = HostConfig {
            memory: Some((ir.memory_mb as i64) * 1024 * 1024),
            nano_cpus: Some((ir.cpu_millicores as i64) * 1_000_000),
            network_mode: Some(match ir.egress.mode {
                EgressMode::None => "none".into(),
                _ => "bridge".into(),
            }),
            runtime: Some(self.runtime.clone()),  // ← unica differenza da Docker
            auto_remove: Some(false),
            ..Default::default()
        };

        let container_name = format!("agentsandbox-{}", ir.id);
        let container = self.client
            .create_container(
                Some(CreateContainerOptions { name: &container_name, platform: None }),
                Config {
                    image: Some(ir.image.clone()),
                    env: Some(env),
                    working_dir: Some(ir.working_dir.clone()),
                    host_config: Some(host_config),
                    cmd: Some(vec!["sleep".into(), ir.ttl_seconds.to_string()]),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| BackendError::Internal(e.to_string()))?;

        self.client
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| BackendError::Internal(e.to_string()))?;

        Ok(ir.id.clone())
    }

    // exec, status, destroy: delega a DockerBackend con stesso container naming
    async fn exec(&self, handle: &str, command: &str, timeout_ms: Option<u64>) -> Result<ExecResult, BackendError> {
        // Stessa implementazione di DockerBackend.exec
        // Copia o estrai in helper condiviso
        todo!("implementa uguale a DockerBackend con container_name = agentsandbox-{handle}")
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        todo!()
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        use bollard::container::RemoveContainerOptions;
        let name = format!("agentsandbox-{}", handle);
        match self.client.remove_container(&name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await {
            Ok(_) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(()),
            Err(e) => Err(BackendError::Internal(e.to_string())),
        }
    }
}
```

```json
// crates/agentsandbox-backend-gvisor/schema/extensions.schema.json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "gVisor Extensions",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "gvisor": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "platform": {
          "type": "string",
          "enum": ["systrap", "kvm", "ptrace"],
          "description": "systrap non richiede KVM (default consigliato)"
        },
        "network": {
          "type": "string",
          "enum": ["sandbox", "host", "none"]
        }
      }
    }
  }
}
```

**Nota su exec/status:** il codice di `exec`, `status` è identico al Docker adapter.
Considera di estrarre l'implementazione comune in un helper interno in `agentsandbox-backend-docker`
e riesportarlo, oppure copia il codice (accettabile per ora, refactor dopo Firecracker).

### Criteri di completamento Fase D

- [ ] Conformance suite passa su Linux con gVisor installato
- [ ] `health_check()` ritorna messaggio chiaro se `runsc` non è nel runtime Docker
- [ ] `scheduling.backend: gvisor` funziona nella spec
- [ ] La documentazione `docs/backends/gvisor.md` elenca i sistemi non supportati

---

## FASE E — Backend Firecracker
**Stima: 5-7 giorni | Solo Linux + KVM**

Firecracker è il backend di isolamento più forte. Richiede KVM.
Su sistemi senza KVM, `health_check()` ritorna `Unavailable` con messaggio esplicito
e il daemon continua con gli altri backend.

### E.1 — Prerequisiti

```
# Verifica prerequisiti:
ls /dev/kvm                    # deve esistere
firecracker --version          # deve essere installato
jailer --version               # stesso release di firecracker
```

### E.2 — Architettura

```
Daemon → REST API Unix socket → Firecracker process → vsock → guest agent
```

Ogni sandbox è una VM Firecracker separata. Il guest contiene un agent minimale
che ascolta su vsock e esegue i comandi ricevuti.

### E.3 — Guest agent (binary minimale nel rootfs)

```rust
// crates/agentsandbox-guest-agent/src/main.rs
// Compilato per linux/musl, incluso nel rootfs delle VM Firecracker.
// Dimensione target: < 2MB stripped.

use std::process::Command;
use std::io::{BufRead, BufReader, Write};

#[derive(serde::Deserialize)]
struct ExecRequest {
    command: String,
    timeout_ms: Option<u64>,
}

#[derive(serde::Serialize)]
struct ExecResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
    duration_ms: u64,
}

fn main() {
    // Ascolta su vsock CID=3, port=1234
    // Ogni connessione = un exec request
    let listener = vsock::VsockListener::bind(vsock::VsockAddr::new(
        vsock::VMADDR_CID_ANY, 1234
    )).expect("vsock bind fallito");

    for stream in listener.incoming() {
        if let Ok(mut stream) = stream {
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).ok();

            let req: ExecRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let start = std::time::Instant::now();
            let output = Command::new("sh")
                .arg("-c")
                .arg(&req.command)
                .output()
                .unwrap_or_default();

            let resp = ExecResponse {
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
                exit_code: output.status.code().unwrap_or(-1),
                duration_ms: start.elapsed().as_millis() as u64,
            };

            let _ = stream.write_all(serde_json::to_string(&resp).unwrap().as_bytes());
            let _ = stream.write_all(b"\n");
        }
    }
}
```

### E.4 — Factory e Backend

```toml
# crates/agentsandbox-backend-firecracker/Cargo.toml
[dependencies]
agentsandbox-sdk = { path = "../agentsandbox-sdk" }
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "1"
reqwest = { version = "0.11", features = ["json"] }  # per API REST Firecracker
vsock = "0.3"                                          # per comunicazione con guest
uuid = { version = "1", features = ["v4"] }
```

```rust
// crates/agentsandbox-backend-firecracker/src/lib.rs
use agentsandbox_sdk::{backend::*, error::BackendError, ir::SandboxIR};

pub struct FirecrackerBackendFactory;

impl BackendFactory for FirecrackerBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "firecracker",
            display_name: "Firecracker MicroVM",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                self_contained: false,
                isolation_level: IsolationLevel::MicroVM,
                supported_presets: vec!["python", "node", "shell"],
                rootless: false,
                snapshot_restore: true,  // Firecracker supporta snapshot
            },
            extensions_schema: Some(include_str!("../schema/extensions.schema.json")),
        }
    }

    fn create(&self, config: &HashMap<String, String>) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let binary = config.get("binary_path")
            .ok_or_else(|| BackendError::Configuration("binary_path richiesto".into()))?;
        let kernel = config.get("kernel_image")
            .ok_or_else(|| BackendError::Configuration("kernel_image richiesto".into()))?;
        let rootfs_dir = config.get("rootfs_dir")
            .ok_or_else(|| BackendError::Configuration("rootfs_dir richiesto".into()))?;

        Ok(Box::new(FirecrackerBackend {
            binary_path: binary.clone(),
            kernel_image: kernel.clone(),
            rootfs_dir: rootfs_dir.clone(),
            jailer_path: config.get("jailer_path").cloned(),
        }))
    }
}

pub struct FirecrackerBackend {
    binary_path: String,
    kernel_image: String,
    rootfs_dir: String,
    jailer_path: Option<String>,
}

#[async_trait::async_trait]
impl SandboxBackend for FirecrackerBackend {
    async fn health_check(&self) -> Result<(), BackendError> {
        // 1. KVM disponibile?
        if !std::path::Path::new("/dev/kvm").exists() {
            return Err(BackendError::Unavailable(
                "/dev/kvm non trovato. Firecracker richiede KVM. \
                 Non supportato su macOS, VPS senza nested virtualization, WSL2."
                    .into()
            ));
        }

        // 2. Binary Firecracker presente?
        if !std::path::Path::new(&self.binary_path).exists() {
            return Err(BackendError::Unavailable(
                format!("firecracker binary non trovato: {}", self.binary_path)
            ));
        }

        // 3. kernel image presente?
        if !std::path::Path::new(&self.kernel_image).exists() {
            return Err(BackendError::Unavailable(
                format!("kernel image non trovata: {}", self.kernel_image)
            ));
        }

        Ok(())
    }

    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let vm_id = ir.id.clone();
        let socket_path = format!("/tmp/agentsandbox-fc-{}.sock", vm_id);

        // Avvia processo Firecracker
        let mut cmd = tokio::process::Command::new(&self.binary_path);
        cmd.arg("--api-sock").arg(&socket_path)
           .arg("--id").arg(&vm_id)
           .stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::null());

        let _child = cmd.spawn()
            .map_err(|e| BackendError::Internal(format!("avvio firecracker: {}", e)))?;

        // Aspetta che il socket sia disponibile (max 2 secondi)
        for _ in 0..20 {
            if std::path::Path::new(&socket_path).exists() { break; }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // Configura la VM via API REST
        self.configure_vm(&socket_path, ir).await?;
        self.start_vm(&socket_path).await?;

        Ok(vm_id)
    }

    async fn exec(&self, handle: &str, command: &str, timeout_ms: Option<u64>) -> Result<ExecResult, BackendError> {
        // Comunicazione via vsock con il guest agent
        let start = std::time::Instant::now();
        // TODO: implementa la comunicazione vsock
        // 1. Connetti a vsock del guest (CID dinamico assegnato a ogni VM)
        // 2. Invia JSON: {"command": command, "timeout_ms": timeout_ms}
        // 3. Ricevi JSON: {stdout, stderr, exit_code}
        todo!("implementa comunicazione vsock con guest agent")
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        let socket_path = format!("/tmp/agentsandbox-fc-{}.sock", handle);
        // Chiama GET /machine-config sull'API Firecracker per verificare stato
        todo!()
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        let socket_path = format!("/tmp/agentsandbox-fc-{}.sock", handle);
        // 1. PUT /actions {"action_type": "SendCtrlAltDel"} — graceful shutdown
        // 2. Kill processo se non risponde entro 2 secondi
        // 3. Rimuovi socket e file temporanei
        todo!()
    }
}

impl FirecrackerBackend {
    async fn configure_vm(&self, socket: &str, ir: &SandboxIR) -> Result<(), BackendError> {
        // Configura kernel, rootfs, vCPU, memoria via API REST Firecracker
        // Endpoint: PUT /machine-config, PUT /boot-source, PUT /drives/rootfs
        todo!()
    }

    async fn start_vm(&self, socket: &str) -> Result<(), BackendError> {
        // PUT /actions {"action_type": "InstanceStart"}
        todo!()
    }
}
```

### Criteri di completamento Fase E

- [ ] `health_check()` ritorna Unavailable con messaggio chiaro su macOS e senza KVM
- [ ] Conformance suite passa su Linux con KVM
- [ ] La VM si avvia in < 500ms dalla chiamata a `create()`
- [ ] `destroy()` non lascia socket, file temporanei o processi orfani
- [ ] `docs/backends/firecracker.md` con prerequisiti e troubleshooting

---

## FASE F — Egress proxy reale
**Stima: 2-3 giorni | Crate: `agentsandbox-proxy`**

Sostituisce il warning placeholder dell'egress con un proxy SOCKS5 reale.

```toml
# crates/agentsandbox-proxy/Cargo.toml
[package]
name = "agentsandbox-proxy"
version = "0.1.0"

[dependencies]
tokio = { version = "1", features = ["full", "net"] }
tracing = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

```rust
// crates/agentsandbox-proxy/src/lib.rs

use std::collections::HashSet;
use std::net::IpAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct EgressProxy {
    allowed_hostnames: HashSet<String>,
    allowed_ips: HashSet<IpAddr>,
    bind_addr: String,
    sandbox_id: String,
}

impl EgressProxy {
    /// Crea e avvia il proxy. Risolve gli hostname a startup (una volta sola).
    pub async fn start(
        sandbox_id: String,
        allow_hostnames: Vec<String>,
        port: u16,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        let mut allowed_ips = HashSet::new();

        // Risolvi hostname → IP a startup
        for host in &allow_hostnames {
            match tokio::net::lookup_host(format!("{}:443", host)).await {
                Ok(addrs) => {
                    for addr in addrs {
                        allowed_ips.insert(addr.ip());
                    }
                    tracing::debug!(sandbox_id = %sandbox_id, host = %host, "hostname risolto");
                }
                Err(e) => {
                    tracing::warn!(
                        sandbox_id = %sandbox_id,
                        host = %host,
                        error = %e,
                        "hostname non risolvibile — escluso dalla allowlist"
                    );
                }
            }
        }

        let proxy = Self {
            allowed_hostnames: allow_hostnames.into_iter().collect(),
            allowed_ips,
            bind_addr: format!("127.0.0.1:{}", port),
            sandbox_id,
        };

        let handle = tokio::spawn(async move {
            if let Err(e) = proxy.run().await {
                tracing::error!("proxy error: {}", e);
            }
        });

        Ok(handle)
    }

    async fn run(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!(
            sandbox_id = %self.sandbox_id,
            addr = %self.bind_addr,
            "proxy egress avviato"
        );

        loop {
            let (stream, peer) = listener.accept().await?;
            let proxy = EgressProxy {
                allowed_hostnames: self.allowed_hostnames.clone(),
                allowed_ips: self.allowed_ips.clone(),
                bind_addr: self.bind_addr.clone(),
                sandbox_id: self.sandbox_id.clone(),
            };
            tokio::spawn(async move {
                if let Err(e) = proxy.handle_socks5(stream).await {
                    tracing::debug!("proxy connection error: {}", e);
                }
            });
        }
    }

    async fn handle_socks5(&self, mut client: TcpStream) -> anyhow::Result<()> {
        // RFC 1928 — SOCKS5 handshake minimale (no auth)

        // 1. Greeting: VER NMETHODS METHODS
        let mut buf = [0u8; 2];
        client.read_exact(&mut buf).await?;
        if buf[0] != 5 { anyhow::bail!("non è SOCKS5"); }
        let nmethods = buf[1] as usize;
        let mut methods = vec![0u8; nmethods];
        client.read_exact(&mut methods).await?;

        // Risposta: VER METHOD (0x00 = no auth)
        client.write_all(&[5, 0]).await?;

        // 2. Request: VER CMD RSV ATYP DST.ADDR DST.PORT
        let mut req = [0u8; 4];
        client.read_exact(&mut req).await?;
        if req[1] != 1 { // CMD=CONNECT
            client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            anyhow::bail!("solo CONNECT supportato");
        }

        let (hostname, port) = match req[3] {
            1 => { // IPv4
                let mut ip = [0u8; 4];
                client.read_exact(&mut ip).await?;
                let mut port_buf = [0u8; 2];
                client.read_exact(&mut port_buf).await?;
                let addr = IpAddr::V4(std::net::Ipv4Addr::from(ip));
                (addr.to_string(), u16::from_be_bytes(port_buf))
            }
            3 => { // Domain
                let len = client.read_u8().await? as usize;
                let mut host_buf = vec![0u8; len];
                client.read_exact(&mut host_buf).await?;
                let host = String::from_utf8(host_buf)?;
                let mut port_buf = [0u8; 2];
                client.read_exact(&mut port_buf).await?;
                (host, u16::from_be_bytes(port_buf))
            }
            4 => { // IPv6
                let mut ip = [0u8; 16];
                client.read_exact(&mut ip).await?;
                let mut port_buf = [0u8; 2];
                client.read_exact(&mut port_buf).await?;
                let addr = IpAddr::V6(std::net::Ipv6Addr::from(ip));
                (addr.to_string(), u16::from_be_bytes(port_buf))
            }
            _ => anyhow::bail!("ATYP non supportato"),
        };

        // 3. Verifica allowlist
        if !self.is_allowed(&hostname) {
            tracing::info!(
                sandbox_id = %self.sandbox_id,
                hostname = %hostname,
                "connessione egress negata"
            );
            // SOCKS5 connection refused
            client.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }

        // 4. Connessione upstream
        let upstream = TcpStream::connect(format!("{}:{}", hostname, port)).await
            .map_err(|e| { anyhow::anyhow!("upstream connect: {}", e) })?;

        // Risposta successo
        client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;

        tracing::debug!(
            sandbox_id = %self.sandbox_id,
            hostname = %hostname,
            "connessione egress consentita"
        );

        // 5. Relay bidirezionale
        let (mut cr, mut cw) = client.into_split();
        let (mut ur, mut uw) = upstream.into_split();
        tokio::select! {
            _ = tokio::io::copy(&mut cr, &mut uw) => {}
            _ = tokio::io::copy(&mut ur, &mut cw) => {}
        }
        Ok(())
    }

    fn is_allowed(&self, hostname: &str) -> bool {
        // Hostname diretto
        if self.allowed_hostnames.contains(hostname) { return true; }
        // IP già risolto
        if let Ok(ip) = hostname.parse::<IpAddr>() {
            return self.allowed_ips.contains(&ip);
        }
        false
    }
}
```

**Integrazione nel Docker backend:** quando `egress.mode == Proxy`:
1. Alloca una porta libera sul loopback
2. Avvia `EgressProxy::start()` con la porta e gli hostname
3. Configura il container con `HTTP_PROXY=socks5://host-gateway:{porta}` e `HTTPS_PROXY=...`
4. Tieni il `JoinHandle` finché la sandbox non viene distrutta

### Criteri di completamento Fase F

- [ ] `network.egress.mode: proxy` con `allow: ["pypi.org"]` — pip install funziona
- [ ] Connessione a host non in allowlist — connessione rifiutata con log
- [ ] `network.egress.mode: none` — nessun accesso rete (verifica con curl)
- [ ] Il proxy termina automaticamente quando la sandbox viene distrutta

---

## FASE G — Multi-tenancy e autenticazione
**Stima: 2 giorni**

### G.1 — Migrazione SQLite

```sql
-- migrations/002_multitenancy.sql

CREATE TABLE tenants (
    id               TEXT PRIMARY KEY,
    api_key_hash     TEXT NOT NULL,
    quota_hourly     INTEGER NOT NULL DEFAULT 100,
    quota_concurrent INTEGER NOT NULL DEFAULT 10,
    enabled          INTEGER NOT NULL DEFAULT 1,
    created_at       TEXT NOT NULL
);

ALTER TABLE sandboxes ADD COLUMN tenant_id TEXT;

CREATE TABLE rate_limit_windows (
    tenant_id    TEXT NOT NULL,
    window_start TEXT NOT NULL,
    count        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, window_start)
);
```

### G.2 — Middleware autenticazione

```rust
// crates/agentsandbox-daemon/src/middleware/auth.rs
use axum::{extract::State, http::Request, middleware::Next, response::Response};
use std::sync::Arc;

pub async fn auth_middleware<B>(
    State(state): State<Arc<AppState>>,
    mut request: Request<B>,
    next: Next<B>,
) -> Result<Response, ApiError> {
    match state.config.auth.mode {
        AuthMode::SingleUser => {
            // In single_user mode, accetta solo richieste da 127.0.0.1
            // (già garantito dal binding sul loopback — nessun check aggiuntivo)
            Ok(next.run(request).await)
        }
        AuthMode::ApiKey => {
            let key = request.headers()
                .get("X-API-Key")
                .and_then(|v| v.to_str().ok())
                .ok_or(ApiError::unauthorized("X-API-Key richiesta"))?;

            let tenant = state.db.verify_api_key(key).await
                .map_err(|_| ApiError::unauthorized("API key non valida"))?;

            request.extensions_mut().insert(tenant);
            Ok(next.run(request).await)
        }
    }
}
```

### Criteri di completamento Fase G

- [ ] `auth.mode: single_user` — daemon funziona senza API key (default)
- [ ] `auth.mode: api_key` — richieste senza `X-API-Key` ritornano 401
- [ ] La migrazione SQLite applica senza errori su DB esistenti
- [ ] La quota oraria viene applicata (test: supera il limite, verifica 429)

---

## FASE H — Osservabilità
**Stima: 1-2 giorni**

### H.1 — Metriche Prometheus

```rust
// crates/agentsandbox-daemon/src/metrics.rs
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct Metrics {
    pub sandboxes_created:  AtomicU64,
    pub sandboxes_active:   AtomicU64,
    pub sandboxes_expired:  AtomicU64,
    pub exec_total:         AtomicU64,
    pub egress_allowed:     AtomicU64,
    pub egress_denied:      AtomicU64,
    pub backend_errors:     AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sandboxes_created: AtomicU64::new(0),
            sandboxes_active:  AtomicU64::new(0),
            sandboxes_expired: AtomicU64::new(0),
            exec_total:        AtomicU64::new(0),
            egress_allowed:    AtomicU64::new(0),
            egress_denied:     AtomicU64::new(0),
            backend_errors:    AtomicU64::new(0),
        })
    }

    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP agentsandbox_sandboxes_created_total Sandbox create\n\
             agentsandbox_sandboxes_created_total {}\n\
             # HELP agentsandbox_sandboxes_active Sandbox attive\n\
             agentsandbox_sandboxes_active {}\n\
             # HELP agentsandbox_exec_total Exec totali\n\
             agentsandbox_exec_total {}\n\
             # HELP agentsandbox_egress_allowed Connessioni egress consentite\n\
             agentsandbox_egress_allowed {}\n\
             # HELP agentsandbox_egress_denied Connessioni egress negate\n\
             agentsandbox_egress_denied {}\n",
            self.sandboxes_created.load(Ordering::Relaxed),
            self.sandboxes_active.load(Ordering::Relaxed),
            self.exec_total.load(Ordering::Relaxed),
            self.egress_allowed.load(Ordering::Relaxed),
            self.egress_denied.load(Ordering::Relaxed),
        )
    }
}
```

### H.2 — Audit log strutturato

```rust
// crates/agentsandbox-daemon/src/audit.rs
use serde::Serialize;

#[derive(Serialize)]
pub struct AuditEvent {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub sandbox_id: String,
    pub tenant_id: Option<String>,
    pub backend_id: String,
    pub event: AuditEventKind,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEventKind {
    SandboxCreated { backend: String, ttl_seconds: u64 },
    // command_hash invece di command in chiaro — privacy
    ExecStarted  { command_hash: String },
    ExecFinished { exit_code: i64, duration_ms: u64 },
    SandboxDestroyed { reason: DestroyReason },
    EgressAllowed { hostname: String },
    EgressDenied  { hostname: String },
    BackendError  { error: String },
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DestroyReason {
    ClientRequest,
    TtlExpired,
    BackendError,
}
```

### Criteri di completamento Fase H

- [ ] `GET /metrics` ritorna formato Prometheus valido
- [ ] Ogni sandbox create/exec/destroy ha una entry in `audit_log`
- [ ] `command_hash` nell'audit (SHA256 del comando) — il comando non è in chiaro
- [ ] I log strutturati con `log_format: json` sono validi JSON per riga

---

## FASE I — Escape hatch (`extensions:`)
**Stima: 1-2 giorni**

Il campo `extensions:` nella spec permette di passare opzioni native del backend.

### I.1 — Aggiornamento compile pipeline

```rust
// crates/agentsandbox-core/src/compile.rs — aggiungi validazione

fn validate_extensions_safety(
    ext: &serde_json::Value,
    backend: &str,
) -> Result<(), CompileError> {
    // Opzioni che non possono mai passare via extensions —
    // interferiscono con il funzionamento interno del core.
    let forbidden = match backend {
        "docker" | "podman" => vec![
            ("/docker/hostConfig/networkMode", "usa spec.network.egress"),
            ("/docker/name", "gestito internamente"),
            ("/podman/hostConfig/networkMode", "usa spec.network.egress"),
            ("/podman/name", "gestito internamente"),
        ],
        "firecracker" => vec![
            ("/firecracker/vsock", "riservato al canale exec interno"),
        ],
        _ => vec![],
    };

    for (path, reason) in forbidden {
        if ext.pointer(path).is_some() {
            return Err(CompileError::ExtensionForbidden(
                path.trim_start_matches('/').into(),
                reason.into(),
            ));
        }
    }
    Ok(())
}
```

### I.2 — Nuovo endpoint

```
GET /v1/backends/:id/extensions-schema
→ JSON Schema delle extensions del backend (o 404 se non supportate)
```

### I.3 — SDK Python con extensions

```python
# sdks/python/agentsandbox/client.py — aggiorna __init__

def __init__(
    self,
    runtime: str = "python",
    # ... parametri esistenti ...
    backend: str | None = None,
    extensions: dict | None = None,
    daemon_url: str = "http://127.0.0.1:7847",
):
    # extensions richiede backend esplicito
    if extensions and not backend:
        raise ValueError(
            "extensions richiede backend esplicito. "
            "Es: Sandbox(runtime='python', backend='docker', extensions={...})"
        )
    self._config = SandboxConfig(
        # ...
        backend=backend,
        extensions=extensions,
    )
```

### Criteri di completamento Fase I

- [ ] `extensions:` senza `scheduling.backend` → `CompileError::ExtensionsRequireExplicitBackend`
- [ ] `extensions.docker.hostConfig.networkMode` → `CompileError::ExtensionForbidden`
- [ ] Docker backend: campo sconosciuto in extensions → 422 con nome del campo
- [ ] `GET /v1/backends/docker/extensions-schema` → JSON Schema
- [ ] SDK Python: `extensions={}` senza `backend=` → `ValueError` chiaro

---

## FASE J — Examples completi e verificati
**Stima: 1 giorno**

### J.1 — Struttura aggiornata

```
examples/
├── README.md
├── verify_all.sh                  ← script di verifica automatica
├── 01-hello-sandbox/
│   ├── README.md
│   ├── run.py
│   └── expected_output.txt
├── 02-code-review-agent/          ← già esistente, aggiorna
│   ├── README.md                  ← aggiungi sezione Troubleshooting
│   ├── requirements.txt
│   ├── agent.py
│   ├── sample_code/
│   │   └── buggy_script.py
│   └── expected_output.txt        ← NUOVO: output reale dopo primo run
├── 03-dependency-auditor/         ← già esistente, aggiorna
│   └── expected_output.txt        ← NUOVO
└── 04-multi-backend-demo/         ← NUOVO
    ├── README.md
    ├── demo.py
    └── expected_output.txt
```

### J.2 — Example 01: Hello Sandbox (nuovo, il più semplice)

```python
# examples/01-hello-sandbox/run.py
"""
Hello Sandbox — esempio minimo AgentSandbox
Prerequisiti: Docker running, agentsandbox-daemon running
Setup: pip install agentsandbox && python run.py
"""
import asyncio
from agentsandbox import Sandbox

async def main():
    print("Creazione sandbox...")
    async with Sandbox(runtime="python", ttl=60) as sb:
        result = await sb.exec("python -c 'print(\"hello from sandbox\")'")
        print(f"stdout:    {result.stdout.strip()}")
        print(f"exit_code: {result.exit_code}")
        print(f"duration:  {result.duration_ms}ms")
        assert result.success, "il comando deve avere successo"
        assert result.stdout.strip() == "hello from sandbox"
    print("Sandbox distrutta. Done.")

asyncio.run(main())
```

### J.3 — Example 04: Multi-backend demo (nuovo)

```python
# examples/04-multi-backend-demo/demo.py
"""
Dimostra che lo stesso codice funziona su backend diversi.
Prerequisiti: Docker running (+ opzionalmente Podman, gVisor)
"""
import asyncio
import httpx
from agentsandbox import Sandbox

COMMAND = "python -c 'import sys, platform; print(platform.system(), sys.version_info[:2])'"

async def run_on(backend_id: str) -> dict:
    try:
        async with Sandbox(runtime="python", ttl=60, backend=backend_id) as sb:
            r = await sb.exec(COMMAND)
            return {"backend": backend_id, "output": r.stdout.strip(),
                    "ms": r.duration_ms, "ok": True}
    except Exception as e:
        return {"backend": backend_id, "error": str(e), "ok": False}

async def main():
    # Scopri backend disponibili
    async with httpx.AsyncClient(base_url="http://127.0.0.1:7847") as client:
        resp = await client.get("/v1/backends")
        available = [b["id"] for b in resp.json() if b.get("healthy")]

    print(f"Backend disponibili: {', '.join(available)}\n")
    results = await asyncio.gather(*[run_on(b) for b in available])

    outputs = set()
    for r in results:
        if r["ok"]:
            print(f"✅ {r['backend']}: {r['output']} ({r['ms']}ms)")
            outputs.add(r["output"])
        else:
            print(f"❌ {r['backend']}: {r['error']}")

    if len(outputs) == 1:
        print("\n✅ Output identico su tutti i backend — contratto rispettato.")
    elif len(outputs) > 1:
        print("\n⚠️  Output diverso tra backend.")

asyncio.run(main())
```

### J.4 — Script di verifica

```bash
#!/bin/bash
# examples/verify_all.sh
set -euo pipefail

PASS=0; FAIL=0

check_daemon() {
    if ! curl -sf http://localhost:7847/v1/health > /dev/null; then
        echo "❌ Daemon non raggiungibile. Avvia con: cargo run -p agentsandbox-daemon"
        exit 1
    fi
    echo "✅ Daemon raggiungibile"
}

run() {
    local name="$1" cmd="$2" dir="$3"
    echo -n "  $name... "
    if (cd "examples/$dir" && eval "$cmd" > /tmp/as_test_out.txt 2>&1); then
        echo "✅"; PASS=$((PASS+1))
    else
        echo "❌"; FAIL=$((FAIL+1))
        sed 's/^/    /' /tmp/as_test_out.txt | head -15
    fi
}

echo "=== AgentSandbox Examples Verification ==="
check_daemon
echo ""

run "01-hello-sandbox"      "python run.py"    "01-hello-sandbox"
run "04-multi-backend-demo" "python demo.py"   "04-multi-backend-demo"

# Solo se ANTHROPIC_API_KEY è settata
if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
    run "02-code-review-agent" \
        "python agent.py sample_code/buggy_script.py" \
        "02-code-review-agent"
else
    echo "  02-code-review-agent... ⏭ (ANTHROPIC_API_KEY non settata)"
fi

echo ""
echo "Risultati: $PASS ✅  $FAIL ❌"
[ $FAIL -eq 0 ]
```

### Criteri di completamento Fase J

- [ ] `bash examples/verify_all.sh` con Docker running — zero fallimenti
- [ ] L'esempio 01 funziona senza `ANTHROPIC_API_KEY`
- [ ] Ogni esempio ha `expected_output.txt` con output reale (non inventato)
- [ ] Ogni README ha sezione Troubleshooting con i 3 errori più comuni
- [ ] `verify_all.sh` esce con codice 1 se qualcosa fallisce (usabile in CI)

---

## FASE K — Release checklist
**Esegui solo dopo che tutte le fasi precedenti sono passate.**

### Qualità codice

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
grep -r 'unwrap()' crates/*/src/ | grep -v '#\[cfg(test)\]' | grep -v test
# Output atteso: nessun risultato
grep -r 'todo!()' crates/*/src/ | grep -v '#\[cfg(test)\]'
# Output atteso: nessun risultato
cargo audit
```

### Sicurezza

- [ ] `secret_env` non appare in nessun log (test automatico)
- [ ] Il native handle del backend non è in nessuna response HTTP
- [ ] `extensions.*.networkMode` → errore nel compile pipeline
- [ ] `privileged: true` → warning nell'audit log

### Contratto pubblico

- [ ] `spec/sandbox.v1.schema.json` committato e aggiornato
- [ ] Ogni backend ha `schema/extensions.schema.json`
- [ ] `GET /v1/backends/:id/extensions-schema` ritorna lo schema corretto
- [ ] `crates/agentsandbox-sdk` compilabile senza Docker, Podman, gVisor, Firecracker
- [ ] `crates/agentsandbox-conformance` compilabile senza backend specifici

### Documentazione

- [ ] `docs/spec-v1.md` — ogni campo documentato con tipo, default, esempio YAML e JSON
- [ ] `docs/api-http-v1.md` — ogni endpoint con `curl` example completo
- [ ] `docs/backends/docker.md`, `podman.md`, `gvisor.md`, `firecracker.md`
- [ ] `BACKEND_GUIDE.md` — come creare un backend in < 2 pagine
- [ ] `CHANGELOG.md` aggiornato

### Pubblicazione

- [ ] Binary: `linux/amd64`, `linux/arm64`, `darwin/arm64`
- [ ] `agentsandbox` su PyPI (SDK Python)
- [ ] `agentsandbox` su npm (SDK TypeScript)
- [ ] `agentsandbox-sdk` su crates.io
- [ ] `agentsandbox-conformance` su crates.io

---

## Note operative per Claude Code

1. **Parti dalle Correzioni pre-Fase B.** Sono veloci (2-3 ore) ma bloccanti.
   Senza di esse il codice esistente è in conflitto con ciò che costruisci.

2. **Fase B prima di tutto il resto.** Il registry e il trait SDK sono i fondamenti.
   Nessuna delle fasi successive compila correttamente senza Fase B completata.

3. **Un backend alla volta.** Non iniziare gVisor prima che Podman passi la conformance suite.

4. **`cargo check -p agentsandbox-sdk` è il tuo test di purezza.** Se fallisce per
   dipendenze da Bollard o SQLx, l'architettura è sbagliata.

5. **La conformance suite è il gate di qualità.** Un backend che non la passa non va
   nel `backends.enabled` di default, indipendentemente da quanto funzioni in altri test.

6. **Firecracker (Fase E) ha un `todo!()` esplicito su exec e vsock.** Non fingere
   che sia completo. Implementa prima `health_check` e `create`, poi `exec` con vsock,
   poi `destroy`. Ogni step produce valore reale anche se il successivo è incompleto.

7. **`verify_all.sh` va eseguito prima di ogni merge.** Se un esempio smette di
   funzionare, è un bug — non un "esempio da aggiornare".