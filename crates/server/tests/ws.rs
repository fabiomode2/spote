//! Smoke test del WebSocket contra un listener loopback (sin red externa).

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use spote_protocol::{ClientMsg, ServerMsg};
use spote_server::{config::Config, router, state::AppState};
use tokio_tungstenite::tungstenite::Message;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn test_state(dir: &tempfile::TempDir) -> AppState {
    let config = Config {
        bind: "127.0.0.1:0".parse().expect("addr válida"),
        db_path: dir.path().join("ws-test.db"),
        invite: "spote".to_string(),
    };
    AppState::new(config).expect("estado")
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
async fn ws_hello_ping_and_join() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = test_state(&dir);
    let (host, token, _) = state.db.login("ana", "spote").expect("login");
    let room = state.db.create_room("sala", true, &host.id).expect("sala");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });

    let url = format!("ws://{addr}/api/v1/ws?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
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
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/v1/ws?token=nope")).await;
    assert!(rejected.is_err(), "token inválido rechaza el handshake");

    let _ = ws.close(None).await;
    server.abort();
}
