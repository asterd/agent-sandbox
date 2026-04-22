use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::{
    error::ApiError,
    state::{AuthContext, SharedState},
    store,
};

pub async fn auth_middleware(
    State(state): State<SharedState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let auth = match state.config.auth.mode {
        crate::config::AuthMode::SingleUser => AuthContext::single_user(),
        crate::config::AuthMode::ApiKey => {
            let key = request
                .headers()
                .get("X-API-Key")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ApiError::unauthorized("X-API-Key richiesta"))?;

            let mut tenant = store::verify_api_key(&state.db, key)
                .await?
                .ok_or_else(|| ApiError::unauthorized("API key non valida"))?;
            if let Some(policy) = state.config.tenants.get(&tenant.id) {
                tenant.allowed_backends = policy.allowed_backends.clone();
            }
            AuthContext::tenant(tenant)
        }
    };

    request.extensions_mut().insert(auth);
    Ok(next.run(request).await)
}
