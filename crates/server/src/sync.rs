//! Sincronización de la biblioteca personal (PC ↔ móvil).
//!
//! Solo metadatos: pistas, favoritas, contadores, historial y playlists. Las
//! rutas locales y el audio no salen del dispositivo. Cada usuario tiene su
//! propio espacio y su propio contador de revisión; no hay forma de leer el de
//! otro cambiando un id.

use spote_protocol::{SyncOp, SyncPullResponse, SyncPushRequest, SyncPushResponse};

use crate::{error::AppError, service, state::AppState};

/// Lote pequeño: cada op puede llevar una carátula de hasta 2 KiB y el cuerpo
/// HTTP está limitado a 1 MiB. Los clientes parten la biblioteca en lotes.
pub const MAX_SYNC_OPS: usize = 250;
pub const MAX_SYNC_ID_CHARS: usize = 64;
pub const MAX_SYNC_NAME_CHARS: usize = 128;
pub const MAX_SYNC_DESCRIPTION_CHARS: usize = 2_000;
pub const MAX_SYNC_POSITION: i64 = 1_000_000;

pub fn snapshot(state: &AppState, user_id: &str) -> Result<SyncPullResponse, AppError> {
    state.db.sync_snapshot(user_id)
}

pub fn pull(state: &AppState, user_id: &str, since: i64) -> Result<SyncPullResponse, AppError> {
    state.db.sync_pull(user_id, since)
}

pub fn push(
    state: &AppState,
    user_id: &str,
    req: &SyncPushRequest,
) -> Result<SyncPushResponse, AppError> {
    if req.base_revision < 0 {
        return Err(AppError::BadRequest(
            "base_revision must be >= 0".to_string(),
        ));
    }
    if req.ops.len() > MAX_SYNC_OPS {
        return Err(AppError::BadRequest(format!(
            "at most {MAX_SYNC_OPS} ops per push"
        )));
    }
    for op in &req.ops {
        validate_op(op)?;
    }
    state.db.sync_push(user_id, req.base_revision, &req.ops)
}

fn validate_id(field: &str, value: &str) -> Result<(), AppError> {
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} required")));
    }
    if value.chars().count() > MAX_SYNC_ID_CHARS {
        return Err(AppError::BadRequest(format!("{field} too long")));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::BadRequest(format!(
            "{field} solo admite letras, números, '-' y '_'"
        )));
    }
    Ok(())
}

fn validate_op(op: &SyncOp) -> Result<(), AppError> {
    match op {
        SyncOp::TrackUpsert {
            track,
            play_count: _,
            last_played,
            ..
        } => {
            service::validate_track(track)?;
            if last_played.is_some_and(|value| value < 0) {
                return Err(AppError::BadRequest("last_played must be >= 0".to_string()));
            }
        }
        SyncOp::TrackDelete { key } => {
            service::validate_text("track.key", key, service::MAX_TRACK_KEY_CHARS)?;
        }
        SyncOp::PlaylistUpsert {
            id,
            name,
            description,
            cover_url,
        } => {
            validate_id("playlist.id", id)?;
            service::validate_text("playlist.name", name, MAX_SYNC_NAME_CHARS)?;
            service::validate_optional_text(
                "playlist.description",
                Some(description),
                MAX_SYNC_DESCRIPTION_CHARS,
            )?;
            service::validate_optional_url(
                "playlist.cover_url",
                cover_url.as_deref(),
                service::MAX_TRACK_URL_CHARS,
            )?;
        }
        SyncOp::PlaylistDelete { id } => validate_id("playlist.id", id)?,
        SyncOp::PlaylistItemUpsert {
            id,
            playlist_id,
            track_key,
            position,
        } => {
            validate_id("item.id", id)?;
            validate_id("item.playlist_id", playlist_id)?;
            service::validate_text("item.track_key", track_key, service::MAX_TRACK_KEY_CHARS)?;
            if !(0..=MAX_SYNC_POSITION).contains(position) {
                return Err(AppError::BadRequest(format!(
                    "item.position must be between 0 and {MAX_SYNC_POSITION}"
                )));
            }
        }
        SyncOp::PlaylistItemDelete { id } => validate_id("item.id", id)?,
    }
    Ok(())
}
