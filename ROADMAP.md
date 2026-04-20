# AgentSandbox — Implementation Roadmap
> Specifica operativa multi-step per Claude Code. Ogni fase è autonoma e produce output verificabile.
> **Leggi tutto prima di iniziare qualsiasi fase.**

---

## Contesto e obiettivo

Costruire `agentsandbox`: un daemon locale + SDK Python/TypeScript che permette a qualsiasi agente LLM di eseguire codice in sandbox isolate senza sapere cosa c'è sotto.

```
agente LLM
    │
    ▼
SDK Python / TypeScript   ←── il prodotto principale
    │
    ▼
API HTTP locale (daemon Rust)
    │
    ▼
Backend Adapter
    ├── Docker  (Fase 1)
    └── Firecracker  (Fase 3, fuori scope iniziale)
```

**Criterio di successo globale:**
```python
from agentsandbox import Sandbox

async with Sandbox(runtime="python", ttl=900, egress=["pypi.org"]) as sb:
    result = await sb.exec("pip install requests && python -c 'import requests; print(requests.__version__)'")
    print(result.stdout)
```
Questo deve funzionare, essere stabile e completare in < 3 secondi dopo il primo pull.

---

## Stack tecnologico — decisioni fisse

| Componente | Scelta | Motivazione |
|---|---|---|
| Daemon | Rust + Axum | Performance, safety, binary singolo |
| Persistence | SQLite via SQLx | Zero dipendenze operative |
| Spec format | YAML + JSON | Leggibilità + compatibilità |
| SDK Python | `httpx` + `asyncio` | Standard de-facto per agenti |
| SDK TypeScript | `fetch` nativo | Zero dipendenze runtime |
| Containerizzazione locale | Docker via Bollard (Rust crate) | Unico backend Fase 1 |
| Testing Rust | `tokio-test` + `testcontainers` | Test integrazione reali |
| Testing Python | `pytest` + `pytest-asyncio` | Standard |
| Testing TS | `vitest` | Veloce, ESM-native |

---

## Struttura repository

```
agentsandbox/
├── crates/
│   ├── agentsandbox-core/        # tipi condivisi, IR, spec parser
│   ├── agentsandbox-daemon/      # binary del daemon, API HTTP, SQLite
│   └── agentsandbox-docker/      # Docker adapter
├── sdks/
│   ├── python/                   # SDK Python (PyPI: agentsandbox)
│   └── typescript/               # SDK TypeScript (npm: agentsandbox)
├── spec/
│   └── sandbox.v1alpha1.schema.json   # JSON Schema ufficiale
├── tests/
│   └── e2e/                      # test end-to-end cross-SDK
├── docs/
│   ├── spec-v1alpha1.md
│   ├── api-http-v1.md
│   └── getting-started.md
├── Cargo.toml                    # workspace
└── ROADMAP.md                    # questo file
```

---

## FASE 0 — Setup workspace e tooling
**Stima:** 2-3 ore | **Prerequisiti:** Rust toolchain, Docker running

### 0.1 — Inizializza workspace Cargo

```toml
# Cargo.toml (workspace root)
[workspace]
members = [
    "crates/agentsandbox-core",
    "crates/agentsandbox-daemon",
    "crates/agentsandbox-docker",
]
resolver = "2"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["macros"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls", "macros"] }
bollard = "0.16"
uuid = { version = "1", features = ["v4"] }
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
futures = "0.3"
```
> Nota: `tracing` ha versione `0.1.x` (non `1.x`). `async-trait` e `futures` sono
> richiesti dal Docker adapter in Fase 2; averli già nel workspace evita
> ri-lock successivi.

### 0.2 — Crea i tre crate vuoti

```bash
cargo new --lib crates/agentsandbox-core
cargo new --lib crates/agentsandbox-docker
cargo new --bin crates/agentsandbox-daemon
```

### 0.3 — Setup Python SDK

```
sdks/python/
├── pyproject.toml
├── agentsandbox/
│   ├── __init__.py
│   ├── client.py
│   ├── models.py
│   └── exceptions.py
└── tests/
    └── test_client.py
```

```toml
# sdks/python/pyproject.toml
[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[project]
name = "agentsandbox"
version = "0.1.0"
requires-python = ">=3.10"
dependencies = ["httpx>=0.25", "pydantic>=2.0"]

[project.optional-dependencies]
dev = ["pytest", "pytest-asyncio", "respx"]
```

### 0.4 — Setup TypeScript SDK

```
sdks/typescript/
├── package.json
├── tsconfig.json
├── src/
│   ├── index.ts
│   ├── client.ts
│   ├── types.ts
│   └── errors.ts
└── tests/
    └── client.test.ts
```

### 0.5 — Criteri di completamento Fase 0

- [ ] `cargo check` passa su tutti e tre i crate
- [ ] `cd sdks/python && pip install -e ".[dev]"` funziona
- [ ] `cd sdks/typescript && npm install` funziona
- [ ] Struttura directory corrisponde allo schema sopra

---

## FASE 1 — Core types e Spec parser
**Stima:** 1 giorno | **Crate:** `agentsandbox-core`

### 1.1 — Spec v1alpha1

La spec è il contratto pubblico. Qualsiasi cosa non espressa qui non esiste.

```rust
// crates/agentsandbox-core/src/spec.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSpec {
    pub api_version: String,          // "sandbox.ai/v1alpha1"
    pub kind: String,                 // "Sandbox"
    pub metadata: Metadata,
    pub spec: SandboxSpecBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: Option<String>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSpecBody {
    pub runtime: RuntimeSpec,
    pub resources: Option<ResourceSpec>,
    pub network: Option<NetworkSpec>,
    pub secrets: Option<Vec<SecretRef>>,
    pub ttl_seconds: Option<u64>,
    pub scheduling: Option<SchedulingSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSpec {
    pub image: Option<String>,        // docker image override
    pub preset: Option<RuntimePreset>,
    pub env: Option<HashMap<String, String>>,  // NON secret env
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimePreset {
    Python,    // python:3.12-slim
    Node,      // node:20-slim
    Rust,      // rust:1.77-slim
    Shell,     // ubuntu:24.04
    Custom,    // richiede image esplicita
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    pub cpu_millicores: Option<u32>,  // default: 1000 (1 CPU)
    pub memory_mb: Option<u32>,       // default: 512
    pub disk_mb: Option<u32>,         // default: 1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSpec {
    pub egress: EgressPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressPolicy {
    #[serde(default)]
    pub allow: Vec<String>,    // hostname list, es. ["pypi.org", "github.com"]
    #[serde(default = "default_true")]
    pub deny_by_default: bool, // default: true
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    pub name: String,          // nome della variabile nel guest
    pub value_from: SecretSource,
}

/// Rappresentata come mappa con esattamente una chiave (envRef o file):
///   valueFrom:
///     envRef: MY_HOST_ENV
///
/// Scelta motivata: serde_yaml 0.9 emette tag YAML (`!envRef`) per enum
/// externally-tagged con varianti newtype. Usare uno struct con due Option
/// produce YAML idiomatico senza tag e permette validazione esplicita in
/// compile (esattamente uno dei due campi deve essere valorizzato).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretSource {
    pub env_ref: Option<String>,   // env var dell'host (mai esposta direttamente)
    pub file: Option<String>,      // path file sull'host
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulingSpec {
    pub prefer_warm: bool,     // default: false in v1alpha1
}
```

### 1.2 — Intermediate Representation (IR)

L'IR è interna, non pubblica. Converte la spec in qualcosa che l'adapter può consumare senza ambiguità.

```rust
// crates/agentsandbox-core/src/ir.rs

// NOTA: Debug è implementato manualmente (non derivato) per redigere i valori
// in secret_env. Derivare Debug esporrebbe i secret in ogni `tracing::debug!`,
// violando la Nota operativa #3.
#[derive(Clone)]
pub struct SandboxIR {
    pub id: String,                    // uuid v4, assegnato al momento del compile
    pub image: String,                 // immagine Docker risolta (mai preset)
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,    // env non-secret
    pub secret_env: Vec<(String, String)>, // secret già risolti (mai loggati)
    pub cpu_millicores: u32,
    pub memory_mb: u32,
    pub disk_mb: u32,
    pub egress_allow: Vec<String>,
    pub deny_by_default: bool,
    pub ttl_seconds: u64,
    pub working_dir: String,
}

// impl std::fmt::Debug per SandboxIR: stampa secret_env come
// `[(chiave, "<redacted>"), ...]` mantenendo visibili tutti gli altri campi.

impl Default for SandboxIR {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            image: "python:3.12-slim".into(),
            command: None,
            env: vec![],
            secret_env: vec![],
            cpu_millicores: 1000,
            memory_mb: 512,
            disk_mb: 1024,
            egress_allow: vec![],
            deny_by_default: true,
            ttl_seconds: 300,
            working_dir: "/workspace".into(),
        }
    }
}
```

### 1.3 — Compile pipeline (Spec → IR)

```rust
// crates/agentsandbox-core/src/compile.rs

use crate::{ir::SandboxIR, spec::*};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CompileError {
    #[error("apiVersion non supportata: {0}")]
    UnsupportedApiVersion(String),
    #[error("runtime.preset e runtime.image non possono essere entrambi assenti")]
    MissingRuntime,
    #[error("runtime.preset=Custom richiede runtime.image esplicita")]
    CustomPresetNeedsImage,
    #[error("secret {0} non trovato nell'ambiente host")]
    SecretNotFound(String),
    #[error("secret {name}: valueFrom deve contenere esattamente uno tra envRef o file")]
    InvalidSecretSource { name: String },
    #[error("egress allow contiene hostname non valido: {0}")]
    InvalidHostname(String),
}

pub fn compile(spec: SandboxSpec) -> Result<SandboxIR, CompileError> {
    // 1. Valida apiVersion
    if spec.api_version != "sandbox.ai/v1alpha1" {
        return Err(CompileError::UnsupportedApiVersion(spec.api_version));
    }

    let mut ir = SandboxIR::default();

    // 2. Risolvi runtime
    let body = &spec.spec;
    ir.image = resolve_image(&body.runtime)?;

    // 3. Risolvi risorse
    if let Some(res) = &body.resources {
        ir.cpu_millicores = res.cpu_millicores.unwrap_or(ir.cpu_millicores);
        ir.memory_mb = res.memory_mb.unwrap_or(ir.memory_mb);
        ir.disk_mb = res.disk_mb.unwrap_or(ir.disk_mb);
    }

    // 4. Risolvi network
    if let Some(net) = &body.network {
        for host in &net.egress.allow {
            validate_hostname(host)?;
        }
        ir.egress_allow = net.egress.allow.clone();
        ir.deny_by_default = net.egress.deny_by_default;
    }

    // 5. Risolvi secrets (mai loggati dopo questo punto)
    if let Some(secrets) = &body.secrets {
        for s in secrets {
            let value = resolve_secret(s)?;
            ir.secret_env.push((s.name.clone(), value));
        }
    }

    // 6. TTL
    ir.ttl_seconds = body.ttl_seconds.unwrap_or(300);

    // 7. Env non-secret
    if let Some(env) = &body.runtime.env {
        ir.env = env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    }

    Ok(ir)
}

fn resolve_image(runtime: &RuntimeSpec) -> Result<String, CompileError> {
    if let Some(image) = &runtime.image {
        return Ok(image.clone());
    }
    match &runtime.preset {
        Some(RuntimePreset::Python) => Ok("python:3.12-slim".into()),
        Some(RuntimePreset::Node)   => Ok("node:20-slim".into()),
        Some(RuntimePreset::Rust)   => Ok("rust:1.77-slim".into()),
        Some(RuntimePreset::Shell)  => Ok("ubuntu:24.04".into()),
        Some(RuntimePreset::Custom) => Err(CompileError::CustomPresetNeedsImage),
        None => Err(CompileError::MissingRuntime),
    }
}

fn resolve_secret(s: &SecretRef) -> Result<String, CompileError> {
    match (&s.value_from.env_ref, &s.value_from.file) {
        (Some(name), None) => std::env::var(name)
            .map_err(|_| CompileError::SecretNotFound(name.clone())),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map(|raw| raw.trim().to_string())
            .map_err(|_| CompileError::SecretNotFound(path.clone())),
        _ => Err(CompileError::InvalidSecretSource { name: s.name.clone() }),
    }
}

fn validate_hostname(host: &str) -> Result<(), CompileError> {
    // Validazione basilare: no IP, no wildcard, no path
    if host.contains('/') || host.contains('*') || host.parse::<std::net::IpAddr>().is_ok() {
        return Err(CompileError::InvalidHostname(host.to_string()));
    }
    Ok(())
}
```

### 1.4 — Test Fase 1

> I test vivono inline in ogni modulo (`src/spec.rs`, `src/ir.rs`,
> `src/compile.rs`) sotto `#[cfg(test)] mod tests`. Questo è idiomatico
> Rust, evita di dover wire-up un `mod tests;` extra e mantiene i test
> vicino al codice che verificano.

```rust
// crates/agentsandbox-core/src/compile.rs (sezione #[cfg(test)])

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_spec(preset: &str) -> SandboxSpec {
        serde_yaml::from_str(&format!(r#"
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata:
  name: test
spec:
  runtime:
    preset: {}
"#, preset)).unwrap()
    }

    #[test]
    fn test_python_preset_resolves_image() {
        let ir = compile(minimal_spec("python")).unwrap();
        assert_eq!(ir.image, "python:3.12-slim");
    }

    #[test]
    fn test_missing_runtime_is_error() {
        let spec: SandboxSpec = serde_yaml::from_str(r#"
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata: {}
spec:
  runtime: {}
"#).unwrap();
        assert!(matches!(compile(spec), Err(CompileError::MissingRuntime)));
    }

    #[test]
    fn test_wrong_api_version_is_error() {
        let mut spec = minimal_spec("python");
        spec.api_version = "sandbox.ai/v2".into();
        assert!(matches!(compile(spec), Err(CompileError::UnsupportedApiVersion(_))));
    }

    #[test]
    fn test_default_ttl_is_300() {
        let ir = compile(minimal_spec("python")).unwrap();
        assert_eq!(ir.ttl_seconds, 300);
    }

    #[test]
    fn test_ip_in_egress_is_error() {
        let spec: SandboxSpec = serde_yaml::from_str(r#"
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  network:
    egress:
      allow: ["1.2.3.4"]
      denyByDefault: true
"#).unwrap();
        assert!(matches!(compile(spec), Err(CompileError::InvalidHostname(_))));
    }
}
```

### 1.5 — Criteri di completamento Fase 1

- [ ] `cargo test -p agentsandbox-core` passa tutti i test
- [ ] `compile()` restituisce `Err` esplicito per ogni caso invalido (zero panic, zero unwrap in produzione)
- [ ] Nessun campo backend-specifico nella spec pubblica
- [ ] I secret non appaiono in nessun log (verifica con `tracing`): `SandboxIR`
      implementa `Debug` manualmente e redige i valori di `secret_env`. Esiste
      un test `debug_redacts_secret_env_values` che blocca la regressione.
- [ ] `cargo clippy -p agentsandbox-core --all-targets -- -D warnings` passa pulito

---

## FASE 2 — Docker Adapter
**Stima:** 1-2 giorni | **Crate:** `agentsandbox-docker`

### 2.1 — Adapter trait (contratto comune)

```rust
// crates/agentsandbox-core/src/adapter.rs

use async_trait::async_trait;
use crate::ir::SandboxIR;

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SandboxInfo {
    pub sandbox_id: String,
    pub status: SandboxStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SandboxStatus {
    Creating,
    Running,
    Stopped,
    Error(String),
}

#[async_trait]
pub trait SandboxAdapter: Send + Sync {
    /// Crea la sandbox. Ritorna il sandbox_id interno del backend.
    async fn create(&self, ir: &SandboxIR) -> Result<String, AdapterError>;

    /// Esegue un comando nella sandbox esistente.
    async fn exec(&self, sandbox_id: &str, command: &str) -> Result<ExecResult, AdapterError>;

    /// Ritorna lo stato corrente della sandbox.
    async fn inspect(&self, sandbox_id: &str) -> Result<SandboxInfo, AdapterError>;

    /// Distrugge la sandbox e libera tutte le risorse.
    async fn destroy(&self, sandbox_id: &str) -> Result<(), AdapterError>;

    /// Nome del backend (per logging e audit).
    fn backend_name(&self) -> &'static str;

    /// Verifica che il backend sia disponibile e funzionante.
    async fn health_check(&self) -> Result<(), AdapterError>;
}

#[derive(thiserror::Error, Debug)]
pub enum AdapterError {
    #[error("sandbox non trovata: {0}")]
    NotFound(String),
    #[error("backend non disponibile: {0}")]
    BackendUnavailable(String),
    #[error("exec fallita con exit code {exit_code}: {stderr}")]
    ExecFailed { exit_code: i64, stderr: String },
    #[error("timeout dopo {0}ms")]
    Timeout(u64),
    #[error("errore interno: {0}")]
    Internal(String),
}
```

### 2.2 — Docker Adapter implementation

```rust
// crates/agentsandbox-docker/src/lib.rs

use agentsandbox_core::{adapter::*, ir::SandboxIR};
use bollard::{
    Docker,
    container::{CreateContainerOptions, Config, StartContainerOptions, RemoveContainerOptions},
    exec::{CreateExecOptions, StartExecResults},
    models::HostConfig,
};
use async_trait::async_trait;
use futures::StreamExt;

pub struct DockerAdapter {
    client: Docker,
}

impl DockerAdapter {
    pub fn new() -> Result<Self, AdapterError> {
        let client = Docker::connect_with_local_defaults()
            .map_err(|e| AdapterError::BackendUnavailable(e.to_string()))?;
        Ok(Self { client })
    }

    fn container_name(sandbox_id: &str) -> String {
        format!("agentsandbox-{}", sandbox_id)
    }
}

#[async_trait]
impl SandboxAdapter for DockerAdapter {
    async fn create(&self, ir: &SandboxIR) -> Result<String, AdapterError> {
        // Costruisci env list (secret + non-secret unificati per Docker)
        let mut env: Vec<String> = ir.env.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        env.extend(ir.secret_env.iter().map(|(k, v)| format!("{}={}", k, v)));

        let host_config = HostConfig {
            memory: Some((ir.memory_mb as i64) * 1024 * 1024),
            nano_cpus: Some((ir.cpu_millicores as i64) * 1_000_000),
            network_mode: Some(self.build_network_mode(ir)),
            auto_remove: Some(false), // gestiamo noi il lifecycle
            ..Default::default()
        };

        let config = Config {
            image: Some(ir.image.clone()),
            env: Some(env),
            working_dir: Some(ir.working_dir.clone()),
            host_config: Some(host_config),
            // Tieni il container vivo senza CMD
            cmd: Some(vec!["sleep".to_string(), ir.ttl_seconds.to_string()]),
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
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        self.client
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        Ok(ir.id.clone())
    }

    async fn exec(&self, sandbox_id: &str, command: &str) -> Result<ExecResult, AdapterError> {
        let container_name = Self::container_name(sandbox_id);
        let start = std::time::Instant::now();

        let exec = self.client
            .create_exec(
                &container_name,
                CreateExecOptions {
                    cmd: Some(vec!["sh", "-c", command]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let StartExecResults::Attached { mut output, .. } = self.client
            .start_exec(&exec.id, None)
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?
        {
            while let Some(chunk) = output.next().await {
                match chunk.map_err(|e| AdapterError::Internal(e.to_string()))? {
                    bollard::container::LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }

        let inspect = self.client
            .inspect_exec(&exec.id)
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?;

        let exit_code = inspect.exit_code.unwrap_or(-1);

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn inspect(&self, sandbox_id: &str) -> Result<SandboxInfo, AdapterError> {
        let container_name = Self::container_name(sandbox_id);
        let info = self.client
            .inspect_container(&container_name, None)
            .await
            .map_err(|_| AdapterError::NotFound(sandbox_id.to_string()))?;

        let status = match info.state.and_then(|s| s.running) {
            Some(true) => SandboxStatus::Running,
            Some(false) => SandboxStatus::Stopped,
            None => SandboxStatus::Error("stato sconosciuto".into()),
        };

        Ok(SandboxInfo {
            sandbox_id: sandbox_id.to_string(),
            status,
            created_at: chrono::Utc::now(), // semplificato per ora
            expires_at: chrono::Utc::now(),
        })
    }

    async fn destroy(&self, sandbox_id: &str) -> Result<(), AdapterError> {
        let container_name = Self::container_name(sandbox_id);
        self.client
            .remove_container(
                &container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| AdapterError::Internal(e.to_string()))?;
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "docker"
    }

    async fn health_check(&self) -> Result<(), AdapterError> {
        self.client
            .ping()
            .await
            .map_err(|e| AdapterError::BackendUnavailable(e.to_string()))?;
        Ok(())
    }

    // NOTA: questo è il punto dove egress viene implementato.
    // In v1alpha1 usiamo --network=none + un proxy interno.
    // Per ora: log warning se egress.allow non è vuoto, non applicare filtri.
    // Documentare esplicitamente questo limite.
    fn build_network_mode(&self, ir: &SandboxIR) -> String {
        if ir.deny_by_default && ir.egress_allow.is_empty() {
            "none".to_string()
        } else {
            // TODO Fase 2.5: implementare proxy squid o socat per whitelist
            tracing::warn!(
                "egress allowlist non ancora applicata in v1alpha1-docker. \
                 La sandbox ha accesso a internet non filtrato."
            );
            "bridge".to_string()
        }
    }
}
```

### 2.3 — Conformance suite

Ogni adapter DEVE passare questa suite di test. È il contratto che garantisce l'intercambiabilità.

```rust
// crates/agentsandbox-core/src/conformance.rs
// Questo modulo viene importato da ogni adapter crate nei test.

#[cfg(test)]
pub mod conformance_suite {
    use crate::{adapter::SandboxAdapter, ir::SandboxIR};

    pub async fn test_create_and_destroy(adapter: &dyn SandboxAdapter) {
        let ir = SandboxIR::default();
        let id = adapter.create(&ir).await.expect("create deve funzionare");
        adapter.destroy(&id).await.expect("destroy deve funzionare");
    }

    pub async fn test_exec_returns_stdout(adapter: &dyn SandboxAdapter) {
        let ir = SandboxIR::default();
        let id = adapter.create(&ir).await.unwrap();
        let result = adapter.exec(&id, "echo 'hello conformance'").await.unwrap();
        assert!(result.stdout.contains("hello conformance"));
        assert_eq!(result.exit_code, 0);
        adapter.destroy(&id).await.unwrap();
    }

    pub async fn test_exec_captures_stderr(adapter: &dyn SandboxAdapter) {
        let ir = SandboxIR::default();
        let id = adapter.create(&ir).await.unwrap();
        let result = adapter.exec(&id, "echo 'err' >&2; exit 1").await.unwrap();
        assert!(result.stderr.contains("err"));
        assert_eq!(result.exit_code, 1);
        adapter.destroy(&id).await.unwrap();
    }

    pub async fn test_inspect_running(adapter: &dyn SandboxAdapter) {
        let ir = SandboxIR::default();
        let id = adapter.create(&ir).await.unwrap();
        let info = adapter.inspect(&id).await.unwrap();
        assert_eq!(info.status, crate::adapter::SandboxStatus::Running);
        adapter.destroy(&id).await.unwrap();
    }

    pub async fn test_destroy_nonexistent_is_ok_or_not_found(adapter: &dyn SandboxAdapter) {
        let result = adapter.destroy("sandbox-che-non-esiste-xyzxyz").await;
        match result {
            Ok(_) => {}
            Err(crate::adapter::AdapterError::NotFound(_)) => {}
            Err(e) => panic!("errore inatteso: {}", e),
        }
    }
}
```

### 2.4 — Criteri di completamento Fase 2

- [ ] `cargo test -p agentsandbox-docker` con Docker running passa tutta la conformance suite
- [ ] `create` → `exec` → `destroy` completa senza leak di container
- [ ] Il warning egress è loggato quando `egress.allow` non è vuoto
- [ ] Nessun native handle Docker (`ContainerId`, ecc.) esposto fuori dal crate

> **Nota operativa.** L'adapter in v1alpha1 NON fa `docker pull`: se l'immagine
> non è già nella cache locale, `create` fallisce con `INTERNAL_ERROR` e il
> messaggio di Docker è riportato fedelmente nel campo `message`. Gli
> operatori (o un futuro `pull on demand`) devono pre-scaricare le immagini
> dei preset. La conformance suite usa `alpine:3.20` e la scarica
> esplicitamente prima di girare (vedi `tests/conformance.rs`).
>
> **Errore strutturale nel codice di esempio sopra.** Il metodo
> `build_network_mode` è un helper privato del `DockerAdapter`, non un
> metodo del trait `SandboxAdapter`: nell'implementazione reale va in
> `impl DockerAdapter { ... }`, NON in `impl SandboxAdapter for DockerAdapter`.
> Il codice del ROADMAP lo mostra dentro il blocco trait per vicinanza
> visuale ma non compilerebbe così; l'implementazione lo ha spostato.

---

## FASE 3 — Daemon HTTP
**Stima:** 2 giorni | **Crate:** `agentsandbox-daemon`

### 3.1 — Schema SQLite

```sql
-- migrations/001_initial.sql

CREATE TABLE sandboxes (
    id TEXT PRIMARY KEY,
    lease_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL,          -- creating | running | stopped | error
    backend TEXT NOT NULL,         -- docker | firecracker
    spec_json TEXT NOT NULL,       -- spec originale serializzata
    ir_json TEXT NOT NULL,         -- IR serializzata (secret_env OMESSI)
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    error_message TEXT
);

CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sandbox_id TEXT NOT NULL,
    event TEXT NOT NULL,           -- created | exec | destroyed | expired | error
    detail TEXT,
    ts TEXT NOT NULL
);

CREATE INDEX idx_sandboxes_status ON sandboxes(status);
CREATE INDEX idx_sandboxes_expires ON sandboxes(expires_at);
CREATE INDEX idx_audit_sandbox ON audit_log(sandbox_id);
```

### 3.2 — API HTTP v1 — endpoint completi

```
POST   /v1/sandboxes                → crea sandbox da spec
GET    /v1/sandboxes/:id            → inspect
POST   /v1/sandboxes/:id/exec       → esegui comando
DELETE /v1/sandboxes/:id            → destroy

GET    /v1/health                   → {"status": "ok", "backend": "docker"}
GET    /v1/sandboxes                → lista sandbox attive (paginata)
```

**Request/Response contracts:**

```json
// POST /v1/sandboxes
// Request body: spec YAML o JSON
// Response 201:
{
  "sandbox_id": "uuid-v4",
  "lease_token": "opaque-token",
  "status": "running",
  "expires_at": "2024-01-01T00:05:00Z",
  "backend": "docker"
}

// POST /v1/sandboxes/:id/exec
// Request:
{ "command": "python -c 'print(1+1)'" }
// Response 200:
{
  "stdout": "2\n",
  "stderr": "",
  "exit_code": 0,
  "duration_ms": 142
}

// Error response (tutti gli errori):
{
  "error": {
    "code": "SANDBOX_NOT_FOUND",
    "message": "sandbox abc123 non trovata",
    "details": {}
  }
}
```

**Codici errore standardizzati:**
```
SANDBOX_NOT_FOUND
SANDBOX_EXPIRED
SPEC_INVALID
BACKEND_UNAVAILABLE
EXEC_TIMEOUT
LEASE_INVALID
INTERNAL_ERROR
```

### 3.3 — Main daemon structure

```rust
// crates/agentsandbox-daemon/src/main.rs

use axum::{Router, routing::{get, post, delete}};
use std::sync::Arc;
use agentsandbox_docker::DockerAdapter;

pub struct AppState {
    pub db: sqlx::SqlitePool,
    pub adapter: Arc<dyn agentsandbox_core::adapter::SandboxAdapter>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("agentsandbox=debug,tower_http=info")
        .init();

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://agentsandbox.db".to_string());

    let db = sqlx::SqlitePool::connect(&db_url).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let adapter = Arc::new(DockerAdapter::new()?);
    adapter.health_check().await?;

    let state = Arc::new(AppState { db, adapter });

    let app = Router::new()
        .route("/v1/health", get(handlers::health))
        .route("/v1/sandboxes", post(handlers::create_sandbox))
        .route("/v1/sandboxes", get(handlers::list_sandboxes))
        .route("/v1/sandboxes/:id", get(handlers::inspect_sandbox))
        .route("/v1/sandboxes/:id/exec", post(handlers::exec_sandbox))
        .route("/v1/sandboxes/:id", delete(handlers::destroy_sandbox))
        .with_state(state);

    let addr = "127.0.0.1:7847";
    tracing::info!("daemon in ascolto su http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

### 3.4 — TTL reaper (background task)

```rust
// crates/agentsandbox-daemon/src/reaper.rs
// Avviato come tokio::spawn in main.

pub async fn reaper_loop(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        if let Err(e) = reap_expired(&state).await {
            tracing::error!("reaper error: {}", e);
        }
    }
}

async fn reap_expired(state: &AppState) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let expired = sqlx::query!(
        "SELECT id FROM sandboxes WHERE status = 'running' AND expires_at < ?",
        now
    )
    .fetch_all(&state.db)
    .await?;

    for row in expired {
        tracing::info!("reaping expired sandbox {}", row.id);
        if let Err(e) = state.adapter.destroy(&row.id).await {
            tracing::warn!("destroy failed for {}: {}", row.id, e);
        }
        sqlx::query!(
            "UPDATE sandboxes SET status = 'stopped' WHERE id = ?",
            row.id
        )
        .execute(&state.db)
        .await?;
    }
    Ok(())
}
```

### 3.5 — Criteri di completamento Fase 3

- [ ] Il daemon si avvia con `cargo run -p agentsandbox-daemon`
- [ ] `curl -X POST localhost:7847/v1/sandboxes -d @spec.yaml` crea una sandbox reale
- [ ] `curl -X POST localhost:7847/v1/sandboxes/{id}/exec -d '{"command":"echo hi"}'` ritorna stdout
- [ ] Le sandbox scadute vengono distrutte automaticamente dal reaper
- [ ] Ogni operazione ha una entry nell'audit_log
- [ ] Gli errori ritornano sempre il formato JSON standardizzato (mai stack trace raw)

> **Note implementative.**
>
> * Usiamo `sqlx::query` runtime (non la macro `query!`) per evitare che
>   `cargo build` richieda un `DATABASE_URL` con schema già applicato; le
>   migrazioni girano automaticamente ad ogni boot (`sqlx::migrate!`).
> * YAML e JSON sono entrambi accettati sul `POST /v1/sandboxes`: la parse
>   route è pilotata dall'header `Content-Type` (`*yaml*` → serde_yaml,
>   altrimenti serde_json).
> * `DELETE /v1/sandboxes/:id` è idempotente: se la riga non esiste e
>   anche il container è già scomparso, ritorna comunque `204`. Quando la
>   riga esiste, il lease token viene verificato per impedire teardown
>   non autorizzati.
> * `ir_json` in SQLite contiene la proiezione `StoredIr` che OMETTE
>   `secret_env`: i secret resolvedi non vengono mai persistiti (coerente
>   con la Nota operativa #3).

---

## FASE 4 — SDK Python
**Stima:** 1 giorno | **Path:** `sdks/python/`

### 4.1 — API pubblica Python

```python
# sdks/python/agentsandbox/client.py

import httpx
from contextlib import asynccontextmanager
from .models import SandboxConfig, ExecResult, SandboxInfo
from .exceptions import SandboxError, SandboxNotFoundError, SpecInvalidError

class Sandbox:
    """
    Context manager per sandbox agentiche.

    Uso base:
        async with Sandbox(runtime="python", ttl=900) as sb:
            result = await sb.exec("python -c 'print(42)'")

    Uso avanzato:
        async with Sandbox(
            runtime="python",
            ttl=300,
            egress=["pypi.org", "files.pythonhosted.org"],
            memory_mb=1024,
        ) as sb:
            await sb.exec("pip install httpx")
            result = await sb.exec("python script.py")
    """

    def __init__(
        self,
        runtime: str = "python",
        image: str | None = None,
        ttl: int = 300,
        egress: list[str] | None = None,
        memory_mb: int = 512,
        cpu_millicores: int = 1000,
        env: dict[str, str] | None = None,
        secrets: dict[str, str] | None = None,
        daemon_url: str = "http://127.0.0.1:7847",
    ):
        self._config = SandboxConfig(
            runtime=runtime,
            image=image,
            ttl=ttl,
            egress=egress or [],
            memory_mb=memory_mb,
            cpu_millicores=cpu_millicores,
            env=env or {},
            secrets=secrets or {},
        )
        self._url = daemon_url
        self._sandbox_id: str | None = None
        self._lease_token: str | None = None
        self._client = httpx.AsyncClient(base_url=daemon_url, timeout=60.0)

    async def __aenter__(self) -> "Sandbox":
        await self._create()
        return self

    async def __aexit__(self, *args):
        await self._destroy()
        await self._client.aclose()

    async def _create(self):
        spec = self._config.to_spec()
        response = await self._client.post("/v1/sandboxes", json=spec)
        _raise_for_status(response)
        data = response.json()
        self._sandbox_id = data["sandbox_id"]
        self._lease_token = data["lease_token"]

    async def exec(self, command: str) -> ExecResult:
        if not self._sandbox_id:
            raise SandboxError("Sandbox non inizializzata. Usa 'async with Sandbox() as sb:'")
        response = await self._client.post(
            f"/v1/sandboxes/{self._sandbox_id}/exec",
            json={"command": command},
            headers={"X-Lease-Token": self._lease_token},
        )
        _raise_for_status(response)
        data = response.json()
        return ExecResult(**data)

    async def inspect(self) -> SandboxInfo:
        response = await self._client.get(f"/v1/sandboxes/{self._sandbox_id}")
        _raise_for_status(response)
        return SandboxInfo(**response.json())

    async def _destroy(self):
        if self._sandbox_id:
            try:
                await self._client.delete(
                    f"/v1/sandboxes/{self._sandbox_id}",
                    headers={"X-Lease-Token": self._lease_token},
                )
            except Exception:
                pass  # best-effort


def _raise_for_status(response: httpx.Response):
    if response.status_code == 404:
        raise SandboxNotFoundError(response.json()["error"]["message"])
    if response.status_code == 422:
        raise SpecInvalidError(response.json()["error"]["message"])
    if response.status_code >= 400:
        raise SandboxError(response.json().get("error", {}).get("message", "errore sconosciuto"))
```

```python
# sdks/python/agentsandbox/models.py

from dataclasses import dataclass

@dataclass
class ExecResult:
    stdout: str
    stderr: str
    exit_code: int
    duration_ms: int

    @property
    def success(self) -> bool:
        return self.exit_code == 0

    def __str__(self):
        return self.stdout

@dataclass
class SandboxConfig:
    runtime: str
    image: str | None
    ttl: int
    egress: list[str]
    memory_mb: int
    cpu_millicores: int
    env: dict[str, str]
    secrets: dict[str, str]

    def to_spec(self) -> dict:
        spec = {
            "apiVersion": "sandbox.ai/v1alpha1",
            "kind": "Sandbox",
            "metadata": {},
            "spec": {
                "runtime": {
                    "preset": self.runtime if not self.image else None,
                    "image": self.image,
                    "env": self.env or None,
                },
                "resources": {
                    "memoryMb": self.memory_mb,
                    "cpuMillicores": self.cpu_millicores,
                },
                "ttlSeconds": self.ttl,
            }
        }
        if self.egress:
            spec["spec"]["network"] = {
                "egress": {
                    "allow": self.egress,
                    "denyByDefault": True,
                }
            }
        return spec
```

### 4.2 — Test SDK Python

```python
# sdks/python/tests/test_client.py
# Questi test richiedono il daemon in esecuzione.
# Eseguire con: pytest tests/ -m integration

import pytest
import pytest_asyncio
from agentsandbox import Sandbox

@pytest.mark.asyncio
@pytest.mark.integration
async def test_basic_exec():
    async with Sandbox(runtime="python", ttl=60) as sb:
        result = await sb.exec("echo 'hello from sandbox'")
        assert result.success
        assert "hello from sandbox" in result.stdout

@pytest.mark.asyncio
@pytest.mark.integration
async def test_python_code_runs():
    async with Sandbox(runtime="python", ttl=60) as sb:
        result = await sb.exec("python -c 'print(1 + 1)'")
        assert result.stdout.strip() == "2"

@pytest.mark.asyncio
@pytest.mark.integration
async def test_exit_code_captured():
    async with Sandbox(runtime="shell", ttl=60) as sb:
        result = await sb.exec("exit 42")
        assert result.exit_code == 42
        assert not result.success

@pytest.mark.asyncio
@pytest.mark.integration
async def test_sandbox_destroyed_on_exit():
    sandbox_id = None
    async with Sandbox(runtime="python", ttl=60) as sb:
        sandbox_id = sb._sandbox_id
        assert sandbox_id is not None
    # Dopo il context manager, la sandbox non deve esistere
    import httpx
    client = httpx.AsyncClient(base_url="http://127.0.0.1:7847")
    r = await client.get(f"/v1/sandboxes/{sandbox_id}")
    assert r.status_code == 404
    await client.aclose()
```

### 4.3 — Criteri di completamento Fase 4

- [ ] `pip install -e sdks/python` + daemon running → tutti i test integration passano
- [ ] `ExecResult.success` funziona correttamente
- [ ] Errori del daemon mappano a eccezioni Python tipizzate
- [ ] Il context manager destroy chiama sempre la sandbox (anche in caso di eccezione)

> **Note implementative.**
>
> * **`secrets` nell'SDK.** Il parametro `secrets: dict[str, str]` NON trasporta
>   valori segreti: la mappa è `{nome_env_nel_guest: nome_env_var_sull_host}` e
>   viene convertita in `valueFrom.envRef`. È il daemon a risolverla contro il
>   proprio ambiente. Questo preserva la Nota operativa #3 (i secret non
>   attraversano mai l'SDK).
> * **Test in due layer.** `tests/test_client_unit.py` (respx) gira senza
>   daemon e copre transport + error mapping. `tests/test_client.py` (marker
>   `integration`) richiede daemon+Docker; `conftest.py` auto-skippa questi
>   test quando il daemon non è raggiungibile, così `pytest` puro non fallisce
>   in ambienti senza Docker.
> * **`to_spec` omette le chiavi vuote** (`env`, `secrets`, `network`,
>   `image`/`preset`) invece di serializzarle come `null`. Questo evita falsi
>   `MissingRuntime` quando entrambi image e preset arrivano a `null` e
>   mantiene il body stabile rispetto al validator lato daemon.

---

## FASE 5 — SDK TypeScript
**Stima:** 1 giorno | **Path:** `sdks/typescript/`

### 5.1 — API pubblica TypeScript

```typescript
// sdks/typescript/src/client.ts

import { SandboxConfig, ExecResult, CreateResponse } from './types';
import { SandboxError, SandboxNotFoundError, SpecInvalidError } from './errors';

export class Sandbox {
  private config: SandboxConfig;
  private daemonUrl: string;
  private sandboxId?: string;
  private leaseToken?: string;

  constructor(config: Partial<SandboxConfig> & { runtime: string }) {
    this.config = {
      runtime: config.runtime,
      image: config.image,
      ttl: config.ttl ?? 300,
      egress: config.egress ?? [],
      memoryMb: config.memoryMb ?? 512,
      cpuMillicores: config.cpuMillicores ?? 1000,
      env: config.env ?? {},
    };
    this.daemonUrl = config.daemonUrl ?? 'http://127.0.0.1:7847';
  }

  static async create(config: Partial<SandboxConfig> & { runtime: string }): Promise<Sandbox> {
    const sb = new Sandbox(config);
    await sb._create();
    return sb;
  }

  async exec(command: string): Promise<ExecResult> {
    if (!this.sandboxId) throw new SandboxError('Sandbox non inizializzata');

    const res = await fetch(`${this.daemonUrl}/v1/sandboxes/${this.sandboxId}/exec`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Lease-Token': this.leaseToken!,
      },
      body: JSON.stringify({ command }),
    });

    await this._raiseForStatus(res);
    return res.json() as Promise<ExecResult>;
  }

  async destroy(): Promise<void> {
    if (!this.sandboxId) return;
    await fetch(`${this.daemonUrl}/v1/sandboxes/${this.sandboxId}`, {
      method: 'DELETE',
      headers: { 'X-Lease-Token': this.leaseToken! },
    }).catch(() => {});
  }

  // Helper per uso con using (TC39 explicit resource management)
  async [Symbol.asyncDispose]() {
    await this.destroy();
  }

  private async _create(): Promise<void> {
    const spec = this._buildSpec();
    const res = await fetch(`${this.daemonUrl}/v1/sandboxes`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(spec),
    });
    await this._raiseForStatus(res);
    const data: CreateResponse = await res.json();
    this.sandboxId = data.sandbox_id;
    this.leaseToken = data.lease_token;
  }

  private _buildSpec(): object {
    return {
      apiVersion: 'sandbox.ai/v1alpha1',
      kind: 'Sandbox',
      metadata: {},
      spec: {
        runtime: {
          preset: this.config.image ? undefined : this.config.runtime,
          image: this.config.image,
          env: Object.keys(this.config.env).length ? this.config.env : undefined,
        },
        resources: {
          memoryMb: this.config.memoryMb,
          cpuMillicores: this.config.cpuMillicores,
        },
        ttlSeconds: this.config.ttl,
        ...(this.config.egress.length && {
          network: {
            egress: { allow: this.config.egress, denyByDefault: true },
          },
        }),
      },
    };
  }

  private async _raiseForStatus(res: Response): Promise<void> {
    if (res.ok) return;
    const body = await res.json().catch(() => ({}));
    const message = body?.error?.message ?? 'errore sconosciuto';
    if (res.status === 404) throw new SandboxNotFoundError(message);
    if (res.status === 422) throw new SpecInvalidError(message);
    throw new SandboxError(message);
  }
}
```

### 5.2 — Criteri di completamento Fase 5

- [ ] `npm run test` passa i test unitari (fetch mockata, nessun daemon); con
      `AGENTSANDBOX_INTEGRATION=1` anche gli integration test passano
- [ ] Il tipo `ExecResult` è completamente tipizzato (no `any`)
- [ ] `Symbol.asyncDispose` funziona con `await using` (TypeScript 5.2+ e
      `"lib": [..., "ESNext.Disposable"]` nel `tsconfig.json`)
- [ ] SDK pubblicabile su npm senza dipendenze runtime (sezione
      `dependencies` del `package.json` assente o vuota)

> **Note implementative / errori strutturali nel codice di esempio sopra.**
>
> * **Spread su `0`.** `...(this.config.egress.length && { network: ... })`
>   valuta a `...0` quando `egress` è vuoto, cosa che produce un errore TS
>   (`Spread types may only be created from object types`). Usa
>   `this.config.egress.length > 0` o un `if` esplicito.
> * **`SandboxConfig`.** L'interfaccia usata dal costruttore non è mostrata
>   nel ROADMAP: nell'implementazione vive in `src/types.ts` e include anche
>   `daemonUrl`, `secrets` e un `fetch` iniettabile (per i test unitari).
> * **Test in due layer.** `tests/client.test.ts` usa `vi.fn()` per stubbare
>   `fetch` e non richiede daemon. `tests/integration.test.ts` auto-skip
>   quando il daemon non è raggiungibile (probe di `/v1/health`); forza
>   l'esecuzione con `AGENTSANDBOX_INTEGRATION=1`. Questo rispecchia la
>   strategia del SDK Python e mantiene `npm run test` verde anche senza
>   Docker.
> * **Secrets.** Come nel Python SDK, `secrets: Record<string, string>` è
>   `{ guestEnvName: hostEnvVarName }` e viene convertita in
>   `valueFrom.envRef`: il valore risolto non attraversa mai il SDK (Nota
>   operativa #3).
> * **Iniezione `fetch`.** Oltre a semplificare i test, permette di usare
>   `undici` o `node-fetch` su runtime molto vecchi. Default: `globalThis.fetch`
>   (Node ≥ 18).

---

## FASE 6 — Egress filtering reale
**Stima:** 1-2 giorni | **Nota: questa è la feature più complessa**

### 6.1 — Approccio v1alpha1

L'approccio è hostname allowlist applicata **a startup**, non real-time. I limiti vanno documentati esplicitamente.

**Implementazione con iptables + DNS pre-risolto:**

```rust
// crates/agentsandbox-docker/src/egress.rs

/// Risolve gli hostname in IP a startup e configura iptables nel container.
/// LIMITE DOCUMENTATO: IP che cambiano dopo la creazione non vengono aggiornati.
/// LIMITE DOCUMENTATO: DNS rebinding non è prevenuto in v1alpha1.
pub async fn apply_egress_rules(
    client: &Docker,
    container_id: &str,
    allow: &[String],
) -> anyhow::Result<()> {
    // 1. Risolvi hostname → IP
    let mut allowed_ips: Vec<String> = vec![];
    for host in allow {
        match tokio::net::lookup_host(format!("{}:443", host)).await {
            Ok(addrs) => {
                for addr in addrs {
                    allowed_ips.push(addr.ip().to_string());
                }
            }
            Err(e) => {
                tracing::warn!("impossibile risolvere {}: {}. Host ignorato.", host, e);
            }
        }
    }

    // 2. Script iptables da eseguire nel container
    let rules = build_iptables_script(&allowed_ips);

    // 3. Esegui script nel container (richiede NET_ADMIN capability)
    // Nota: per ora loggare le regole senza applicarle se NET_ADMIN non disponibile
    tracing::info!("regole egress per {}: {:?}", container_id, allowed_ips);

    Ok(())
}

fn build_iptables_script(allowed_ips: &[String]) -> String {
    let mut s = String::from("#!/bin/sh\niptables -P OUTPUT DROP\n");
    s.push_str("iptables -A OUTPUT -o lo -j ACCEPT\n");
    for ip in allowed_ips {
        s.push_str(&format!("iptables -A OUTPUT -d {} -j ACCEPT\n", ip));
    }
    s
}
```

### 6.2 — Documentazione limiti obbligatoria

Aggiungere a `docs/spec-v1alpha1.md`:

```markdown
## Limiti noti di network.egress in v1alpha1

- La risoluzione DNS avviene una volta sola a startup della sandbox.
  Se un hostname cambia IP dopo la creazione, il nuovo IP non è nella allowlist.
- Il DNS rebinding non è prevenuto: un server esterno può rispondere con un IP
  interno dopo che la connessione è stata autorizzata.
- Wildcard nei hostname (*.example.com) non sono supportate e producono un errore.
- IP diretti nell'egress.allow producono un errore (usa hostname).

Questi limiti saranno rimossi in v1beta1 con l'introduzione di un proxy L4 dedicato.
```

---

## FASE 7 — Documentazione e getting started
**Stima:** 4-6 ore

### 7.1 — README.md root

```markdown
# AgentSandbox

Esegui codice di agenti LLM in sandbox isolate. Zero configurazione, zero vendor lock-in.

## Quickstart (30 secondi)

# 1. Avvia il daemon
cargo install agentsandbox-daemon
agentsandbox-daemon

# 2. Usa l'SDK Python
pip install agentsandbox

# Python
from agentsandbox import Sandbox
import asyncio

async def main():
    async with Sandbox(runtime="python", ttl=300) as sb:
        result = await sb.exec("python -c 'print(42)'")
        print(result.stdout)  # "42\n"

asyncio.run(main())
```

### 7.2 — Criteri di completamento Fase 7

- [ ] Un developer mai sentito parlare del progetto riesce a fare il quickstart in < 5 minuti
- [ ] Ogni limite noto è documentato (non nascosto)
- [ ] I codici errore dell'API sono documentati con esempi

---

## Checklist finale pre-release v0.1.0

**Stabilità:**
- [ ] Zero `unwrap()` in codice non-test nei tre crate principali
- [ ] Tutti gli errori pubblici hanno messaggi leggibili da un umano
- [ ] Il daemon non crasha in caso di Docker non disponibile (errore avvio, non panic)
- [ ] TTL reaper testato con sandbox scaduta

**Contratto pubblico:**
- [ ] La spec YAML è documentata con esempi
- [ ] L'API HTTP è documentata con curl examples
- [ ] Gli SDK hanno docstring su tutti i metodi pubblici
- [ ] CHANGELOG.md inizializzato

**Test coverage minima:**
- [ ] Core: compile pipeline (100% dei casi d'errore)
- [ ] Docker adapter: conformance suite completa
- [ ] SDK Python: test integration (create/exec/destroy)
- [ ] SDK TypeScript: test integration (create/exec/destroy)
- [ ] E2E: il "test di verità" (`runtime="python"`, exec, destroy) passa

---

## FASE 8 — Examples (branch `examples/`)
**Stima:** 1 giorno | **Prerequisiti:** Fase 4 e 5 completate e daemon funzionante

Questi esempi vivono nel branch `examples/` del repo e servono a tre scopi:
1. Verificare che il framework non abbia friction nascosta in uso reale
2. Dare al primo utente qualcosa da copiare e modificare in 5 minuti
3. Testare il "test di verità" del progetto in modo concreto e dimostrabile

---

### 8.1 — Esempio Python: Code Review Agent

**Cosa fa:** Prende un file Python in input, lo manda a Claude via API, Claude suggerisce fix, il fix viene eseguito nella sandbox per verificare che compili ed i test passino. Reale, utile, dimostrabile.

**Struttura:**
```
examples/
└── python-code-review-agent/
    ├── README.md
    ├── requirements.txt          # anthropic, agentsandbox
    ├── agent.py                  # entry point
    ├── sample_code/
    │   └── buggy_script.py       # script con bug intenzionali
    └── .env.example              # ANTHROPIC_API_KEY=...
```

**Codice completo `agent.py`:**

```python
"""
Code Review Agent — esempio AgentSandbox
----------------------------------------
Flusso:
1. Legge un file Python con potenziali bug
2. Chiede a Claude di identificare e correggere i problemi
3. Esegue il codice corretto in una sandbox isolata
4. Riporta: bug trovati, codice fixato, output dell'esecuzione

Prerequisiti:
    pip install anthropic agentsandbox
    export ANTHROPIC_API_KEY=...
    agentsandbox-daemon  # in un altro terminale
"""

import asyncio
import sys
from pathlib import Path
import anthropic
from agentsandbox import Sandbox

SYSTEM_PROMPT = """Sei un code reviewer esperto Python.
Quando ricevi codice, rispondi SEMPRE in questo formato JSON esatto, senza markdown:
{
  "bugs": ["descrizione bug 1", "descrizione bug 2"],
  "fixed_code": "codice Python corretto e completo",
  "explanation": "spiegazione breve dei fix"
}
Non aggiungere testo fuori dal JSON."""

async def review_and_run(filepath: str) -> None:
    source = Path(filepath).read_text()
    client = anthropic.Anthropic()

    print(f"📄 Revisione di: {filepath}")
    print("─" * 50)

    # 1. Chiedi a Claude di fare review
    print("🤖 Claude sta analizzando il codice...")
    message = client.messages.create(
        model="claude-opus-4-5",
        max_tokens=2048,
        system=SYSTEM_PROMPT,
        messages=[{
            "role": "user",
            "content": f"Analizza e correggi questo codice Python:\n\n```python\n{source}\n```"
        }]
    )

    import json
    try:
        review = json.loads(message.content[0].text)
    except json.JSONDecodeError:
        print("❌ Claude non ha risposto nel formato atteso")
        return

    # 2. Mostra i bug trovati
    print(f"\n🐛 Bug trovati ({len(review['bugs'])}):")
    for bug in review["bugs"]:
        print(f"   • {bug}")

    print(f"\n💡 Spiegazione: {review['explanation']}")

    # 3. Esegui il codice fixato in sandbox
    print("\n🔒 Esecuzione del codice fixato in sandbox isolata...")

    async with Sandbox(
        runtime="python",
        ttl=60,
        memory_mb=256,
        # Nessun egress: il codice non deve fare network calls
    ) as sb:
        # Scrivi il file fixato nella sandbox
        escaped = review["fixed_code"].replace("'", "'\\''")
        write_result = await sb.exec(f"cat > /workspace/script.py << 'PYEOF'\n{review['fixed_code']}\nPYEOF")

        # Esegui
        result = await sb.exec("python /workspace/script.py")

        print("\n📦 Output sandbox:")
        print("─" * 30)
        if result.stdout:
            print(result.stdout)
        if result.stderr:
            print(f"STDERR: {result.stderr}")
        print("─" * 30)

        if result.success:
            print(f"✅ Codice eseguito con successo (exit 0, {result.duration_ms}ms)")
        else:
            print(f"❌ Codice fallito con exit code {result.exit_code}")
            print("   Il fix di Claude potrebbe essere incompleto.")

if __name__ == "__main__":
    filepath = sys.argv[1] if len(sys.argv) > 1 else "sample_code/buggy_script.py"
    asyncio.run(review_and_run(filepath))
```

**`sample_code/buggy_script.py`** (file con bug intenzionali — Claude deve trovarli):

```python
# Script con bug intenzionali per testare il code review agent

def calculate_average(numbers):
    # Bug 1: divisione per zero non gestita
    return sum(numbers) / len(numbers)

def find_duplicates(items):
    duplicates = []
    for i in range(len(items)):
        for j in range(len(items)):
            # Bug 2: confronta ogni elemento con se stesso
            if items[i] == items[j]:
                duplicates.append(items[i])
    return duplicates

def parse_config(config_str):
    # Bug 3: split senza strip, crea chiavi con spazi
    result = {}
    for line in config_str.split('\n'):
        key, value = line.split('=')
        result[key] = value
    return result

if __name__ == "__main__":
    print(calculate_average([1, 2, 3, 4, 5]))
    print(calculate_average([]))  # questo crasherà

    items = [1, 2, 2, 3, 3, 3]
    print(find_duplicates(items))

    config = "host = localhost\nport = 8080\n"
    print(parse_config(config))
```

**`README.md` dell'esempio:**

```markdown
# Code Review Agent — AgentSandbox Example

Un agente che usa Claude per fare code review e verifica i fix in sandbox isolata.

## Setup (2 minuti)

```bash
# 1. Avvia il daemon AgentSandbox
agentsandbox-daemon &

# 2. Installa le dipendenze
pip install anthropic agentsandbox

# 3. Configura la API key
export ANTHROPIC_API_KEY=sk-ant-...

# 4. Esegui
python agent.py sample_code/buggy_script.py
```

## Output atteso

```
📄 Revisione di: sample_code/buggy_script.py
──────────────────────────────────────────────
🤖 Claude sta analizzando il codice...

🐛 Bug trovati (3):
   • Divisione per zero quando la lista è vuota
   • find_duplicates confronta ogni elemento con se stesso
   • parse_config non fa strip delle chiavi

💡 Spiegazione: ...

🔒 Esecuzione del codice fixato in sandbox isolata...

📦 Output sandbox:
──────────────────
3.0
[2, 3]
{'host': 'localhost', 'port': '8080'}
──────────────────
✅ Codice eseguito con successo (exit 0, 312ms)
```

## Cosa dimostra

- Integrazione Claude API + AgentSandbox in < 80 righe
- Il codice generato da Claude viene eseguito in sandbox isolata (nessun accesso rete, memoria limitata)
- Il codice host non viene mai toccato
```

---

### 8.2 — Esempio TypeScript: Dependency Auditor Agent

**Cosa fa:** Prende un `package.json`, installa le dipendenze in sandbox isolata con accesso solo a registry.npmjs.org, esegue `npm audit`, ritorna un report strutturato. Utile in CI, dimostra egress filtering reale.

**Struttura:**
```
examples/
└── ts-dependency-auditor/
    ├── README.md
    ├── package.json
    ├── tsconfig.json
    ├── src/
    │   └── auditor.ts
    └── sample/
        └── package.json          # package con dipendenze note vulnerabili
```

**`src/auditor.ts`:**

```typescript
/**
 * Dependency Auditor Agent — AgentSandbox TypeScript Example
 *
 * Flusso:
 * 1. Legge un package.json
 * 2. Installa le dipendenze in sandbox con egress solo su npmjs.org
 * 3. Esegue npm audit
 * 4. Invia il report a Claude per una summary human-readable
 * 5. Stampa il report finale
 *
 * Prerequisiti:
 *   npm install
 *   ANTHROPIC_API_KEY=... npx ts-node src/auditor.ts sample/package.json
 */

import { Sandbox } from 'agentsandbox';
import Anthropic from '@anthropic-ai/sdk';
import * as fs from 'fs';
import * as path from 'path';

const anthropic = new Anthropic();

interface AuditReport {
  vulnerabilities: number;
  critical: number;
  high: number;
  raw: string;
  summary: string;
}

async function auditDependencies(packageJsonPath: string): Promise<AuditReport> {
  const packageJson = fs.readFileSync(packageJsonPath, 'utf-8');

  console.log(`📦 Audit di: ${packageJsonPath}`);
  console.log('─'.repeat(50));

  // 1. Crea sandbox con egress limitato solo a npm registry
  console.log('🔒 Creazione sandbox isolata (egress: npmjs.org only)...');

  const sb = await Sandbox.create({
    runtime: 'node',
    ttl: 120,
    memoryMb: 512,
    egress: [
      'registry.npmjs.org',
      'registry.yarnpkg.com',
    ],
  });

  try {
    // 2. Copia il package.json nella sandbox
    const escaped = packageJson.replace(/'/g, "'\\''");
    await sb.exec(`mkdir -p /workspace && cat > /workspace/package.json << 'EOF'\n${packageJson}\nEOF`);

    // 3. Installa dipendenze
    console.log('⬇️  Installazione dipendenze in sandbox...');
    const installResult = await sb.exec('cd /workspace && npm install --prefer-offline 2>&1');

    if (!installResult.success) {
      throw new Error(`npm install fallito:\n${installResult.stderr}`);
    }

    // 4. Esegui npm audit
    console.log('🔍 Esecuzione npm audit...');
    const auditResult = await sb.exec('cd /workspace && npm audit --json 2>&1 || true');
    // npm audit ritorna exit code != 0 se trova vulnerabilità, quindi || true

    let auditData: any = {};
    try {
      auditData = JSON.parse(auditResult.stdout);
    } catch {
      // Se non è JSON valido, usa l'output raw
      auditData = { raw: auditResult.stdout };
    }

    const vulnMeta = auditData.metadata?.vulnerabilities ?? {};
    const total = Object.values(vulnMeta).reduce((a: number, b: any) => a + b, 0) as number;

    // 5. Chiedi a Claude una summary
    console.log('🤖 Claude sta analizzando il report...');
    const message = await anthropic.messages.create({
      model: 'claude-opus-4-5',
      max_tokens: 512,
      messages: [{
        role: 'user',
        content: `Questo è l'output di npm audit per un progetto Node.js.
Scrivi una summary in 3-5 righe: quante vulnerabilità ci sono, quali sono le più critiche e cosa fare.
Sii diretto e pratico. Output in italiano.

\`\`\`json
${auditResult.stdout.slice(0, 3000)}
\`\`\``,
      }],
    });

    const summary = message.content[0].type === 'text' ? message.content[0].text : '';

    return {
      vulnerabilities: total,
      critical: vulnMeta.critical ?? 0,
      high: vulnMeta.high ?? 0,
      raw: auditResult.stdout,
      summary,
    };

  } finally {
    await sb.destroy();
  }
}

async function main() {
  const packageJsonPath = process.argv[2] ?? 'sample/package.json';

  if (!fs.existsSync(packageJsonPath)) {
    console.error(`❌ File non trovato: ${packageJsonPath}`);
    process.exit(1);
  }

  try {
    const report = await auditDependencies(packageJsonPath);

    console.log('\n📊 Report finale:');
    console.log('─'.repeat(50));
    console.log(`Vulnerabilità totali: ${report.vulnerabilities}`);
    console.log(`  Critical: ${report.critical}`);
    console.log(`  High:     ${report.high}`);
    console.log('\n🤖 Analisi Claude:');
    console.log(report.summary);

    if (report.critical > 0) {
      process.exit(1); // Usabile in CI
    }
  } catch (err) {
    console.error('❌ Errore:', err);
    process.exit(1);
  }
}

main();
```

**`sample/package.json`** (dipendenze con vulnerabilità note per demo):

```json
{
  "name": "sample-audit-target",
  "version": "1.0.0",
  "dependencies": {
    "lodash": "4.17.15",
    "axios": "0.21.1",
    "express": "4.17.1"
  }
}
```

---

### 8.3 — Struttura finale del branch examples

```
examples/
├── README.md                          # indice di tutti gli esempi
├── python-code-review-agent/
│   ├── README.md
│   ├── requirements.txt
│   ├── agent.py
│   └── sample_code/
│       └── buggy_script.py
└── ts-dependency-auditor/
    ├── README.md
    ├── package.json
    ├── tsconfig.json
    ├── src/
    │   └── auditor.ts
    └── sample/
        └── package.json
```

**`examples/README.md`:**

```markdown
# AgentSandbox — Examples

Esempi reali e funzionanti che mostrano AgentSandbox in uso.

| Esempio | SDK | Cosa fa | Egress |
|---|---|---|---|
| [Code Review Agent](./python-code-review-agent) | Python | Usa Claude per fare code review, verifica i fix in sandbox | Nessuno |
| [Dependency Auditor](./ts-dependency-auditor) | TypeScript | Audit npm in sandbox isolata, summary Claude | npmjs.org |

## Prerequisiti comuni

1. Docker running
2. AgentSandbox daemon: `agentsandbox-daemon`
3. `ANTHROPIC_API_KEY` settata

## Vuoi contribuire un esempio?

Un buon esempio AgentSandbox:
- Fa qualcosa di reale (non hello world)
- Dimostra almeno una feature del framework (egress, TTL, secrets, runtime specifico)
- Funziona in < 5 minuti dall'installazione
- Ha un README con output atteso
```

---

### 8.4 — Criteri di completamento Fase 8

- [ ] `python agent.py sample_code/buggy_script.py` produce output con bug trovati + exec sandbox
- [ ] `npx ts-node src/auditor.ts sample/package.json` produce report con vulnerabilità
- [ ] Entrambi gli esempi funzionano con daemon fresh start (no stato precedente)
- [ ] L'esempio TypeScript esce con code 1 se trova vulnerabilità critiche (usabile in CI)
- [ ] Il README di ogni esempio include l'output atteso letterale (non inventato)
- [ ] Nessun esempio richiede configurazione oltre `ANTHROPIC_API_KEY` e Docker

---

## Note operative per Claude Code

1. **Implementa una fase alla volta.** Non iniziare la Fase 2 prima che i criteri di completamento della Fase 1 siano verificati.

2. **Mai `unwrap()` in codice di produzione.** Ogni errore deve propagarsi con `?` o essere gestito esplicitamente.

3. **I secret non entrano mai nei log.** Prima di aggiungere qualsiasi `tracing::debug!` su struct che potrebbero contenere secret, verifica che il campo `secret_env` sia escluso dal `Debug` derive o sia oscurato.

4. **Nessun fallback silenzioso.** Se un'operazione non può completare, ritorna errore. Mai degradare silenziosamente (es. ignorare egress rules senza log).

5. **Testa prima il contratto, poi l'implementazione.** Scrivi i test della conformance suite prima dell'implementazione dell'adapter.

6. **Il file `spec/sandbox.v1alpha1.schema.json` va aggiornato** ogni volta che `SandboxSpec` cambia. Usa `schemars` per generarlo automaticamente dal tipo Rust.