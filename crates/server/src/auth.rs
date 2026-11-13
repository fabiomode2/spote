//! Extractor de autenticación: `Authorization: Bearer <token>`.

use axum::{extract::FromRequestParts, http::request::Parts};
use spote_protocol::User;

use crate::{error::AppError, state::AppState};

/// Usuario autenticado, ya resuelto contra la tabla `sessions`.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)
            .ok_or_else(|| AppError::Unauthorized("missing bearer token".to_string()))?;
        let user = state
            .db
            .user_by_token(&token)?
            .ok_or_else(|| AppError::Unauthorized("invalid token".to_string()))?;
        Ok(AuthUser(user))
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let raw = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    let token = token.trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return None;
    }
    Some(token.to_string())
}
