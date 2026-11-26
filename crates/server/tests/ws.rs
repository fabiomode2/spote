//! Smoke test del WebSocket contra un listener loopback (sin red externa).

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use spote_protocol::{ClientMsg, ServerMsg};
use spote_server::{config::Config, router, service, state::AppState};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const INVITE: &str = "test-invite-0123456789abcdef";
const KEY_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn test_state(dir: &tempfile::TempDir) -> AppState {
    let config = Config::from_lookup(|key| match key {
        "SPOTE_INVITE" => Some(INVITE.to_string()),
        "SPOTE_CLIENT_KEYS" => Some(format!("ana:{KEY_A}")),
        "SPOTE_DB" => Some(dir.path().join("ws-test.db").display().to_string()),
        "SPOTE_BIND" => Some("127.0.0.1:0".to_string()),
        _ => None,
    })
    .expect("config de test");
    AppState::new(config).expect("estado")
}

fn ws_request(
    url: &str,
    token: Option<&str>,
    key: Option<&str>,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = url.into_client_request().expect("request WS");
    if let Some(token) = token {
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).expect("header"),
        );
    }
    if let Some(key) = key {
        request
            .headers_mut()
            .insert("x-spote-key", HeaderValue::from_str(key).expect("header"));
    }
    request
}

async fn send(ws: &mut Ws, msg: ClientMsg) {
    let json = serde_json::to_string(&msg).expect("json");
    ws.send(Message::Text(json.into())).await.expect("envío WS");
}

async fn next_server_msg(ws: &mut Ws) -> ServerMsg {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timeout esperando mensaje")
            .expect("stream cerrado")
            .expect("frame válido");
        match message {
            Message::Text(text) => {
                return serde_json::from_str(text.as_str()).expect("json del servidor");
            }
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("frame inesperado: {other:?}"),
        }
    }
}

#[tokio::test]
async fn ws_requires_client_key_and_bearer_then_works() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let fp = state.config.client_key_fingerprint(KEY_A).expect("huella");
    let (host, token) = state
        .db
        .login("ana", &fp, service_now() + 60_000)
        .expect("login");
    let room = state.db.create_room("sala", true, &host.id).expect("sala");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });

    let url = format!("ws://{addr}/api/v1/ws");
    let no_key = tokio_tungstenite::connect_async(ws_request(&url, Some(&token), None)).await;
    assert!(no_key.is_err(), "sin clave de cliente no hay upgrade");

    let no_token = tokio_tungstenite::connect_async(ws_request(&url, None, Some(KEY_A))).await;
    assert!(no_token.is_err(), "sin token no hay upgrade");

    let (mut ws, _) = tokio_tungstenite::connect_async(ws_request(&url, Some(&token), Some(KEY_A)))
        .await
        .expect("handshake");

    send(
        &mut ws,
        ClientMsg::Hello {
            device: "test".to_string(),
            now_playing: None,
        },
    )
    .await;
    let welcome = loop {
        match next_server_msg(&mut ws).await {
            ServerMsg::Welcome { server_time, user } => break (server_time, user),
            ServerMsg::Error { message } => panic!("error del servidor: {message}"),
            _ => {}
        }
    };
    assert!(welcome.0 > 0);
    assert_eq!(welcome.1.id, host.id);

    send(&mut ws, ClientMsg::Ping { t0: 42 }).await;
    let pong = loop {
        match next_server_msg(&mut ws).await {
            ServerMsg::Pong { t0, t1 } => break (t0, t1),
            ServerMsg::Error { message } => panic!("error del servidor: {message}"),
            _ => {}
        }
    };
    assert_eq!(pong.0, 42);
    assert!(pong.1 >= 42);

    send(
        &mut ws,
        ClientMsg::Join {
            room: room.id.clone(),
        },
    )
    .await;
    let joined = loop {
        match next_server_msg(&mut ws).await {
            ServerMsg::RoomState { state, members } => break (state, members),
            ServerMsg::Error { message } => panic!("error del servidor: {message}"),
            _ => {}
        }
    };
    assert_eq!(joined.0.room_id, room.id);
    assert_eq!(joined.1.len(), 1);
    assert_eq!(joined.1[0].user.id, host.id);

    let rejected =
        tokio_tungstenite::connect_async(ws_request(&url, Some("nope"), Some(KEY_A))).await;
    assert!(rejected.is_err(), "token inválido rechaza el handshake");

    let _ = ws.close(None).await;
    server.abort();
}

#[tokio::test]
async fn avatar_update_broadcasts_presence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let fp = state.config.client_key_fingerprint(KEY_A).expect("huella");
    let (user, token) = state
        .db
        .login("ana", &fp, service_now() + 60_000)
        .expect("login");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("addr");
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(server_state)).await;
    });

    let url = format!("ws://{addr}/api/v1/ws");
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_request(&url, Some(&token), Some(KEY_A)))
        .await
        .expect("handshake");

    send(
        &mut ws,
        ClientMsg::Hello {
            device: "test".to_string(),
            now_playing: None,
        },
    )
    .await;
    loop {
        match next_server_msg(&mut ws).await {
            ServerMsg::Welcome { .. } => break,
            ServerMsg::Error { message } => panic!("error del servidor: {message}"),
            _ => {}
        }
    }

    let avatar = format!("data:image/png;base64,{}", "A".repeat(64));
    service::set_avatar(&state, &user, Some(avatar.clone())).expect("avatar");

    let presences = loop {
        match next_server_msg(&mut ws).await {
            ServerMsg::Presence { presences }
                if presences
                    .iter()
                    .any(|presence| presence.user.avatar.is_some()) =>
            {
                break presences;
            }
            ServerMsg::Error { message } => panic!("error del servidor: {message}"),
            _ => {}
        }
    };
    let ana = presences
        .iter()
        .find(|presence| presence.user.id == user.id)
        .expect("ana en presencia");
    assert_eq!(ana.user.avatar.as_deref(), Some(avatar.as_str()));

    let _ = ws.close(None).await;
    server.abort();
}

fn service_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
