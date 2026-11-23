//! WebSocket `/api/v1/ws`.
//!
//! El handshake exige la clave de cliente (`X-Spote-Key`) y el token de sesión
//! (`Authorization: Bearer …`). Nunca se acepta el token por query string: las
//! URLs acaban en logs de nginx, proxies y navegadores.
//!
//! Cada conexión tiene un canal de salida (`mpsc`): tanto las respuestas del
//! propio cliente como las difusiones del hub pasan por él. Así el `sink` y el
//! `stream` del socket no se pisan.

use std::time::{Duration, Instant};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use spote_protocol::{ClientMsg, ControlRequest, Presence, ServerMsg, User};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{auth::AuthUser, error::AppError, service, service::ReadyOutcome, state::AppState};

/// Un chat de 2.000 caracteres o un `TrackRef` con carátula caben de sobra.
const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;
/// El cliente manda ping cada 10 s; 90 s sin nada es una conexión muerta.
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Handshake: el middleware ya validó la clave de cliente; aquí se valida la
/// sesión y se aplican los límites de conexión antes del upgrade.
pub async fn upgrade(
    State(state): State<AppState>,
    auth: AuthUser,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if !state.hub.can_accept_connection(&auth.0.id) {
        return Err(AppError::TooManyRequests(
            "too many websocket connections".to_string(),
        ));
    }
    let user = auth.0;
    Ok(ws
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle(state, user, socket)))
}

async fn handle(state: AppState, user: User, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ServerMsg>();
    let conn_id = Uuid::new_v4().to_string();

    let initial = match state.db.load_presence(&user.id) {
        Ok(Some(mut presence)) => {
            presence.user = user.clone();
            presence.online = true;
            presence.last_seen = service::now_ms();
            presence
        }
        _ => Presence {
            user: user.clone(),
            online: true,
            last_seen: service::now_ms(),
            now_playing: None,
        },
    };

    state
        .hub
        .connect(&conn_id, &user, tx.clone(), initial.now_playing.clone());
    if let Err(error) = state.db.upsert_presence(&initial, false) {
        tracing::warn!(%error, "no se pudo guardar la presencia inicial");
    }
    // Presencia online/offline va con throttle; el flusher periódico la vacía.
    state.hub.broadcast_presence(false);
    tracing::info!(user = %user.name, "ws conectado");

    let mut gates = Gates::default();
    loop {
        tokio::select! {
            incoming = tokio::time::timeout(WS_IDLE_TIMEOUT, stream.next()) => {
                let incoming = match incoming {
                    Ok(value) => value,
                    Err(_) => {
                        tracing::debug!(user = %user.name, "ws idle timeout");
                        break;
                    }
                };
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Text(text) => {
                        match serde_json::from_str::<ClientMsg>(text.as_str()) {
                            Ok(client_msg) => {
                                if let Err(error) = handle_client_msg(&state, &user, &conn_id, &tx, &mut gates, client_msg) {
                                    let _ = tx.send(ServerMsg::Error { message: error.message().to_string() });
                                }
                            }
                            Err(error) => {
                                let _ = tx.send(ServerMsg::Error {
                                    message: format!("invalid message: {error}"),
                                });
                            }
                        }
                    }
                    Message::Binary(_) => {
                        let _ = tx.send(ServerMsg::Error {
                            message: "binary frames not supported".to_string(),
                        });
                    }
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }
            outgoing = rx.recv() => {
                let Some(server_msg) = outgoing else { break };
                match serde_json::to_string(&server_msg) {
                    Ok(json) => {
                        if sink.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => tracing::error!(%error, "no se pudo serializar un mensaje WS"),
                }
            }
        }
    }

    if let Some(presence) = state.hub.disconnect(&conn_id) {
        if let Err(error) = state.db.upsert_presence(&presence, false) {
            tracing::warn!(%error, "no se pudo guardar la presencia al desconectar");
        }
    }
    state.hub.broadcast_presence(false);
    tracing::info!(user = %user.name, "ws desconectado");
}

fn active_room(state: &AppState, conn_id: &str) -> Result<String, AppError> {
    state
        .hub
        .active_room(conn_id)
        .ok_or_else(|| AppError::BadRequest("join a room first".to_string()))
}

fn handle_client_msg(
    state: &AppState,
    user: &User,
    conn_id: &str,
    tx: &UnboundedSender<ServerMsg>,
    gates: &mut Gates,
    msg: ClientMsg,
) -> Result<(), AppError> {
    match msg {
        ClientMsg::Hello {
            device,
            now_playing,
        } => {
            service::validate_text("device", &device, 64)?;
            tracing::info!(user = %user.name, device = %device, "hello");
            let _ = tx.send(ServerMsg::Welcome {
                server_time: service::now_ms(),
                user: user.clone(),
            });
            service::touch_presence(state, user, now_playing, false, true)?;
        }
        ClientMsg::Ping { t0 } => {
            let _ = tx.send(ServerMsg::Pong {
                t0,
                t1: service::now_ms(),
            });
        }
        ClientMsg::Presence {
            now_playing,
            playing,
        } => {
            if !gates.presence.allow() {
                return Err(AppError::TooManyRequests(
                    "presence too frequent".to_string(),
                ));
            }
            service::touch_presence(state, user, now_playing, playing, true)?;
        }
        ClientMsg::Join { room } => {
            if let Some(previous) = state.hub.active_room(conn_id) {
                if previous != room {
                    state.hub.leave_room(conn_id, &previous);
                    let _ = service::leave_room(state, &user.id, &previous);
                }
            }
            let joined = service::join_room(state, user, &room)?;
            state.hub.join_room(conn_id, &room);
            let _ = tx.send(ServerMsg::RoomState {
                state: joined.state,
                members: joined.members,
            });
            let _ = tx.send(ServerMsg::Queue {
                items: state.db.queue_list(&room)?,
            });
            for msg in state.db.chat_history(&room, service::CHAT_HISTORY_LIMIT)? {
                let _ = tx.send(ServerMsg::Chat { msg });
            }
            tracing::info!(user = %user.name, room = %room, "entra en sala");
        }
        ClientMsg::Leave { room } => {
            let room = if room.is_empty() {
                state.hub.active_room(conn_id).unwrap_or_default()
            } else {
                room
            };
            if room.is_empty() {
                return Ok(());
            }
            state.hub.leave_room(conn_id, &room);
            let _ = service::leave_room(state, &user.id, &room);
            tracing::info!(user = %user.name, room = %room, "sale de sala");
        }
        ClientMsg::Control {
            action,
            position_ms,
        } => {
            if !gates.control.allow() {
                return Err(AppError::TooManyRequests("too many controls".to_string()));
            }
            let room = active_room(state, conn_id)?;
            service::control(
                state,
                &user.id,
                &room,
                &ControlRequest {
                    action,
                    position_ms,
                },
            )?;
        }
        ClientMsg::QueueAdd { track } => {
            if !gates.queue.allow() {
                return Err(AppError::TooManyRequests("queue is busy".to_string()));
            }
            let room = active_room(state, conn_id)?;
            service::queue_add(state, &user.id, &room, track)?;
        }
        ClientMsg::QueueRemove { item } => {
            if !gates.queue.allow() {
                return Err(AppError::TooManyRequests("queue is busy".to_string()));
            }
            let room = active_room(state, conn_id)?;
            service::queue_remove(state, &user.id, &room, &item)?;
        }
        ClientMsg::Ready {
            track_key,
            buffered_ms,
        } => {
            let room = active_room(state, conn_id)?;
            match service::mark_ready(state, &user.id, &room, &track_key, buffered_ms)? {
                ReadyOutcome::Started => {}
                ReadyOutcome::Waiting => {
                    service::spawn_ready_timeout(state.clone(), room, track_key);
                }
            }
        }
        ClientMsg::Chat { text } => {
            if !gates.chat.allow() {
                return Err(AppError::TooManyRequests("chat is too fast".to_string()));
            }
            let room = active_room(state, conn_id)?;
            service::chat_add(state, user, &room, &text)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Límites por conexión
// ---------------------------------------------------------------------------

struct RateGate {
    limit: u32,
    window: Duration,
    start: Instant,
    count: u32,
}

impl RateGate {
    fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            start: Instant::now(),
            count: 0,
        }
    }

    fn allow(&mut self) -> bool {
        if self.start.elapsed() >= self.window {
            self.start = Instant::now();
            self.count = 0;
        }
        if self.count >= self.limit {
            return false;
        }
        self.count += 1;
        true
    }
}

struct Gates {
    chat: RateGate,
    queue: RateGate,
    control: RateGate,
    presence: RateGate,
}

impl Default for Gates {
    fn default() -> Self {
        Self {
            chat: RateGate::new(10, Duration::from_secs(10)),
            queue: RateGate::new(30, Duration::from_secs(60)),
            control: RateGate::new(60, Duration::from_secs(10)),
            presence: RateGate::new(20, Duration::from_secs(10)),
        }
    }
}
