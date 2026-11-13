//! WebSocket `/api/v1/ws?token=…`.
//!
//! Cada conexión tiene un canal de salida (`mpsc`): tanto las respuestas del
//! propio cliente como los difusiones del hub pasan por él. Así el `sink` y el
//! `stream` del socket no se pisan.

use axum::{
    extract::{
        rejection::QueryRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use spote_protocol::{ClientMsg, ControlRequest, Presence, ServerMsg, User};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::{error::AppError, service, service::ReadyOutcome, state::AppState};

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    token: String,
}

/// Handshake: valida el token antes de aceptar el upgrade.
pub async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    query: Result<Query<WsQuery>, QueryRejection>,
) -> Result<Response, AppError> {
    let Query(query) = query.map_err(AppError::from)?;
    let user = state
        .db
        .user_by_token(&query.token)?
        .ok_or_else(|| AppError::Unauthorized("invalid token".to_string()))?;
    Ok(ws.on_upgrade(move |socket| handle(state, user, socket)))
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

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Text(text) => {
                        match serde_json::from_str::<ClientMsg>(text.as_str()) {
                            Ok(client_msg) => {
                                if let Err(error) = handle_client_msg(&state, &user, &conn_id, &tx, client_msg) {
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
    msg: ClientMsg,
) -> Result<(), AppError> {
    match msg {
        ClientMsg::Hello {
            device,
            now_playing,
        } => {
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
            service::touch_presence(state, user, now_playing, playing, true)?;
        }
        ClientMsg::Join { room } => {
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
            state.hub.leave_room(conn_id, &room);
            service::leave_room(state, &user.id, &room)?;
            tracing::info!(user = %user.name, room = %room, "sale de sala");
        }
        ClientMsg::Control {
            action,
            position_ms,
        } => {
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
            let room = active_room(state, conn_id)?;
            service::queue_add(state, &user.id, &room, track)?;
        }
        ClientMsg::QueueRemove { item } => {
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
            let room = active_room(state, conn_id)?;
            service::chat_add(state, user, &room, &text)?;
        }
    }
    Ok(())
}
