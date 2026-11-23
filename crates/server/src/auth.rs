//! Autenticación en dos capas:
//!
//! 1. Clave de cliente obligatoria en **todas** las rutas de `/api/v1`
//!    (`X-Spote-Key`). Sin ella la API no existe para el mundo.
//! 2. Token de sesión `Authorization: Bearer …`, atado a la clave de cliente
//!    que lo creó y con caducidad.
//!
//! La clave de cliente se compara por SHA-256 en tiempo constante y nunca se
//! registra en logs.

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts},
    middleware::Next,
    response::Response,
};
use spote_protocol::User;

use crate::{config::sha256, error::AppError, state::AppState};

pub const CLIENT_KEY_HEADER: &str = "x-spote-key";

/// Huella de la clave de cliente validada por el middleware.
#[derive(Debug, Clone)]
pub struct ClientKeyId(pub String);

/// Hash del token de la sesión actual (nunca el token en claro).
#[derive(Debug, Clone)]
pub struct SessionTokenHash(pub String);

/// Usuario autenticado, ya resuelto contra la tabla `sessions`.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let key = parts
            .extensions
            .get::<ClientKeyId>()
            .cloned()
            .ok_or_else(|| AppError::Unauthorized("client key required".to_string()))?;
        let token = bearer_token(parts)
            .ok_or_else(|| AppError::Unauthorized("missing bearer token".to_string()))?;
        let user = state
            .db
            .user_by_token(&token, &key.0)?
            .ok_or_else(|| AppError::Unauthorized("invalid session".to_string()))?;

        let token_hash = hex::encode(sha256(token.as_bytes()));
        parts.extensions.insert(SessionTokenHash(token_hash));
        Ok(AuthUser(user))
    }
}

impl FromRequestParts<AppState> for SessionTokenHash {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or_else(|| AppError::Unauthorized("missing session".to_string()))
    }
}

impl FromRequestParts<AppState> for ClientKeyId {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .cloned()
            .ok_or_else(|| AppError::Unauthorized("client key required".to_string()))
    }
}

/// Middleware que cierra `/api/v1` a cualquier petición sin una clave de cliente
/// autorizada. Se aplica antes del routing de la API, también a los 404.
pub async fn require_client_key(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let raw = request
        .headers()
        .get(CLIENT_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Unauthorized("client key required".to_string()))?;

    let Some(fingerprint) = state.config.client_key_fingerprint(raw) else {
        tracing::debug!("clave de cliente rechazada");
        return Err(AppError::Unauthorized("client key required".to_string()));
    };

    if let Some(label) = state.config.client_key_label(&fingerprint) {
        tracing::debug!(client = %label, "clave de cliente aceptada");
    }
    request.extensions_mut().insert(ClientKeyId(fingerprint));
    Ok(next.run(request).await)
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    let token = token.trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return None;
    }
    Some(token.to_string())
}
