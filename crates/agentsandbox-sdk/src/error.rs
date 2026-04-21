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
