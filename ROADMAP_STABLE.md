# AgentSandbox — Roadmap v1beta1 → v1stable
> Documento sequenziale al roadmap v1alpha1. Prerequisito: tutte le Fasi 0-8 del documento precedente completate e passanti.
> **Obiettivo di questo documento:** portare il progetto da "funziona in locale" a "qualcuno può scrivere un terzo backend senza toccare il core".

---

## Cambio di prospettiva rispetto a v1alpha1

In v1alpha1 il criterio di successo era:
> "un agente può chiedere una sandbox e ottenerla senza sapere cosa c'è sotto"

In v1stable il criterio di successo è doppio:

**Per il consumer (invariato):**
```python
async with Sandbox(runtime="python", ttl=900) as sb:
    result = await sb.exec("python script.py")
```

**Per il contributor di un nuovo backend (nuovo):**
```rust
// Un contributor esterno deve poter scrivere questo
// e ottenere un backend funzionante senza modificare
// nessun file nel core del progetto.

pub struct NixBackend { /* ... */ }

impl SandboxBackend for NixBackend {
    // implementa il trait — fine
}

// Registra il backend via feature flag o plugin manifest
// e il sistema lo trova automaticamente
```

Il test di verità del contributor: clona il repo, legge `BACKEND_GUIDE.md`, implementa il trait, passa la conformance suite, apre una PR con solo il nuovo crate. Zero modifiche al core.

---

## Mappa delle fasi

```
FASE A — Stabilizzazione API pubblica (spec v1beta1)
FASE B — Plugin architecture per backend esterni
FASE C — Egress filtering reale (proxy L4)
FASE D — Firecracker backend (come primo backend esterno)
FASE E — Multi-tenancy e autenticazione
FASE F — Osservabilità production-grade
FASE G — SDK v1.0 con breaking change policy
FASE H — Release checklist e governance
```

---

## FASE A — Spec v1beta1 e stabilizzazione API
**Stima:** 3-4 giorni
**Obiettivo:** congelare il contratto pubblico prima di aggiungere complessità. Ogni cosa aggiunta dopo è additiva, mai breaking.

### A.1 — Diff tra v1alpha1 e v1beta1

Le modifiche sono **esclusivamente additive**. Nessun campo v1alpha1 viene rimosso o rinominato.

```yaml
# v1alpha1 — invariato, continua a funzionare
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata:
  name: my-sandbox
spec:
  runtime:
    preset: python
  ttlSeconds: 300

---

# v1beta1 — nuovi campi opzionali
apiVersion: sandbox.ai/v1beta1
kind: Sandbox
metadata:
  name: my-sandbox
  labels:
    team: platform
    env: production
spec:
  runtime:
    preset: python
    version: "3.12"          # NUOVO: versione pinned del preset
  resources:
    cpuMillicores: 1000
    memoryMb: 512
    diskMb: 1024
    timeoutMs: 30000         # NUOVO: timeout per singola exec
  network:
    egress:
      allow: ["pypi.org"]
      denyByDefault: true
      mode: proxy            # NUOVO: "none" | "proxy" | "passthrough"
  scheduling:
    backend: docker          # NUOVO: hint esplicito al backend selector
    preferWarm: false
    priority: normal         # NUOVO: "low" | "normal" | "high"
  storage:                   # NUOVO: volumi opzionali
    volumes: []
  observability:             # NUOVO: controllo su cosa viene emesso
    auditLevel: basic        # "none" | "basic" | "full"
    metricsEnabled: false
  ttlSeconds: 300
```

### A.2 — Versioning della spec nel compile pipeline

```rust
// crates/agentsandbox-core/src/compile.rs

pub enum SpecVersion {
    V1Alpha1,
    V1Beta1,
}

pub fn detect_version(raw: &serde_json::Value) -> Result<SpecVersion, CompileError> {
    match raw.get("apiVersion").and_then(|v| v.as_str()) {
        Some("sandbox.ai/v1alpha1") => Ok(SpecVersion::V1Alpha1),
        Some("sandbox.ai/v1beta1")  => Ok(SpecVersion::V1Beta1),
        Some(v) => Err(CompileError::UnsupportedApiVersion(v.to_string())),
        None    => Err(CompileError::MissingApiVersion),
    }
}

/// Entry point unificato: accetta entrambe le versioni,
/// normalizza sempre a IR corrente.
pub fn compile_any(raw: &str) -> Result<SandboxIR, CompileError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .or_else(|_| serde_yaml::from_str::<serde_json::Value>(raw))
        .map_err(|e| CompileError::ParseError(e.to_string()))?;

    match detect_version(&value)? {
        SpecVersion::V1Alpha1 => {
            let spec: SpecV1Alpha1 = serde_json::from_value(value)
                .map_err(|e| CompileError::ValidationError(e.to_string()))?;
            compile_v1alpha1(spec)
        }
        SpecVersion::V1Beta1 => {
            let spec: SpecV1Beta1 = serde_json::from_value(value)
                .map_err(|e| CompileError::ValidationError(e.to_string()))?;
            compile_v1beta1(spec)
        }
    }
}
```

### A.3 — JSON Schema ufficiale e validation middleware

```rust
// crates/agentsandbox-core/src/schema.rs
// Usa `jsonschema` crate per validazione formale prima del parsing.

use jsonschema::JSONSchema;
use std::sync::OnceLock;

static SCHEMA_V1BETA1: OnceLock<JSONSchema> = OnceLock::new();

pub fn schema_v1beta1() -> &'static JSONSchema {
    SCHEMA_V1BETA1.get_or_init(|| {
        let raw = include_str!("../../spec/sandbox.v1beta1.schema.json");
        let schema: serde_json::Value = serde_json::from_str(raw).unwrap();
        JSONSchema::compile(&schema).unwrap()
    })
}

pub fn validate_raw(raw: &serde_json::Value) -> Result<(), Vec<String>> {
    let result = schema_v1beta1().validate(raw);
    match result {
        Ok(_) => Ok(()),
        Err(errors) => Err(errors.map(|e| e.to_string()).collect()),
    }
}
```

### A.4 — Criteri di completamento Fase A

- [ ] `compile_any()` accetta sia v1alpha1 che v1beta1 senza breaking change
- [ ] `POST /v1/sandboxes` con spec v1alpha1 esistente continua a funzionare invariato
- [ ] JSON Schema v1beta1 generato e committato in `spec/`
- [ ] Validation middleware nel daemon ritorna errori strutturati per ogni campo invalido
- [ ] `CHANGELOG.md` documenta ogni differenza tra v1alpha1 e v1beta1

---

## FASE B — Plugin Architecture per backend esterni
**Stima:** 4-5 giorni
**Questa è la fase più importante del documento.**

### B.1 — Principio architetturale

Il sistema deve permettere a un contributor esterno di:
1. Creare un nuovo crate Rust
2. Implementare un trait pubblico e stabile
3. Passare la conformance suite standard
4. Registrare il backend **senza modificare nessun file del core**

Questo si ottiene con tre meccanismi combinati:
- Un **trait pubblico e versionato** (`SandboxBackend`)
- Un **registry runtime** che scopre i backend disponibili
- Un **manifest di backend** (`backend.toml`) che descrive le capability

### B.2 — Il trait pubblico stabile

Questo è il contratto che non cambierà mai senza un major version bump.
Ogni backend esterno implementa esattamente questo.

```rust
// crates/agentsandbox-sdk/src/backend.rs
// Questo crate è SEPARATO dal core — è la superficie pubblica per i contributor.
// Versione: 1.0.0 — Breaking changes solo con major bump.

use async_trait::async_trait;
use std::collections::HashMap;

/// Versione del trait. Un backend deve dichiarare quale versione implementa.
/// Il daemon rifiuta backend con versione incompatibile con un errore esplicito.
pub const BACKEND_TRAIT_VERSION: &str = "1.0";

/// Descriptor statico del backend. Ritornato da `BackendFactory::describe()`.
/// Non richiede una connessione attiva — deve funzionare sempre.
#[derive(Debug, Clone)]
pub struct BackendDescriptor {
    /// Identificatore univoco. Usato nella spec come `scheduling.backend`.
    /// Esempi: "docker", "firecracker", "nix", "wasm", "gvisor"
    pub id: &'static str,

    /// Nome human-readable per UI e log.
    pub display_name: &'static str,

    /// Versione del backend stesso (non del trait).
    pub version: &'static str,

    /// Versione del trait che questo backend implementa.
    pub trait_version: &'static str,

    /// Capability dichiarate. Il backend selector usa queste per il matching.
    pub capabilities: BackendCapabilities,
}

/// Capability che un backend può dichiarare.
/// Il backend selector usa queste per decidere se un backend
/// può soddisfare una spec prima ancora di creare la sandbox.
#[derive(Debug, Clone, Default)]
pub struct BackendCapabilities {
    /// Supporta isolamento di rete (egress filtering)?
    pub network_isolation: bool,

    /// Supporta limiti di memoria garantiti (non best-effort)?
    pub memory_hard_limit: bool,

    /// Supporta limiti di CPU garantiti?
    pub cpu_hard_limit: bool,

    /// Supporta storage persistente tra exec sulla stessa sandbox?
    pub persistent_storage: bool,

    /// Il backend può fare health check senza dipendenze esterne?
    pub self_contained: bool,

    /// Livello di isolamento del backend.
    pub isolation_level: IsolationLevel,

    /// Preset runtime supportati. Se vuoto, accetta qualsiasi `runtime.image`.
    pub supported_presets: Vec<&'static str>,

    /// Metadata aggiuntivi specifici del backend (non interpretati dal core).
    pub extra: HashMap<&'static str, &'static str>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum IsolationLevel {
    #[default]
    Process,         // subprocess con namespace (minimo)
    Container,       // container (Docker, Podman)
    MicroVM,         // microVM (Firecracker, Cloud Hypervisor)
    Wasm,            // WASM sandbox
    HardwareVM,      // VM completa (QEMU, HyperKit)
}

/// Intermediate Representation che il backend riceve.
/// Contiene tutto ciò che serve per creare la sandbox.
/// I secret sono già risolti — il backend non deve fare lookup.
#[derive(Debug, Clone)]
pub struct SandboxIR {
    pub id: String,
    pub image: String,
    pub env: Vec<(String, String)>,
    pub secret_env: Vec<(String, String)>,  // già risolti, non loggare mai
    pub cpu_millicores: u32,
    pub memory_mb: u32,
    pub disk_mb: u32,
    pub egress: EgressIR,
    pub ttl_seconds: u64,
    pub timeout_ms: u64,
    pub working_dir: String,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct EgressIR {
    pub mode: EgressMode,
    pub allow_hostnames: Vec<String>,
    pub allow_ips: Vec<String>,   // pre-risolti dal compile pipeline
    pub deny_by_default: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EgressMode {
    None,         // --network=none
    Proxy,        // traffico filtrato via proxy interno
    Passthrough,  // nessun filtro (development only)
}

/// Risultato di una exec.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
    pub resource_usage: Option<ResourceUsage>,
}

/// Utilizzo risorse opzionale — se il backend non può misurarlo,
/// ritorna None. Mai inventare valori.
#[derive(Debug, Clone)]
pub struct ResourceUsage {
    pub cpu_user_ms: Option<u64>,
    pub memory_peak_mb: Option<u64>,
    pub disk_read_bytes: Option<u64>,
    pub disk_write_bytes: Option<u64>,
}

/// Stato corrente della sandbox.
#[derive(Debug, Clone)]
pub struct SandboxStatus {
    pub sandbox_id: String,
    pub state: SandboxState,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub backend_id: String,
    /// Handle nativo del backend — SOLO per uso interno del backend stesso.
    /// MAI esposto fuori dal crate del backend.
    pub(crate) native_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SandboxState {
    Creating,
    Running,
    Stopped,
    Failed(String),
    Expired,
}

/// Errori che un backend può ritornare.
/// Il core mappa questi su codici HTTP e messaggi pubblici.
#[derive(thiserror::Error, Debug)]
pub enum BackendError {
    #[error("sandbox non trovata: {0}")]
    NotFound(String),

    #[error("backend non disponibile: {0}")]
    Unavailable(String),

    #[error("risorsa insufficiente: {0}")]
    ResourceExhausted(String),

    #[error("operazione non supportata da questo backend: {0}")]
    NotSupported(String),

    #[error("timeout dopo {0}ms")]
    Timeout(u64),

    #[error("errore di configurazione del backend: {0}")]
    Configuration(String),

    #[error("errore interno del backend: {0}")]
    Internal(String),
}

// ─────────────────────────────────────────────────────────────────
// IL TRAIT CHE OGNI BACKEND IMPLEMENTA
// Questo è il contratto pubblico. Stabile dalla v1.0.
// ─────────────────────────────────────────────────────────────────

/// Factory per creare istanze del backend.
/// Separato dal trait principale per permettere al registry
/// di creare backend senza conoscerne il tipo concreto.
pub trait BackendFactory: Send + Sync {
    /// Descriptor statico — non richiede connessione.
    fn describe(&self) -> BackendDescriptor;

    /// Crea un'istanza del backend con la configurazione fornita.
    /// `config` è la sezione `backends.<id>` del file di configurazione del daemon.
    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError>;
}

/// Il backend principale. Ogni implementazione è stateless rispetto
/// alle sandbox — lo stato è in SQLite, non nel backend.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    // ── Lifecycle ──────────────────────────────────────────────

    /// Crea e avvia la sandbox. Ritorna l'ID interno del backend
    /// (diverso dal sandbox_id del daemon — può essere un container ID,
    /// un path WASM, un VM ID, ecc.)
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError>;

    /// Esegue un comando nella sandbox.
    /// Il comando è una stringa shell interpretata dal backend.
    async fn exec(
        &self,
        backend_handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError>;

    /// Ritorna lo stato corrente della sandbox.
    async fn status(&self, backend_handle: &str) -> Result<SandboxStatus, BackendError>;

    /// Distrugge la sandbox e libera tutte le risorse.
    /// Idempotente: chiamare destroy su una sandbox già distrutta è Ok(()).
    async fn destroy(&self, backend_handle: &str) -> Result<(), BackendError>;

    // ── Capability check ───────────────────────────────────────

    /// Verifica che il backend sia disponibile e operativo.
    /// Chiamato al daemon startup e periodicamente dal health checker.
    async fn health_check(&self) -> Result<(), BackendError>;

    /// Verifica se questo backend può soddisfare l'IR fornita.
    /// Chiamato prima di create() per fail-fast su incompatibilità.
    /// Default: Ok(()) — override per aggiungere check specifici.
    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        let _ = ir;
        Ok(())
    }

    // ── Opzionali — override solo se supportati ────────────────

    /// Copia un file nell'ambiente della sandbox.
    /// Default: BackendError::NotSupported
    async fn upload_file(
        &self,
        backend_handle: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), BackendError> {
        let _ = (backend_handle, path, content);
        Err(BackendError::NotSupported("upload_file".into()))
    }

    /// Legge un file dall'ambiente della sandbox.
    /// Default: BackendError::NotSupported
    async fn download_file(
        &self,
        backend_handle: &str,
        path: &str,
    ) -> Result<Vec<u8>, BackendError> {
        let _ = (backend_handle, path);
        Err(BackendError::NotSupported("download_file".into()))
    }

    /// Snapshot della sandbox per warm pool.
    /// Default: BackendError::NotSupported
    async fn snapshot(&self, backend_handle: &str) -> Result<String, BackendError> {
        let _ = backend_handle;
        Err(BackendError::NotSupported("snapshot".into()))
    }

    /// Restore da snapshot per warm pool.
    /// Default: BackendError::NotSupported
    async fn restore(&self, snapshot_id: &str, ir: &SandboxIR) -> Result<String, BackendError> {
        let _ = (snapshot_id, ir);
        Err(BackendError::NotSupported("restore".into()))
    }
}
```

### B.3 — Backend Registry

Il registry è il meccanismo che connette i backend al daemon senza hardcoding.

```rust
// crates/agentsandbox-daemon/src/registry.rs

use agentsandbox_sdk::backend::{BackendFactory, SandboxBackend, BackendDescriptor};
use std::collections::HashMap;
use std::sync::Arc;

/// Registry globale dei backend disponibili.
/// Viene popolato al startup del daemon tramite `register()`.
pub struct BackendRegistry {
    factories: HashMap<String, Arc<dyn BackendFactory>>,
    instances: HashMap<String, Arc<dyn SandboxBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            instances: HashMap::new(),
        }
    }

    /// Registra un backend factory.
    /// Chiamato al startup per ogni backend disponibile.
    pub fn register(&mut self, factory: Arc<dyn BackendFactory>) {
        let desc = factory.describe();
        tracing::info!(
            "backend registrato: {} ({}) — trait v{}",
            desc.id,
            desc.display_name,
            desc.trait_version
        );
        self.factories.insert(desc.id.to_string(), factory);
    }

    /// Inizializza tutti i backend registrati con la configurazione.
    /// Un backend che fallisce l'inizializzazione viene loggato ma non blocca gli altri.
    pub async fn initialize(
        &mut self,
        config: &DaemonConfig,
    ) -> Vec<String> {
        let mut failed = vec![];

        for (id, factory) in &self.factories {
            let backend_config = config.backends.get(id).cloned().unwrap_or_default();

            match factory.create(&backend_config) {
                Ok(backend) => {
                    // Health check prima di aggiungere
                    match backend.health_check().await {
                        Ok(_) => {
                            tracing::info!("backend {} inizializzato e healthy", id);
                            self.instances.insert(id.clone(), Arc::from(backend));
                        }
                        Err(e) => {
                            tracing::warn!("backend {} health check fallito: {} — non disponibile", id, e);
                            failed.push(id.clone());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("backend {} inizializzazione fallita: {} — non disponibile", id, e);
                    failed.push(id.clone());
                }
            }
        }

        failed
    }

    /// Ritorna il backend selezionato per una IR.
    /// Mai ritorna un backend non healthy.
    pub fn select(&self, ir: &SandboxIR) -> Result<Arc<dyn SandboxBackend>, RegistryError> {
        BackendSelector::select(ir, &self.instances)
    }

    pub fn list_available(&self) -> Vec<BackendDescriptor> {
        self.factories.values().map(|f| f.describe()).collect()
    }
}

/// Errori del registry.
#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("nessun backend disponibile per la spec fornita")]
    NoBackendAvailable,

    #[error("backend '{0}' richiesto esplicitamente ma non disponibile")]
    RequestedBackendUnavailable(String),

    #[error("backend '{0}' non soddisfa i requisiti: {1}")]
    BackendIncompatible(String, String),
}
```

### B.4 — Backend Selector (regole esplicite, zero fallback silenzioso)

```rust
// crates/agentsandbox-daemon/src/selector.rs

pub struct BackendSelector;

impl BackendSelector {
    pub fn select(
        ir: &SandboxIR,
        available: &HashMap<String, Arc<dyn SandboxBackend>>,
    ) -> Result<Arc<dyn SandboxBackend>, RegistryError> {

        // Regola 1: se la spec ha un backend hint esplicito, usalo o fallisci.
        if let Some(requested) = &ir.backend_hint {
            return available
                .get(requested)
                .cloned()
                .ok_or_else(|| RegistryError::RequestedBackendUnavailable(requested.clone()));
        }

        // Regola 2: se egress.mode == Proxy, richiede network_isolation.
        let needs_network_isolation = ir.egress.mode == EgressMode::Proxy;

        // Regola 3: se memory_mb > 4096, preferisci MicroVM.
        let prefers_microvm = ir.memory_mb > 4096;

        // Costruisci lista candidati con score.
        let mut candidates: Vec<(u32, Arc<dyn SandboxBackend>)> = available
            .values()
            .filter_map(|backend| {
                // TODO: recuperare le capabilities dal descriptor
                // Per ora usiamo il backend_name come proxy
                Some((1u32, backend.clone()))
            })
            .collect();

        if candidates.is_empty() {
            return Err(RegistryError::NoBackendAvailable);
        }

        // Ordina per score decrescente, prendi il primo.
        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(candidates.remove(0).1)
    }
}
```

### B.5 — Configurazione daemon con backend multipli

```toml
# agentsandbox.toml — configurazione del daemon

[daemon]
host = "127.0.0.1"
port = 7847
database_url = "sqlite://agentsandbox.db"
log_level = "info"

# Backend attivi. L'ordine non conta — usa il selector.
# Commentare un backend lo disabilita senza rimuoverlo dalla config.
[backends]
enabled = ["docker", "firecracker"]

[backends.docker]
socket = "/var/run/docker.sock"
pull_policy = "if_not_present"   # "always" | "if_not_present" | "never"
network_bridge = "agentsandbox0"

[backends.firecracker]
binary_path = "/usr/local/bin/firecracker"
kernel_image = "/var/lib/agentsandbox/vmlinux"
rootfs_dir = "/var/lib/agentsandbox/rootfs"
jailer_path = "/usr/local/bin/jailer"
require_kvm = true

# Backend di terze parti: stessa struttura, nome arbitrario
[backends.my-nix-backend]
nix_path = "/nix"
store_prefix = "/nix/store"
```

### B.6 — Startup del daemon con registry

```rust
// crates/agentsandbox-daemon/src/main.rs (aggiornato)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = DaemonConfig::load("agentsandbox.toml")?;

    let mut registry = BackendRegistry::new();

    // Backend built-in — sempre disponibili se compilati
    #[cfg(feature = "backend-docker")]
    registry.register(Arc::new(agentsandbox_docker::DockerBackendFactory::new()));

    #[cfg(feature = "backend-firecracker")]
    registry.register(Arc::new(agentsandbox_firecracker::FirecrackerBackendFactory::new()));

    // Backend di terze parti — caricati se presenti in Cargo.toml
    // (vedi sezione B.7 per il meccanismo di plugin)

    let failed = registry.initialize(&config).await;
    if failed.len() == config.backends.enabled.len() {
        anyhow::bail!("tutti i backend configurati non sono disponibili: {:?}", failed);
    }

    let state = Arc::new(AppState {
        db: sqlx::SqlitePool::connect(&config.daemon.database_url).await?,
        registry: Arc::new(registry),
        config,
    });

    // ... resto del setup axum invariato
}
```

### B.7 — Come un contributor implementa un backend esterno

Questa sezione diventa `BACKEND_GUIDE.md` nel repo.

**Struttura del crate esterno:**

```
agentsandbox-backend-nix/          ← nome convenzionale ma non obbligatorio
├── Cargo.toml
├── backend.toml                   ← manifest del backend
├── src/
│   ├── lib.rs
│   ├── factory.rs
│   └── backend.rs
├── conformance/
│   └── conformance_test.rs        ← OBBLIGATORIO
└── README.md
```

**`Cargo.toml` del backend esterno:**

```toml
[package]
name = "agentsandbox-backend-nix"
version = "0.1.0"
edition = "2021"

[dependencies]
# L'unica dipendenza dal progetto principale è l'SDK pubblico
agentsandbox-sdk = { version = "1.0", features = [] }
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
# ... dipendenze specifiche del backend

[dev-dependencies]
# Per la conformance suite
agentsandbox-conformance = { version = "1.0" }
```

**`src/factory.rs`:**

```rust
use agentsandbox_sdk::backend::{
    BackendFactory, BackendDescriptor, BackendCapabilities,
    IsolationLevel, SandboxBackend, BackendError, BACKEND_TRAIT_VERSION,
};
use std::collections::HashMap;

pub struct NixBackendFactory;

impl BackendFactory for NixBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "nix",
            display_name: "Nix Sandbox",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: false,
                memory_hard_limit: false,
                cpu_hard_limit: false,
                persistent_storage: true,
                self_contained: true,
                isolation_level: IsolationLevel::Process,
                supported_presets: vec!["python", "node", "rust"],
                extra: HashMap::from([
                    ("nix_version", "2.18"),
                ]),
            },
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let nix_path = config.get("nix_path")
            .ok_or_else(|| BackendError::Configuration("nix_path richiesto".into()))?;

        Ok(Box::new(NixBackend {
            nix_path: nix_path.clone(),
        }))
    }
}
```

**`src/backend.rs` (scheletro da completare):**

```rust
use agentsandbox_sdk::backend::*;
use async_trait::async_trait;

pub struct NixBackend {
    pub nix_path: String,
}

#[async_trait]
impl SandboxBackend for NixBackend {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        // Implementa con nix-shell o nix develop
        todo!()
    }

    async fn exec(
        &self,
        backend_handle: &str,
        command: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError> {
        todo!()
    }

    async fn status(&self, backend_handle: &str) -> Result<SandboxStatus, BackendError> {
        todo!()
    }

    async fn destroy(&self, backend_handle: &str) -> Result<(), BackendError> {
        todo!()
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        // Verifica che nix sia nel path e funzionante
        let output = tokio::process::Command::new(&self.nix_path)
            .arg("--version")
            .output()
            .await
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(BackendError::Unavailable("nix --version fallito".into()))
        }
    }
}
```

**`conformance/conformance_test.rs` (OBBLIGATORIO per ogni backend):**

```rust
// Ogni backend DEVE includere questo file e passare tutti i test.
// Non è opzionale. Una PR senza conformance test non viene accettata.

use agentsandbox_conformance::ConformanceSuite;
use agentsandbox_backend_nix::NixBackendFactory;
use agentsandbox_sdk::backend::BackendFactory;
use std::collections::HashMap;

async fn make_backend() -> Box<dyn agentsandbox_sdk::backend::SandboxBackend> {
    let factory = NixBackendFactory;
    let config = HashMap::from([
        ("nix_path".to_string(), "/nix/bin/nix".to_string()),
    ]);
    factory.create(&config).expect("factory deve funzionare")
}

// Macro che espande in tutti i test della conformance suite.
// Se la suite viene aggiornata, i backend che non passano
// vengono evidenziati automaticamente.
agentsandbox_conformance::run_conformance_suite!(make_backend);
```

### B.8 — Crate `agentsandbox-conformance` (nuovo)

```rust
// crates/agentsandbox-conformance/src/lib.rs
// Questo crate è il contratto di qualità per tutti i backend.

use agentsandbox_sdk::backend::*;

pub struct ConformanceSuite<F, Fut>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Box<dyn SandboxBackend>>,
{
    make_backend: F,
}

impl<F, Fut> ConformanceSuite<F, Fut>
where
    F: Fn() -> Fut + Clone,
    Fut: std::future::Future<Output = Box<dyn SandboxBackend>>,
{
    pub fn new(make_backend: F) -> Self {
        Self { make_backend }
    }

    pub async fn run_all(&self) -> ConformanceReport {
        let mut report = ConformanceReport::new();

        report.run("health_check", self.test_health_check()).await;
        report.run("create_returns_handle", self.test_create_returns_handle()).await;
        report.run("exec_echo", self.test_exec_echo()).await;
        report.run("exec_exit_code", self.test_exec_exit_code()).await;
        report.run("exec_stderr", self.test_exec_stderr()).await;
        report.run("status_running", self.test_status_running()).await;
        report.run("destroy_idempotent", self.test_destroy_idempotent()).await;
        report.run("destroy_cleans_resources", self.test_destroy_cleans_resources()).await;
        report.run("concurrent_sandboxes", self.test_concurrent_sandboxes()).await;
        report.run("ttl_expired_sandbox", self.test_ttl_expired_sandbox()).await;

        report
    }

    async fn test_health_check(&self) -> Result<(), String> {
        let backend = (self.make_backend)().await;
        backend.health_check().await.map_err(|e| e.to_string())
    }

    async fn test_exec_echo(&self) -> Result<(), String> {
        let backend = (self.make_backend)().await;
        let ir = SandboxIR::default_for_test();
        let handle = backend.create(&ir).await.map_err(|e| e.to_string())?;
        let result = backend.exec(&handle, "echo 'conformance-test-marker'", None)
            .await.map_err(|e| e.to_string())?;
        backend.destroy(&handle).await.map_err(|e| e.to_string())?;

        if !result.stdout.contains("conformance-test-marker") {
            return Err(format!(
                "stdout non contiene marker atteso. Got: {:?}",
                result.stdout
            ));
        }
        if result.exit_code != 0 {
            return Err(format!("exit_code atteso 0, got {}", result.exit_code));
        }
        Ok(())
    }

    async fn test_destroy_idempotent(&self) -> Result<(), String> {
        let backend = (self.make_backend)().await;
        let ir = SandboxIR::default_for_test();
        let handle = backend.create(&ir).await.map_err(|e| e.to_string())?;
        backend.destroy(&handle).await.map_err(|e| e.to_string())?;
        // Seconda destroy deve essere Ok o NotFound, mai altri errori
        match backend.destroy(&handle).await {
            Ok(_) => Ok(()),
            Err(BackendError::NotFound(_)) => Ok(()),
            Err(e) => Err(format!("seconda destroy ha ritornato errore inatteso: {}", e)),
        }
    }

    async fn test_concurrent_sandboxes(&self) -> Result<(), String> {
        let backend = std::sync::Arc::new((self.make_backend)().await);
        let ir = SandboxIR::default_for_test();

        // Crea 3 sandbox in parallelo
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let b = backend.clone();
                let i = ir.clone();
                tokio::spawn(async move { b.create(&i).await })
            })
            .collect();

        let mut created = vec![];
        for h in handles {
            match h.await.unwrap() {
                Ok(handle) => created.push(handle),
                Err(e) => return Err(format!("creazione concorrente fallita: {}", e)),
            }
        }

        // Distruggi tutte
        for handle in created {
            backend.destroy(&handle).await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    // ... altri test
}

/// Macro per espandere la suite in test tokio normali.
#[macro_export]
macro_rules! run_conformance_suite {
    ($make_backend:expr) => {
        #[tokio::test]
        async fn conformance_health_check() {
            agentsandbox_conformance::ConformanceSuite::new($make_backend)
                .test_health_check().await.expect("health_check");
        }

        #[tokio::test]
        async fn conformance_exec_echo() {
            agentsandbox_conformance::ConformanceSuite::new($make_backend)
                .test_exec_echo().await.expect("exec_echo");
        }

        // ... un #[tokio::test] per ogni test della suite
    };
}
```

### B.9 — Criteri di completamento Fase B

- [ ] `crates/agentsandbox-sdk` esiste come crate separato con trait pubblico
- [ ] `crates/agentsandbox-conformance` esiste con tutti i test implementati
- [ ] Il Docker adapter è stato refactored per implementare il nuovo trait
- [ ] `BackendRegistry` funziona con Docker registrato via `register()`
- [ ] Un backend fittizio (`agentsandbox-backend-noop`) esiste come template per contributor
- [ ] `BACKEND_GUIDE.md` nel repo spiega come creare un backend esterno in < 1 pagina
- [ ] La conformance suite passa per il Docker backend
- [ ] Il daemon si avvia correttamente con `backends.enabled = ["docker"]` in config

---

## FASE C — Egress filtering reale (proxy L4)
**Stima:** 3-4 giorni
**Prerequisito:** Fase B completata.

### C.1 — Architettura del proxy

In v1alpha1 l'egress era un warning. In v1stable è applicato realmente.
L'approccio scelto è un **proxy SOCKS5 interno per container** — non iptables (richiede privilegi), non DNS-only (bypassabile).

```
[container]
    │
    │  SOCKS5 (porta 1080 sull'host)
    ▼
[agentsandbox-proxy]     ← nuovo binary leggero
    │
    ├── allow: pypi.org → risolve, invia
    └── deny: tutto il resto → connessione rifiutata con log
```

Il proxy è un binary Rust separato avviato dal daemon, uno per sandbox con network isolation.

### C.2 — Proxy implementation

```rust
// crates/agentsandbox-proxy/src/lib.rs

use tokio::net::{TcpListener, TcpStream};
use std::collections::HashSet;

pub struct EgressProxy {
    allowed_hostnames: HashSet<String>,
    allowed_ips: HashSet<std::net::IpAddr>,
    bind_addr: String,
}

impl EgressProxy {
    pub fn new(allow: &[String], bind_port: u16) -> Self {
        // Pre-risolvi hostname → IP (una volta sola a startup)
        let mut allowed_ips = HashSet::new();
        for host in allow {
            if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(
                &format!("{}:443", host)
            ) {
                for addr in addrs {
                    allowed_ips.insert(addr.ip());
                }
            }
        }

        Self {
            allowed_hostnames: allow.iter().cloned().collect(),
            allowed_ips,
            bind_addr: format!("127.0.0.1:{}", bind_port),
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!("proxy egress in ascolto su {}", self.bind_addr);

        loop {
            let (stream, peer) = listener.accept().await?;
            let proxy = self.clone(); // Arc in produzione
            tokio::spawn(async move {
                if let Err(e) = proxy.handle_socks5(stream, peer).await {
                    tracing::warn!("proxy error da {}: {}", peer, e);
                }
            });
        }
    }

    async fn handle_socks5(
        &self,
        mut stream: TcpStream,
        _peer: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        // Implementa SOCKS5 RFC 1928
        // 1. Handshake auth (no auth)
        // 2. Request (CONNECT hostname:port)
        // 3. Check allowlist
        // 4. Se allowed: apri connessione upstream e relay
        // 5. Se denied: rispondi con SOCKS5 error code e logga
        todo!("implementazione SOCKS5")
    }

    fn is_allowed(&self, hostname: &str, ip: &std::net::IpAddr) -> bool {
        self.allowed_hostnames.contains(hostname)
            || self.allowed_ips.contains(ip)
    }
}
```

### C.3 — Integrazione nel Docker adapter

```rust
// Il Docker adapter, quando egress.mode == Proxy:
// 1. Chiede al daemon di allocare una porta proxy libera
// 2. Avvia agentsandbox-proxy su quella porta
// 3. Configura il container con HTTP_PROXY / HTTPS_PROXY
//    che puntano al proxy sull'host

impl DockerBackend {
    async fn setup_egress(&self, ir: &SandboxIR) -> Result<Option<ProxyHandle>, BackendError> {
        match ir.egress.mode {
            EgressMode::None => Ok(None),
            EgressMode::Passthrough => {
                tracing::warn!(
                    sandbox_id = %ir.id,
                    "egress mode=passthrough: nessun filtro applicato"
                );
                Ok(None)
            }
            EgressMode::Proxy => {
                let port = allocate_port().await?;
                let proxy = EgressProxy::new(&ir.egress.allow_hostnames, port);
                let handle = tokio::spawn(proxy.run());
                Ok(Some(ProxyHandle { port, task: handle }))
            }
        }
    }
}
```

### C.4 — Limiti documentati (obbligatori in docs/)

```markdown
## Limiti noti di network.egress in v1stable

**Risolti rispetto a v1alpha1:**
- L'egress è ora applicato realmente via proxy SOCKS5 (non più solo un warning)
- Il deny-by-default è effettivo

**Limiti rimanenti documentati:**
- La risoluzione DNS avviene una volta sola a startup della sandbox.
  CDN con IP rotation (es. alcune configurazioni Cloudflare) possono
  non funzionare correttamente.
- Il proxy SOCKS5 non supporta UDP in v1stable.
- HTTPS inspection non è implementata: il proxy vede il hostname
  dall'handshake TLS SNI, non il path. Non può filtrare per path.
- Un processo nel container che non rispetta le variabili HTTP_PROXY
  può bypassare il filtro. Usa isolation_level: MicroVM per
  isolamento garantito a livello di rete.

Questi limiti saranno indirizzati in v2 con nftables nel namespace
di rete del container (richiede Docker con NET_ADMIN).
```

---

## FASE D — Firecracker backend (come primo backend esterno ufficiale)
**Stima:** 5-7 giorni | **Solo su Linux con KVM**

Firecracker dimostra che l'architettura plugin funziona con un backend reale e complesso.
Il crate vive in `crates/agentsandbox-firecracker/` — strutturalmente identico a un backend esterno.

### D.1 — Prerequisiti di sistema documentati

```markdown
## Prerequisiti Firecracker backend

**Richiesti:**
- Linux kernel >= 5.10
- KVM disponibile: `ls /dev/kvm` deve esistere
- Firecracker binary: `wget https://github.com/firecracker-microvm/firecracker/releases/...`
- jailer binary (stesso release)
- Kernel guest: vmlinux compilato con config minimale
- Rootfs per ogni preset: immagini ext4 minimali

**Non supportato:**
- macOS (KVM non disponibile)
- VPS senza nested virtualization (EC2 metal OK, t3 NO)
- WSL2 (senza configurazione speciale)

Il backend Firecracker fallisce l'health_check su questi sistemi
e ritorna BackendError::Unavailable con messaggio esplicito.
Il daemon continua con gli altri backend disponibili.
```

### D.2 — Struttura del crate

```rust
// crates/agentsandbox-firecracker/src/lib.rs

pub struct FirecrackerBackendFactory;

impl BackendFactory for FirecrackerBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "firecracker",
            display_name: "Firecracker MicroVM",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,    // vera isolation a livello VM
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: false,      // richiede KVM e binaries
                isolation_level: IsolationLevel::MicroVM,
                supported_presets: vec!["python", "node", "shell"],
                extra: HashMap::new(),
            },
        }
    }

    fn create(&self, config: &HashMap<String, String>) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let binary = config.get("binary_path")
            .ok_or_else(|| BackendError::Configuration("binary_path richiesto".into()))?;

        Ok(Box::new(FirecrackerBackend {
            binary_path: binary.clone(),
            kernel_image: config.get("kernel_image").cloned()
                .ok_or_else(|| BackendError::Configuration("kernel_image richiesto".into()))?,
            rootfs_dir: config.get("rootfs_dir").cloned()
                .ok_or_else(|| BackendError::Configuration("rootfs_dir richiesto".into()))?,
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

// Ogni VM ha una socket UNIX in /tmp/agentsandbox-fc-{id}.sock
// Il backend comunica con la VM via API REST su questa socket.
// Il guest expone un agent minimale che accetta comandi exec via vsock.
```

### D.3 — Criteri di completamento Fase D

- [ ] `health_check()` ritorna Unavailable con messaggio chiaro su macOS o senza KVM
- [ ] La conformance suite passa su Linux con KVM
- [ ] Il backend Firecracker è selezionato automaticamente quando `isolation_level: MicroVM` è richiesto
- [ ] Documentazione prerequisiti di sistema in `docs/backends/firecracker.md`

---

## FASE E — Multi-tenancy e autenticazione
**Stima:** 3 giorni

### E.1 — Modello di autenticazione

```rust
// Due modalità:
// 1. Single-user locale (default) — nessuna auth, 127.0.0.1 only
// 2. Multi-tenant — API key per tenant, binding su 0.0.0.0

// agentsandbox.toml
[auth]
mode = "single_user"   # "single_user" | "api_key"

# Per api_key mode:
[[auth.tenants]]
id = "tenant-abc"
api_key = "ask_..."    # hash bcrypt in DB, mai plaintext
quota_sandboxes_per_hour = 100
quota_concurrent = 10
```

### E.2 — Schema SQLite aggiornato

```sql
-- migrations/002_multitenancy.sql

CREATE TABLE tenants (
    id TEXT PRIMARY KEY,
    api_key_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    quota_hourly INTEGER NOT NULL DEFAULT 100,
    quota_concurrent INTEGER NOT NULL DEFAULT 10,
    enabled INTEGER NOT NULL DEFAULT 1
);

-- Aggiungi tenant_id alle sandbox esistenti
ALTER TABLE sandboxes ADD COLUMN tenant_id TEXT;

-- Rate limiting
CREATE TABLE rate_limit_windows (
    tenant_id TEXT NOT NULL,
    window_start TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, window_start)
);
```

### E.3 — Middleware di autenticazione

```rust
// crates/agentsandbox-daemon/src/middleware/auth.rs

use axum::{extract::State, http::Request, middleware::Next, response::Response};

pub async fn auth_middleware<B>(
    State(state): State<Arc<AppState>>,
    mut request: Request<B>,
    next: Next<B>,
) -> Result<Response, ApiError> {
    match &state.config.auth.mode {
        AuthMode::SingleUser => {
            // Verifica che la richiesta venga da localhost
            // In single_user mode non serve API key
            Ok(next.run(request).await)
        }
        AuthMode::ApiKey => {
            let key = request
                .headers()
                .get("X-API-Key")
                .and_then(|v| v.to_str().ok())
                .ok_or(ApiError::Unauthorized("X-API-Key richiesta".into()))?;

            let tenant = state.db.verify_api_key(key).await
                .map_err(|_| ApiError::Unauthorized("API key non valida".into()))?;

            request.extensions_mut().insert(tenant);
            Ok(next.run(request).await)
        }
    }
}
```

---

## FASE F — Osservabilità production-grade
**Stima:** 2 giorni

### F.1 — Metriche (Prometheus-compatible)

```rust
// crates/agentsandbox-daemon/src/metrics.rs
// Espone GET /metrics in formato Prometheus text

pub struct DaemonMetrics {
    pub sandboxes_created_total: Counter,
    pub sandboxes_active: Gauge,
    pub sandbox_exec_duration_ms: Histogram,
    pub backend_errors_total: CounterVec,     // by backend, by error type
    pub egress_connections_allowed: Counter,
    pub egress_connections_denied: Counter,
}

// Endpoint:
// GET /metrics → formato Prometheus
// GET /v1/metrics/json → formato JSON (per chi non ha Prometheus)
```

### F.2 — Audit log strutturato

```rust
// Ogni evento di audit è una struttura serializzabile, non una stringa libera.

#[derive(Serialize)]
pub struct AuditEvent {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub sandbox_id: String,
    pub tenant_id: Option<String>,
    pub event: AuditEventKind,
    pub backend: String,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i64>,
    // NEVER include: secret_env, api_key, raw command se contiene dati sensibili
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEventKind {
    SandboxCreated { spec_version: String },
    ExecStarted { command_hash: String },   // hash del comando, non il testo
    ExecCompleted { exit_code: i64 },
    SandboxDestroyed { reason: DestroyReason },
    EgressDenied { hostname: String },
    BackendError { error: String },
}

#[derive(Serialize)]
pub enum DestroyReason {
    ClientRequest,
    TtlExpired,
    HealthCheckFailed,
    BackendError,
}
```

### F.3 — Structured logging

```rust
// Tutti i log usano tracing con campi strutturati, mai stringhe interpolate.

// SBAGLIATO:
tracing::info!("sandbox {} creata in {}ms", id, duration);

// GIUSTO:
tracing::info!(
    sandbox_id = %id,
    backend = %backend_name,
    duration_ms = duration,
    "sandbox creata"
);

// Questo permette a qualsiasi aggregatore (Loki, CloudWatch, Datadog)
// di filtrare per sandbox_id senza parsing regex.
```

---

## FASE G — SDK v1.0 con breaking change policy
**Stima:** 2 giorni

### G.1 — Semantic versioning degli SDK

```
SDK 0.x.y → v1alpha1 del daemon (breaking changes permesse)
SDK 1.0.0 → v1beta1 e v1stable del daemon (breaking changes → major bump)
```

### G.2 — Nuove feature SDK per v1stable

**Python:**
```python
# Nuovo in v1.0

# 1. Upload/download file (se backend supporta)
async with Sandbox(runtime="python") as sb:
    await sb.upload("/workspace/data.csv", open("local.csv", "rb").read())
    result = await sb.exec("python process.py")
    output = await sb.download("/workspace/output.json")

# 2. Streaming output (exec long-running)
async with Sandbox(runtime="python") as sb:
    async for chunk in sb.exec_stream("python train.py"):
        print(chunk.stdout, end="", flush=True)

# 3. Backend info
async with Sandbox(runtime="python") as sb:
    info = await sb.info()
    print(info.backend)          # "docker" o "firecracker" o "nix"
    print(info.isolation_level)  # "Container" o "MicroVM"

# 4. Explicit backend request (con fallback documentato)
async with Sandbox(runtime="python", backend="firecracker") as sb:
    # Se firecracker non è disponibile → SandboxError("backend 'firecracker' non disponibile")
    # Mai fallback silenzioso a docker
    pass
```

**TypeScript:**
```typescript
// Stesso pattern, API speculare all'SDK Python
const sb = await Sandbox.create({
    runtime: "node",
    backend: "docker",
});

// Streaming
for await (const chunk of sb.execStream("node server.js")) {
    process.stdout.write(chunk.stdout);
}

await sb.destroy();
```

### G.3 — Deprecation policy

```markdown
## Policy breaking changes SDK

1. Una feature deprecata rimane funzionante per almeno 2 minor version.
2. Le deprecazioni sono annotate con `@deprecated` in Python e TSDoc in TypeScript.
3. Il CHANGELOG documenta ogni deprecazione con la versione di rimozione pianificata.
4. Il daemon supporta sempre le ultime 2 versioni spec (es. v1alpha1 + v1beta1).
   v1alpha1 non viene rimossa finché non esiste v1.

Esempio:
    SDK 1.2.0: depreca `Sandbox(runtime="python")` in favore di `Sandbox(preset="python")`
    SDK 1.4.0: rimuove `Sandbox(runtime=...)` → major bump a SDK 2.0.0
```

---

## FASE H — Release checklist e governance
**Stima:** 1 giorno

### H.1 — Checklist pre-release v1.0.0

**Correttezza:**
- [ ] Nessun `unwrap()` in codice non-test (grep verificato in CI)
- [ ] Nessun `todo!()` in codice non-test
- [ ] Tutti i test della conformance suite passano per Docker backend
- [ ] Test e2e completi su Linux e macOS (Docker backend)
- [ ] Fuzzing del compile pipeline con `cargo-fuzz` (almeno 1 ora)

**Sicurezza:**
- [ ] `cargo audit` senza vulnerabilità note
- [ ] I secret non appaiono in nessun log (test automatico)
- [ ] L'audit log non contiene comandi in plaintext
- [ ] Il native handle del backend non è mai esposto nell'API pubblica

**Compatibilità:**
- [ ] Spec v1alpha1 continua a funzionare senza modifiche
- [ ] SDK Python 3.10, 3.11, 3.12 testati
- [ ] SDK TypeScript con Node 18, 20, 22 testati

**Documentazione:**
- [ ] `BACKEND_GUIDE.md` completo e revisionato
- [ ] `docs/api-http-v1.md` con curl examples per ogni endpoint
- [ ] `docs/backends/docker.md` e `docs/backends/firecracker.md`
- [ ] `CHANGELOG.md` aggiornato
- [ ] `CONTRIBUTING.md` con processo PR per nuovi backend

**Release:**
- [ ] Tag `v1.0.0` sul commit
- [ ] Binary precompilati per linux/amd64, linux/arm64, darwin/arm64
- [ ] `agentsandbox-sdk` su PyPI con versione 1.0.0
- [ ] `agentsandbox` su npm con versione 1.0.0
- [ ] `agentsandbox-sdk` crate su crates.io (per autori di backend esterni)
- [ ] `agentsandbox-conformance` crate su crates.io

### H.2 — Struttura finale del repository

```
agentsandbox/
├── crates/
│   ├── agentsandbox-sdk/           ← NUOVO: trait pubblici per backend author
│   ├── agentsandbox-conformance/   ← NUOVO: suite di test comune
│   ├── agentsandbox-core/          ← spec parser, IR, compile pipeline
│   ├── agentsandbox-daemon/        ← binary daemon, API HTTP, registry
│   ├── agentsandbox-proxy/         ← NUOVO: proxy egress SOCKS5
│   ├── agentsandbox-docker/        ← Docker backend (built-in)
│   └── agentsandbox-firecracker/   ← Firecracker backend (built-in, optional)
├── sdks/
│   ├── python/
│   └── typescript/
├── spec/
│   ├── sandbox.v1alpha1.schema.json
│   └── sandbox.v1beta1.schema.json
├── examples/
│   ├── python-code-review-agent/
│   └── ts-dependency-auditor/
├── docs/
│   ├── spec-v1alpha1.md
│   ├── spec-v1beta1.md
│   ├── api-http-v1.md
│   ├── BACKEND_GUIDE.md            ← guida per autori di backend
│   ├── getting-started.md
│   └── backends/
│       ├── docker.md
│       └── firecracker.md
├── .github/
│   └── workflows/
│       ├── ci.yml                  ← test + conformance suite
│       ├── release.yml             ← build binary + publish SDK
│       └── conformance-matrix.yml  ← test tutti i backend built-in
├── Cargo.toml
├── CHANGELOG.md
├── CONTRIBUTING.md
└── BACKEND_GUIDE.md                ← symlink a docs/BACKEND_GUIDE.md
```

### H.3 — CI pipeline per backend di terze parti

```yaml
# .github/workflows/conformance-matrix.yml
# Questo workflow testa tutti i backend built-in ad ogni push.
# Un contributor esterno può copiarlo nel proprio repo.

name: Conformance Matrix
on: [push, pull_request]

jobs:
  conformance:
    strategy:
      matrix:
        backend: [docker]
        # firecracker: solo su runner self-hosted con KVM
        os: [ubuntu-22.04, macos-14]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Start daemon
        run: cargo run -p agentsandbox-daemon &
      - name: Run conformance suite
        run: cargo test -p agentsandbox-${{ matrix.backend }} conformance
      - name: Run SDK tests
        run: |
          cd sdks/python && pip install -e ".[dev]"
          pytest tests/ -m integration
```

---

---

## FASE I — Escape hatch: accesso controllato alle opzioni native del backend
**Stima:** 1-2 giorni | **Prerequisito:** Fase B completata.

### Il problema

Il trait `SandboxBackend` copre il 90% dei casi d'uso. Ma Docker ha centinaia di opzioni
(`--cap-add`, `--security-opt`, `--device`, `--ulimit`, volume mounts avanzati, cgroup v2 settings)
e Firecracker ha la propria superficie (balloon device, vsock config, MMDS metadata, CPU templates).

Non è possibile né sensato wrappare tutto. Ma non dare nessuna via d'uscita significa che
un utente con un caso d'uso legittimamente avanzato deve forkare il backend o rinunciare al progetto.

La soluzione è un **escape hatch esplicito, opaco al core, versionato per backend**.

---

### I.1 — Principio di design

Quattro regole non negoziabili:

1. **Opaco al core.** Il compile pipeline non interpreta mai il contenuto dell'escape hatch.
   Lo passa attraverso come blob. Solo il backend lo legge.

2. **Esplicito nella spec.** L'utente deve scrivere `extensions:` consapevolmente.
   Non può attivare comportamenti nativi per errore.

3. **Dichiarato dal backend.** Ogni backend pubblica uno schema JSON delle estensioni
   che accetta. Estensioni sconosciute producono un errore, mai silenzio.

4. **Mai nel contratto pubblico dell'API.** Il `sandbox_id` e il `lease_token` non cambiano.
   Le estensioni non trapelano nella response. L'astrazione esterna rimane intatta.

---

### I.2 — Spec con escape hatch

```yaml
# Uso normale — nessuna estensione, funziona su qualsiasi backend
apiVersion: sandbox.ai/v1beta1
kind: Sandbox
metadata:
  name: my-sandbox
spec:
  runtime:
    preset: python
  ttlSeconds: 300

---

# Uso avanzato con escape hatch — richiede backend specifico
apiVersion: sandbox.ai/v1beta1
kind: Sandbox
metadata:
  name: my-sandbox
spec:
  runtime:
    preset: python
  scheduling:
    backend: docker          # OBBLIGATORIO con extensions — vedi sotto perché
  ttlSeconds: 300

  # Il campo extensions è opaco al core.
  # Il contenuto è specifico del backend dichiarato in scheduling.backend.
  # Se scheduling.backend è assente, extensions produce un errore di compilazione.
  extensions:
    docker:
      hostConfig:
        capAdd: ["NET_ADMIN"]
        ulimits:
          - name: nofile
            soft: 65536
            hard: 65536
        devices:
          - pathOnHost: /dev/nvidia0
            pathInContainer: /dev/nvidia0
            cgroupPermissions: rwm
        binds:
          - "/data/models:/models:ro"
      # Qualsiasi chiave non riconosciuta da Docker produce un errore esplicito,
      # non un comportamento silenzioso.

---

# Stesso pattern per Firecracker
spec:
  scheduling:
    backend: firecracker
  extensions:
    firecracker:
      machine_config:
        cpu_template: "C3"
        track_dirty_pages: true
      balloon:
        amount_mib: 256
        deflate_on_oom: true
      mmds:
        version: "V2"
        ipv4_address: "169.254.169.254"
```

**Perché `scheduling.backend` è obbligatorio con `extensions`:**
Le estensioni sono per definizione backend-specifiche. Permettere extensions senza dichiarare
il backend esplicito renderebbe impossibile la validazione e creerebbe comportamenti ambigui
su sistemi con backend multipli disponibili. Il compile pipeline lo rifiuta con errore chiaro:

```
CompileError::ExtensionsRequireExplicitBackend(
  "il campo extensions richiede scheduling.backend esplicito"
)
```

---

### I.3 — IR con escape hatch

```rust
// crates/agentsandbox-core/src/ir.rs

#[derive(Debug, Clone)]
pub struct SandboxIR {
    // ... tutti i campi esistenti invariati ...

    /// Escape hatch: contenuto opaco passato al backend selezionato.
    /// Il core non lo interpreta mai. Il backend lo valida contro
    /// il proprio schema prima di usarlo.
    /// None se l'utente non ha specificato extensions.
    pub extensions: Option<serde_json::Value>,

    /// Backend esplicitamente richiesto dall'utente.
    /// Se Some, il registry non fa selezione automatica.
    /// Se None, il selector sceglie il backend più appropriato.
    pub backend_hint: Option<String>,
}
```

Il compile pipeline popola `extensions` copiando il valore raw dalla spec
**senza validarlo**. La validazione è responsabilità del backend.

---

### I.4 — Il backend riceve e valida le estensioni

```rust
// crates/agentsandbox-sdk/src/backend.rs

// Il trait SandboxBackend riceve le estensioni nell'IR.
// Il metodo can_satisfy() è il punto dove validarle PRIMA della creazione.

#[async_trait]
pub trait SandboxBackend: Send + Sync {

    /// Verifica che il backend possa soddisfare la IR — incluse le extensions.
    /// Questo è il punto dove fare schema validation delle extensions.
    /// Errore qui → risposta 422 al client prima di creare qualsiasi risorsa.
    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        // Default: se ci sono extensions e il backend non le supporta → errore.
        if ir.extensions.is_some() {
            return Err(BackendError::NotSupported(
                "questo backend non supporta extensions".into()
            ));
        }
        Ok(())
    }

    // ... resto del trait invariato
}
```

**Implementazione nel Docker backend:**

```rust
// crates/agentsandbox-docker/src/backend.rs

use serde::Deserialize;

/// Schema delle extensions Docker. Validato con serde prima dell'uso.
/// Ogni campo è opzionale — l'utente specifica solo ciò che serve.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
// deny_unknown_fields è fondamentale: campi non riconosciuti → errore,
// non silenzio. L'utente sa subito se ha sbagliato un nome.
pub struct DockerExtensions {
    pub host_config: Option<DockerHostConfigExt>,
    pub labels: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DockerHostConfigExt {
    pub cap_add: Option<Vec<String>>,
    pub cap_drop: Option<Vec<String>>,
    pub security_opt: Option<Vec<String>>,
    pub ulimits: Option<Vec<DockerUlimit>>,
    pub devices: Option<Vec<DockerDevice>>,
    pub binds: Option<Vec<String>>,
    pub shm_size_mb: Option<u64>,
    pub sysctls: Option<std::collections::HashMap<String, String>>,
    pub privileged: Option<bool>,   // loggato con WARNING nell'audit log
}

#[derive(Debug, Deserialize)]
pub struct DockerUlimit {
    pub name: String,
    pub soft: u64,
    pub hard: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerDevice {
    pub path_on_host: String,
    pub path_in_container: String,
    pub cgroup_permissions: String,
}

impl DockerBackend {
    fn parse_extensions(ir: &SandboxIR) -> Result<DockerExtensions, BackendError> {
        match &ir.extensions {
            None => Ok(DockerExtensions::default()),
            Some(raw) => {
                // Estrai solo la sezione "docker" dalle extensions
                let docker_section = raw.get("docker")
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default()));

                serde_json::from_value::<DockerExtensions>(docker_section)
                    .map_err(|e| BackendError::Configuration(
                        format!("extensions.docker non valide: {}", e)
                    ))
            }
        }
    }

    fn apply_extensions(
        ext: &DockerExtensions,
        host_config: &mut bollard::models::HostConfig,
    ) {
        if let Some(hc) = &ext.host_config {
            if let Some(caps) = &hc.cap_add {
                host_config.cap_add = Some(caps.clone());
            }
            if let Some(opts) = &hc.security_opt {
                host_config.security_opt = Some(opts.clone());
            }
            if hc.privileged == Some(true) {
                // Audit warning: privileged mode è un rischio di sicurezza
                tracing::warn!(
                    "extensions.docker.hostConfig.privileged=true: \
                     la sandbox avrà accesso privilegiato all'host"
                );
                host_config.privileged = Some(true);
            }
            // ... altri campi
        }
    }
}

// can_satisfy() valida prima della creazione
#[async_trait]
impl SandboxBackend for DockerBackend {
    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        // Prova a parsare le extensions: se il parsing fallisce,
        // l'errore torna al client PRIMA di creare il container.
        Self::parse_extensions(ir)?;
        Ok(())
    }

    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let ext = Self::parse_extensions(ir)?; // Safe: già validato in can_satisfy

        let mut host_config = HostConfig {
            memory: Some((ir.memory_mb as i64) * 1024 * 1024),
            nano_cpus: Some((ir.cpu_millicores as i64) * 1_000_000),
            ..Default::default()
        };

        // Applica le estensioni DOPO i valori base.
        // Le estensioni non possono abbassare i limiti di memoria/CPU
        // sotto i valori richiesti dalla spec — solo aggiungere opzioni.
        Self::apply_extensions(&ext, &mut host_config);

        // ... resto della creazione invariato
        todo!()
    }

    // ... resto del trait
}
```

---

### I.5 — Cosa succede nell'audit log

Le estensioni vengono registrate nell'audit log in forma ridotta, non verbatim.
Questo evita di loggare mount paths, device paths o security options sensibili in chiaro.

```rust
// In audit_log, quando extensions è Some:
AuditEventKind::SandboxCreated {
    spec_version: "sandbox.ai/v1beta1".into(),
    extensions_present: true,
    extensions_backend: Some("docker".into()),
    // Il contenuto delle extensions NON viene loggato.
    // Solo la presenza e il backend target.
}
```

---

### I.6 — Come il backend pubblica il proprio schema di estensioni

Ogni backend che supporta extensions pubblica il proprio JSON Schema
come endpoint della sua documentazione e come file nel proprio crate.

```rust
// Aggiungi al BackendDescriptor:
pub struct BackendDescriptor {
    // ... campi esistenti ...

    /// JSON Schema delle extensions accettate da questo backend.
    /// None se il backend non supporta extensions.
    /// Usato dal daemon per esporre GET /v1/backends/{id}/extensions-schema
    pub extensions_schema: Option<&'static str>, // JSON Schema embedded
}

// Nel Docker backend factory:
fn describe(&self) -> BackendDescriptor {
    BackendDescriptor {
        id: "docker",
        extensions_schema: Some(include_str!("../schema/docker-extensions.schema.json")),
        // ...
    }
}
```

**Endpoint nuovo nel daemon:**

```
GET /v1/backends
→ lista backend disponibili con capabilities

GET /v1/backends/docker/extensions-schema
→ JSON Schema delle extensions Docker (per IDE autocomplete, validazione client-side)

GET /v1/backends/firecracker/extensions-schema
→ JSON Schema delle extensions Firecracker
```

Questo permette agli SDK di scaricare lo schema e validare le extensions
**prima** di fare la richiesta, con messaggi d'errore contestuali.

---

### I.7 — SDK Python con escape hatch

```python
from agentsandbox import Sandbox

# Modo 1: dizionario Python diretto
async with Sandbox(
    runtime="python",
    backend="docker",
    extensions={
        "docker": {
            "hostConfig": {
                "capAdd": ["NET_ADMIN"],
                "ulimits": [{"name": "nofile", "soft": 65536, "hard": 65536}],
            }
        }
    }
) as sb:
    result = await sb.exec("python script.py")

# Modo 2: helper tipizzato per Docker (opzionale, import separato)
from agentsandbox.extensions.docker import DockerExtensions, HostConfig, Ulimit

async with Sandbox(
    runtime="python",
    backend="docker",
    extensions=DockerExtensions(
        host_config=HostConfig(
            cap_add=["NET_ADMIN"],
            ulimits=[Ulimit(name="nofile", soft=65536, hard=65536)],
        )
    ).to_dict()
) as sb:
    result = await sb.exec("python script.py")
```

Gli helper tipizzati (`agentsandbox.extensions.docker`, `.firecracker`, ecc.)
sono **opzionali e separati** dall'SDK base. L'SDK base non sa nulla di Docker o Firecracker —
riceve solo un dizionario e lo passa al daemon. I tipi sono solo ergonomia per l'utente avanzato.

---

### I.8 — Cosa NON è permesso nelle extensions (hardcoded nel core)

Alcune opzioni native non possono mai passare attraverso l'escape hatch,
indipendentemente da cosa il backend supporti. Il compile pipeline le rifiuta
con `CompileError::ExtensionForbidden` prima ancora di chiamare il backend.

```rust
// crates/agentsandbox-core/src/compile.rs

fn validate_extensions_safety(
    extensions: &serde_json::Value,
    backend: &str,
) -> Result<(), CompileError> {
    match backend {
        "docker" => {
            // network_mode non può essere overriddato via extensions:
            // il networking è gestito dal core (proxy egress).
            if extensions.pointer("/docker/hostConfig/networkMode").is_some() {
                return Err(CompileError::ExtensionForbidden(
                    "extensions.docker.hostConfig.networkMode non è permesso. \
                     Usa spec.network.egress per configurare il networking."
                        .into(),
                ));
            }
            // Il container name non può essere overriddato:
            // il core lo gestisce per il lifecycle tracking.
            if extensions.pointer("/docker/name").is_some() {
                return Err(CompileError::ExtensionForbidden(
                    "extensions.docker.name non è permesso.".into()
                ));
            }
        }
        "firecracker" => {
            // Il vsock non può essere configurato via extensions:
            // è usato internamente per la comunicazione exec.
            if extensions.pointer("/firecracker/vsock").is_some() {
                return Err(CompileError::ExtensionForbidden(
                    "extensions.firecracker.vsock è riservato al sistema interno.".into()
                ));
            }
        }
        _ => {}
    }
    Ok(())
}
```

---

### I.9 — Documentazione obbligatoria per ogni backend che supporta extensions

Il file `docs/backends/docker.md` deve includere:

```markdown
## Extensions Docker

Le extensions permettono di accedere a opzioni Docker native non coperte dalla spec.

⚠️ **Quando usare extensions:**
Le extensions sono per casi d'uso avanzati. Se hai bisogno di extensions
per un caso d'uso comune (es. limiti memoria, timeout), apri prima una issue:
potrebbe essere un'opzione da aggiungere alla spec standard.

⚠️ **Implicazioni:**
- Una spec con extensions è portabile solo su backend Docker.
  `scheduling.backend: docker` è obbligatorio e vincolante.
- Non esiste garanzia di compatibilità futura per le opzioni native Docker.
  Un upgrade di Docker potrebbe cambiare il comportamento delle opzioni usate.
- Le extensions non sono validate dal core — errori nelle opzioni native
  producono errori Docker, non errori AgentSandbox.

### Schema completo extensions Docker

[link al JSON Schema scaricabile]

### Esempio: GPU access

[esempio completo]

### Opzioni non permesse

Le seguenti opzioni Docker non sono accessibili via extensions
perché interferiscono con il funzionamento interno di AgentSandbox:
- `hostConfig.networkMode` → usa `spec.network.egress`
- `name` → gestito internamente
- `hostConfig.autoRemove` → gestito dal lifecycle manager
```

---

### I.10 — Criteri di completamento Fase I

- [ ] `compile_any()` accetta `extensions` nella spec e popola `ir.extensions`
- [ ] `extensions` senza `scheduling.backend` esplicito produce `CompileError::ExtensionsRequireExplicitBackend`
- [ ] Le opzioni proibite producono `CompileError::ExtensionForbidden` con messaggio chiaro
- [ ] Docker backend: `can_satisfy()` valida le extensions con `deny_unknown_fields`
- [ ] Docker backend: `create()` applica le extensions dopo i valori base della spec
- [ ] `privileged: true` produce un warning nell'audit log (non un errore)
- [ ] `GET /v1/backends` lista i backend con flag `supports_extensions: bool`
- [ ] `GET /v1/backends/docker/extensions-schema` ritorna il JSON Schema
- [ ] SDK Python: `extensions={}` viene passato al daemon senza modifiche
- [ ] `docs/backends/docker.md` include sezione Extensions con schema e warning
- [ ] Test: extensions con campo sconosciuto → 422 con messaggio che nomina il campo

---

## Note operative per Claude Code (v1stable)

1. **Inizia dalla Fase B prima della C e D.** Il plugin architecture è il prerequisito di tutto il resto. Senza un trait stabile, Firecracker e il proxy non hanno dove appoggiarsi.

2. **`agentsandbox-sdk` deve essere un crate separato da `agentsandbox-core`.** Il core è implementazione, l'SDK è contratto pubblico. Un contributor esterno deve poter dipendere solo da `agentsandbox-sdk` senza transitivamente portarsi dietro Bollard, SQLx e Axum.

3. **Il crate `agentsandbox-conformance` deve compilare senza nessun backend specifico come dipendenza.** Usa solo `agentsandbox-sdk`. Un backend author esegue la conformance suite contro la propria implementazione senza dover installare Docker o Firecracker.

4. **Ogni backend built-in (Docker, Firecracker) è architetturalmente indistinguibile da un backend esterno.** Se per farlo funzionare devi aggiungere un campo in `agentsandbox-core`, l'architettura è sbagliata — il campo va nell'SDK pubblico o nel backend stesso.

5. **Il `BackendSelector` non deve mai contenere nomi di backend hardcoded.** Usa solo le `BackendCapabilities` dichiarate dal descriptor. Se scrivi `if backend_name == "docker"` nel selector, stai rompendo il principio di estensibilità.

6. **La conformance suite non è opzionale per i PR di nuovi backend.** Se un PR aggiunge un backend senza `run_conformance_suite!()`, il CI deve fallire automaticamente.
