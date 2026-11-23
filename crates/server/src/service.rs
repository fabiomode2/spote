//! Lógica de negocio compartida entre REST y WebSocket.
//!
//! Cada operación valida permisos, persiste en SQLite y difunde el resultado a
//! las conexiones de la sala. Los handlers se quedan finos a propósito.

use std::time::{SystemTime, UNIX_EPOCH};

use spote_protocol::{
    ChatMsg, ControlAction, ControlRequest, CreateRoomRequest, LoginRequest, LoginResponse,
    Presence, QueueItem, Room, RoomState, ServerMsg, TrackRef, User, READY_TIMEOUT_MS,
};

use crate::{error::AppError, state::AppState};

/// Margen entre el `start_at` y el arranque real (precarga en los clientes).
pub const START_DELAY_MS: i64 = 800;
pub const MAX_CHAT_CHARS: usize = 2_000;
pub const MAX_NAME_CHARS: usize = 32;
pub const MAX_ROOM_NAME_CHARS: usize = 48;
pub const MAX_TRACK_KEY_CHARS: usize = 256;
pub const MAX_TRACK_TITLE_CHARS: usize = 512;
pub const MAX_TRACK_ARTIST_CHARS: usize = 256;
pub const MAX_TRACK_ALBUM_CHARS: usize = 256;
pub const MAX_TRACK_URL_CHARS: usize = 2_048;
pub const MAX_SOURCE_ID_CHARS: usize = 256;
pub const MAX_TRACK_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
/// Chat que se manda al entrar en una sala.
pub const CHAT_HISTORY_LIMIT: u32 = 50;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Resultado de aplicar un control de transporte.
pub struct ControlOutcome {
    pub state: RoomState,
    pub queue: Vec<QueueItem>,
    pub queue_changed: bool,
}

/// Resultado de un `ready`.
pub enum ReadyOutcome {
    Waiting,
    Started,
}

// ---------------------------------------------------------------------------
// Auth y usuarios
// ---------------------------------------------------------------------------

pub fn login(
    state: &AppState,
    req: &LoginRequest,
    client_key_fp: &str,
) -> Result<LoginResponse, AppError> {
    if !state.config.verify_invite(&req.invite) {
        return Err(AppError::Unauthorized("login rejected".to_string()));
    }
    let name = validate_text("name", &req.name, MAX_NAME_CHARS)?;

    let expires_at = now_ms() + state.config.session_days * 24 * 60 * 60 * 1_000;
    let (user, token) = state.db.login(name, client_key_fp, expires_at)?;
    // Presencia conocida (offline si nunca se conectó) para /users.
    let presence = state
        .db
        .load_presence(&user.id)?
        .unwrap_or_else(|| Presence {
            user: user.clone(),
            online: false,
            last_seen: now_ms(),
            now_playing: None,
        });
    state.hub.ensure_presence(presence);
    let client = state
        .config
        .client_key_label(client_key_fp)
        .unwrap_or("unknown");
    tracing::info!(user = %user.name, client = %client, "login");
    Ok(LoginResponse {
        token,
        user,
        invite: String::new(),
    })
}

/// Todos los usuarios con su presencia (memoria primero, DB de respaldo).
pub fn users(state: &AppState) -> Result<Vec<Presence>, AppError> {
    let mut out = Vec::new();
    for user in state.db.list_users()? {
        let presence = match state.hub.presence_of(&user.id) {
            Some(p) => p,
            None => state
                .db
                .load_presence(&user.id)?
                .unwrap_or_else(|| Presence {
                    user: user.clone(),
                    online: false,
                    last_seen: 0,
                    now_playing: None,
                }),
        };
        out.push(Presence { user, ..presence });
    }
    Ok(out)
}

pub fn add_friend(state: &AppState, user_id: &str, target_id: &str) -> Result<User, AppError> {
    state.db.add_friend(user_id, target_id)
}

/// Actualiza presencia (memoria + DB) y difunde. `immediate` salta el throttle.
pub fn touch_presence(
    state: &AppState,
    user: &User,
    now_playing: Option<TrackRef>,
    playing: bool,
    immediate: bool,
) -> Result<(), AppError> {
    if let Some(track) = &now_playing {
        validate_track(track)?;
    }
    let presence = Presence {
        user: user.clone(),
        online: true,
        last_seen: now_ms(),
        now_playing,
    };
    state.hub.set_presence(presence.clone());
    state.db.upsert_presence(&presence, playing)?;
    state.hub.broadcast_presence(immediate);
    Ok(())
}

// ---------------------------------------------------------------------------
// Salas
// ---------------------------------------------------------------------------

pub fn create_room(
    state: &AppState,
    host: &User,
    req: &CreateRoomRequest,
) -> Result<Room, AppError> {
    let name = validate_text("room name", &req.name, MAX_ROOM_NAME_CHARS)?;
    let room = state.db.create_room(name, req.is_public, &host.id)?;
    tracing::info!(room = %room.id, name = %room.name, host = %host.name, "sala creada");
    Ok(room)
}

pub fn list_rooms(state: &AppState, user_id: &str) -> Result<Vec<Room>, AppError> {
    state.db.rooms_visible(user_id)
}

/// Sala visible para el usuario (pública o miembro).
pub fn room(state: &AppState, user_id: &str, room_id: &str) -> Result<Room, AppError> {
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    if !room.is_public && room.host_id != user_id && !state.db.is_member(room_id, user_id)? {
        return Err(AppError::Forbidden("room is private".to_string()));
    }
    Ok(room)
}

pub fn join_room(state: &AppState, user: &User, room_id: &str) -> Result<Room, AppError> {
    let existing = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    if !existing.is_public
        && existing.host_id != user.id
        && !state.db.is_member(room_id, &user.id)?
    {
        return Err(AppError::Forbidden("room is private".to_string()));
    }
    let room = state.db.join_room(room_id, &user.id)?;
    broadcast_room_state(state, &room);
    Ok(room)
}

/// Añade a un usuario a una sala privada. Solo el host puede hacerlo.
pub fn add_member(
    state: &AppState,
    host_id: &str,
    room_id: &str,
    target_id: &str,
) -> Result<Room, AppError> {
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    if room.host_id != host_id {
        return Err(AppError::Forbidden(
            "only the host can add members".to_string(),
        ));
    }
    if target_id == host_id {
        return Err(AppError::BadRequest("host is already a member".to_string()));
    }
    if state.db.user_by_id(target_id)?.is_none() {
        return Err(AppError::NotFound("user not found".to_string()));
    }
    let room = state.db.join_room(room_id, target_id)?;
    broadcast_room_state(state, &room);
    Ok(room)
}

/// Expulsa a un miembro. Solo el host puede hacerlo.
pub fn kick_member(
    state: &AppState,
    host_id: &str,
    room_id: &str,
    target_id: &str,
) -> Result<Room, AppError> {
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    if room.host_id != host_id {
        return Err(AppError::Forbidden(
            "only the host can kick members".to_string(),
        ));
    }
    if target_id == host_id {
        return Err(AppError::BadRequest("host cannot kick itself".to_string()));
    }
    if !state.db.kick_member(room_id, target_id)? {
        return Err(AppError::NotFound("member not found".to_string()));
    }
    state.hub.evict_user_from_room(target_id, room_id);
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    broadcast_room_state(state, &room);
    Ok(room)
}

pub fn leave_room(state: &AppState, user_id: &str, room_id: &str) -> Result<Room, AppError> {
    state.db.leave_room(room_id, user_id)?;
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    broadcast_room_state(state, &room);
    Ok(room)
}

fn broadcast_room_state(state: &AppState, room: &Room) {
    state.hub.broadcast_room(
        &room.id,
        ServerMsg::RoomState {
            state: room.state.clone(),
            members: room.members.clone(),
        },
    );
}

fn ensure_member(state: &AppState, room_id: &str, user_id: &str) -> Result<(), AppError> {
    if state.db.room(room_id)?.is_none() {
        return Err(AppError::NotFound("room not found".to_string()));
    }
    if !state.db.is_member(room_id, user_id)? {
        return Err(AppError::Forbidden("not a room member".to_string()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Transporte
// ---------------------------------------------------------------------------

pub fn control(
    state: &AppState,
    user_id: &str,
    room_id: &str,
    req: &ControlRequest,
) -> Result<ControlOutcome, AppError> {
    ensure_member(state, room_id, user_id)?;
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    let mut st = room.state;
    let now = now_ms();
    let mut queue_changed = false;

    match req.action {
        ControlAction::Play => {
            if st.track.is_none() {
                if let Some(item) = state.db.queue_pop_front(room_id)? {
                    st.track = Some(item.track);
                    st.position_ms = 0;
                    queue_changed = true;
                }
            }
            if st.track.is_some() {
                st.playing = true;
                st.anchor_ms = now;
                st.start_at_ms = None;
            }
        }
        ControlAction::Pause => {
            st.position_ms = st.target_position(now);
            st.playing = false;
            st.anchor_ms = now;
            st.start_at_ms = None;
        }
        ControlAction::Seek => {
            let position = req
                .position_ms
                .ok_or_else(|| AppError::BadRequest("position_ms required for seek".to_string()))?;
            st.position_ms = position;
            st.anchor_ms = now;
            st.start_at_ms = None;
        }
        ControlAction::Next => {
            let previous = st.track.take();
            if let Some(prev) = previous.as_ref() {
                state.hub.push_history(room_id, prev);
            }
            match state.db.queue_pop_front(room_id)? {
                Some(item) => {
                    st.track = Some(item.track.clone());
                    st.position_ms = 0;
                    st.anchor_ms = now;
                    st.playing = true;
                    st.start_at_ms = None;
                    state.db.log_play_event(room_id, user_id, &item.track, 0)?;
                    queue_changed = true;
                }
                None => {
                    st.position_ms = 0;
                    st.anchor_ms = now;
                    st.playing = false;
                    st.start_at_ms = None;
                }
            }
        }
        ControlAction::Prev => match state.hub.pop_history(room_id) {
            Some(track) => {
                st.track = Some(track);
                st.position_ms = 0;
                st.anchor_ms = now;
                st.playing = true;
                st.start_at_ms = None;
            }
            None => {
                st.position_ms = 0;
                st.anchor_ms = now;
            }
        },
    }

    st.room_id = room_id.to_string();
    st.updated_by = Some(user_id.to_string());
    state.db.save_state(&st)?;

    let queue = state.db.queue_list(room_id)?;
    let members = state.db.members(room_id)?;
    state.hub.broadcast_room(
        room_id,
        ServerMsg::RoomState {
            state: st.clone(),
            members,
        },
    );
    if queue_changed {
        state.hub.broadcast_room(
            room_id,
            ServerMsg::Queue {
                items: queue.clone(),
            },
        );
    }
    Ok(ControlOutcome {
        state: st,
        queue,
        queue_changed,
    })
}

pub fn queue_add(
    state: &AppState,
    user_id: &str,
    room_id: &str,
    track: TrackRef,
) -> Result<QueueItem, AppError> {
    ensure_member(state, room_id, user_id)?;
    validate_track(&track)?;
    let item = state.db.queue_add(room_id, &track, user_id)?;
    broadcast_queue(state, room_id)?;
    Ok(item)
}

pub fn queue_remove(
    state: &AppState,
    user_id: &str,
    room_id: &str,
    item_id: &str,
) -> Result<Vec<QueueItem>, AppError> {
    ensure_member(state, room_id, user_id)?;
    let added_by = state
        .db
        .queue_item_added_by(room_id, item_id)?
        .ok_or_else(|| AppError::NotFound("queue item not found".to_string()))?;
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    if added_by != user_id && room.host_id != user_id {
        return Err(AppError::Forbidden(
            "only the owner or the host can remove this item".to_string(),
        ));
    }
    if !state.db.queue_remove(room_id, item_id)? {
        return Err(AppError::NotFound("queue item not found".to_string()));
    }
    let items = broadcast_queue(state, room_id)?;
    Ok(items)
}

fn broadcast_queue(state: &AppState, room_id: &str) -> Result<Vec<QueueItem>, AppError> {
    let items = state.db.queue_list(room_id)?;
    state.hub.broadcast_room(
        room_id,
        ServerMsg::Queue {
            items: items.clone(),
        },
    );
    Ok(items)
}

// ---------------------------------------------------------------------------
// Ready / arranque sincronizado
// ---------------------------------------------------------------------------

pub fn mark_ready(
    state: &AppState,
    user_id: &str,
    room_id: &str,
    track_key: &str,
    buffered_ms: u64,
) -> Result<ReadyOutcome, AppError> {
    ensure_member(state, room_id, user_id)?;
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    let track = room
        .state
        .track
        .clone()
        .ok_or_else(|| AppError::BadRequest("no track to prepare".to_string()))?;
    if track.key != track_key {
        return Err(AppError::BadRequest(
            "ready for a different track".to_string(),
        ));
    }

    state.db.set_ready(room_id, user_id, true)?;
    tracing::debug!(room = %room_id, user = %user_id, buffered_ms, "peer ready");

    let members = state.db.members(room_id)?;
    let all_ready = !members.is_empty() && members.iter().all(|m| m.ready);
    if !all_ready {
        return Ok(ReadyOutcome::Waiting);
    }

    // Todos listos: invalida cualquier timeout pendiente y arranca ya.
    let seq = state.hub.force_start(room_id);
    if state.hub.claim_start(room_id, seq) {
        start_now(state, room_id, Some(&track.key))?;
    }
    Ok(ReadyOutcome::Started)
}

/// Programa el arranque por timeout si nadie acaba de estar listo.
pub fn spawn_ready_timeout(state: AppState, room_id: String, track_key: String) {
    let Some(seq) = state.hub.begin_start(&room_id) else {
        return;
    };
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(READY_TIMEOUT_MS)).await;
        if !state.hub.claim_start(&room_id, seq) {
            return;
        }
        match start_now(&state, &room_id, Some(&track_key)) {
            Ok(true) => tracing::info!(room = %room_id, "arranque por timeout de ready"),
            Ok(false) => {}
            Err(error) => tracing::warn!(room = %room_id, %error, "no se pudo arrancar la sala"),
        }
    });
}

/// Fija `start_at` a `now + 800ms`, resetea ready y difunde `StartAt` + estado.
/// Devuelve `false` si la sala ha cambiado de pista entretanto.
pub fn start_now(
    state: &AppState,
    room_id: &str,
    expected_track: Option<&str>,
) -> Result<bool, AppError> {
    let now = now_ms();
    let room = state
        .db
        .room(room_id)?
        .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
    let Some(track) = room.state.track.clone() else {
        return Ok(false);
    };
    if let Some(key) = expected_track {
        if key != track.key {
            return Ok(false);
        }
    }
    // Ya hay un arranque en marcha para esta pista: un `ready` tardío no
    // debe volver a anclar la reproducción a cero.
    if room.state.playing && room.state.start_at_ms.is_some() {
        return Ok(false);
    }

    let at_ms = now + START_DELAY_MS;
    let new_state = RoomState {
        room_id: room_id.to_string(),
        track: Some(track.clone()),
        playing: true,
        position_ms: 0,
        anchor_ms: at_ms,
        updated_by: Some("server".to_string()),
        start_at_ms: Some(at_ms),
    };
    state.db.save_state(&new_state)?;
    state.db.reset_ready(room_id)?;
    state.db.log_play_event(room_id, "server", &track, 0)?;

    let members = state.db.members(room_id)?;
    state
        .hub
        .broadcast_room(room_id, ServerMsg::StartAt { track, at_ms });
    state.hub.broadcast_room(
        room_id,
        ServerMsg::RoomState {
            state: new_state,
            members,
        },
    );
    Ok(true)
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

pub fn chat_add(
    state: &AppState,
    user: &User,
    room_id: &str,
    text: &str,
) -> Result<ChatMsg, AppError> {
    ensure_member(state, room_id, &user.id)?;
    let text = validate_text("message", text, MAX_CHAT_CHARS)?;
    let msg = state.db.chat_add(room_id, user, text)?;
    state
        .hub
        .broadcast_room(room_id, ServerMsg::Chat { msg: msg.clone() });
    Ok(msg)
}

// ---------------------------------------------------------------------------
// Validación de texto de usuario
// ---------------------------------------------------------------------------

/// Recorta, limita y rechaza caracteres de control.
///
/// Los mensajes acaban en una terminal: un `\x1b]52;…` o un `\r` pueden cambiar
/// el portapapeles o reescribir la interfaz. Se rechazan en la puerta.
pub(crate) fn validate_text<'a>(
    field: &str,
    value: &'a str,
    max_chars: usize,
) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} required")));
    }
    if value.chars().count() > max_chars {
        return Err(AppError::BadRequest(format!("{field} too long")));
    }
    if value.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "{field} contains control characters"
        )));
    }
    Ok(value)
}

pub(crate) fn validate_optional_text(
    field: &str,
    value: Option<&str>,
    max_chars: usize,
) -> Result<(), AppError> {
    if let Some(value) = value {
        if value.is_empty() {
            return Ok(());
        }
        if value.chars().count() > max_chars {
            return Err(AppError::BadRequest(format!("{field} too long")));
        }
        if value.chars().any(char::is_control) {
            return Err(AppError::BadRequest(format!(
                "{field} contains control characters"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_track(track: &TrackRef) -> Result<(), AppError> {
    validate_text("track.key", &track.key, MAX_TRACK_KEY_CHARS)?;
    validate_optional_text("track.title", Some(&track.title), MAX_TRACK_TITLE_CHARS)?;
    validate_optional_text("track.artist", Some(&track.artist), MAX_TRACK_ARTIST_CHARS)?;
    validate_optional_text("track.album", track.album.as_deref(), MAX_TRACK_ALBUM_CHARS)?;

    if track.duration_ms > MAX_TRACK_DURATION_MS {
        return Err(AppError::BadRequest(
            "track.duration_ms too large".to_string(),
        ));
    }
    validate_optional_url(
        "track.art_url",
        track.art_url.as_deref(),
        MAX_TRACK_URL_CHARS,
    )?;
    if let Some(hint) = &track.source_hint {
        validate_text("source_hint.provider", &hint.provider, 64)?;
        validate_text("source_hint.id", &hint.id, MAX_SOURCE_ID_CHARS)?;
    }
    Ok(())
}

pub(crate) fn validate_optional_url(
    field: &str,
    value: Option<&str>,
    max_chars: usize,
) -> Result<(), AppError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.chars().count() > max_chars {
        return Err(AppError::BadRequest(format!("{field} too long")));
    }
    if !(value.starts_with("https://") || value.starts_with("http://")) {
        return Err(AppError::BadRequest(format!("{field} must be http(s)")));
    }
    if value.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "{field} contains control characters"
        )));
    }
    Ok(())
}
