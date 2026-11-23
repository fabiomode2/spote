//! Cliente del backend social: login, salas, presencia y sincronización.
//!
//! Vive en un hilo con su propio runtime. La UI manda comandos por un canal y
//! recibe eventos; nunca toca la red desde el hilo de render.

use std::sync::mpsc::Receiver;

use futures_util::{SinkExt, StreamExt};
use spote_core::config::ServerCfg;
use spote_protocol::{
    encode_line, ChatMsg, ClientMsg, ControlAction, LoginRequest, LoginResponse, Presence,
    QueueItem, Room, RoomState, ServerMsg, TimeResponse, TrackRef,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::HeaderValue, Error as WsError, Message,
};

const CLIENT_KEY_HEADER: &str = "x-spote-key";

#[derive(Debug)]
enum ConnectError {
    Unauthorized,
    Other(String),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SocialCmd {
    Refresh,
    Join(String),
    Leave,
    Control(ControlAction, Option<u64>),
    QueueAdd(TrackRef),
    QueueRemove(String),
    NowPlaying(Option<TrackRef>, bool),
}

#[derive(Debug, Clone)]
pub enum SocialEvent {
    Ready {
        user: String,
        token: String,
    },
    Status(String),
    Error(String),
    Rooms(Vec<Room>),
    Room(Room),
    State(RoomState),
    Queue(Vec<QueueItem>),
    Presence(Vec<Presence>),
    Chat(ChatMsg),
    /// Offset estimado cliente→servidor en ms (positivo = servidor adelantado).
    Clock(i64),
}

pub struct Social {
    commands: UnboundedSender<SocialCmd>,
    events: Receiver<SocialEvent>,
}

impl Social {
    pub fn start(cfg: ServerCfg) -> Self {
        let (command_tx, command_rx) = unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("spote-social".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(worker(cfg, command_rx, event_tx));
            })
            .ok();
        Self {
            commands: command_tx,
            events: event_rx,
        }
    }

    pub fn send(&self, command: SocialCmd) {
        let _ = self.commands.send(command);
    }

    pub fn try_recv(&self) -> Option<SocialEvent> {
        self.events.try_recv().ok()
    }
}

async fn worker(
    cfg: ServerCfg,
    mut commands: UnboundedReceiver<SocialCmd>,
    events: std::sync::mpsc::Sender<SocialEvent>,
) {
    let base = cfg.url.trim_end_matches('/').to_string();
    if base.is_empty() {
        return;
    }
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            let _ = events.send(SocialEvent::Error(err.to_string()));
            return;
        }
    };

    if cfg.client_key.trim().is_empty() {
        let _ = events.send(SocialEvent::Error(
            "falta [server] client_key en config.toml".into(),
        ));
        return;
    }

    // 1. Login (o reutiliza el token guardado si sigue valiendo).
    let mut token = cfg.token.clone();
    let mut user = cfg.name.clone();
    if !token.is_empty() && !token_is_valid(&client, &base, &cfg.client_key, &token).await {
        token.clear();
    }
    if token.is_empty() {
        let Some((new_token, new_user)) = login(&client, &base, &cfg, &events).await else {
            return;
        };
        token = new_token;
        user = new_user;
    }
    let _ = events.send(SocialEvent::Status(format!("conectado como {user}")));
    refresh_rooms(&client, &base, &cfg.client_key, &token, &events).await;

    // 2. Reloj: 5 muestras, quedarse con la de menor RTT.
    if let Some(offset) = estimate_clock(&client, &base, &cfg.client_key).await {
        let _ = events.send(SocialEvent::Clock(offset));
    }

    // 3. WebSocket con reconexión. Si la sesión caduca, se renueva sola.
    loop {
        match connect_ws(&base, &cfg.client_key, &token).await {
            Ok(stream) => {
                let _ = events.send(SocialEvent::Status("websocket conectado".into()));
                if run_ws(stream, &mut commands, &events).await {
                    return;
                }
            }
            Err(ConnectError::Unauthorized) => {
                let _ = events.send(SocialEvent::Error(
                    "sesión caducada; renovando login".into(),
                ));
                match login(&client, &base, &cfg, &events).await {
                    Some((new_token, new_user)) => {
                        token = new_token;
                        let _ =
                            events.send(SocialEvent::Status(format!("conectado como {new_user}")));
                        refresh_rooms(&client, &base, &cfg.client_key, &token, &events).await;
                        continue;
                    }
                    None => return,
                }
            }
            Err(ConnectError::Other(err)) => {
                let _ = events.send(SocialEvent::Error(format!("ws: {err}")));
            }
        }
        // Backoff y reintento, drenando comandos para no bloquear la UI.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
        loop {
            match tokio::time::timeout_at(deadline, commands.recv()).await {
                Ok(Some(SocialCmd::Refresh)) => {
                    refresh_rooms(&client, &base, &cfg.client_key, &token, &events).await;
                }
                Ok(Some(_)) | Ok(None) => {}
                Err(_) => break,
            }
        }
        if let Ok(response) = client
            .get(format!("{base}/api/v1/time"))
            .header(CLIENT_KEY_HEADER, &cfg.client_key)
            .send()
            .await
        {
            if let Ok(time) = response.json::<TimeResponse>().await {
                let _ = time;
            }
        }
    }
}

async fn token_is_valid(
    client: &reqwest::Client,
    base: &str,
    client_key: &str,
    token: &str,
) -> bool {
    match client
        .get(format!("{base}/api/v1/me"))
        .bearer_auth(token)
        .header(CLIENT_KEY_HEADER, client_key)
        .send()
        .await
    {
        Ok(response) => !matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ),
        // Sin red no se descarta el token: se reintenta.
        Err(_) => true,
    }
}

async fn login(
    client: &reqwest::Client,
    base: &str,
    cfg: &ServerCfg,
    events: &std::sync::mpsc::Sender<SocialEvent>,
) -> Option<(String, String)> {
    if cfg.invite.trim().is_empty() {
        let _ = events.send(SocialEvent::Error(
            "falta [server] invite en config.toml".into(),
        ));
        return None;
    }
    let name = if cfg.name.is_empty() {
        hostname()
    } else {
        cfg.name.clone()
    };
    let request = LoginRequest {
        name: name.clone(),
        invite: cfg.invite.clone(),
    };
    match client
        .post(format!("{base}/api/v1/auth/login"))
        .header(CLIENT_KEY_HEADER, &cfg.client_key)
        .json(&request)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            match response.json::<LoginResponse>().await {
                Ok(login) => {
                    let _ = events.send(SocialEvent::Ready {
                        user: login.user.name.clone(),
                        token: login.token.clone(),
                    });
                    Some((login.token, login.user.name))
                }
                Err(err) => {
                    let _ = events.send(SocialEvent::Error(format!("login: {err}")));
                    None
                }
            }
        }
        Ok(response) if response.status() == reqwest::StatusCode::UNAUTHORIZED => {
            let _ = events.send(SocialEvent::Error(
                "login rechazado · revisa [server] client_key, invite y nombre".into(),
            ));
            None
        }
        Ok(response) => {
            let _ = events.send(SocialEvent::Error(format!(
                "login rechazado ({})",
                response.status()
            )));
            None
        }
        Err(err) => {
            let _ = events.send(SocialEvent::Error(format!("servidor: {err}")));
            None
        }
    }
}

fn ws_url(base: &str) -> String {
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{base}")
    };
    format!("{ws}/api/v1/ws")
}

async fn connect_ws(
    base: &str,
    client_key: &str,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ConnectError,
> {
    let mut request = ws_url(base)
        .into_client_request()
        .map_err(|err| ConnectError::Other(err.to_string()))?;
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|err| ConnectError::Other(err.to_string()))?,
    );
    request.headers_mut().insert(
        CLIENT_KEY_HEADER,
        HeaderValue::from_str(client_key).map_err(|err| ConnectError::Other(err.to_string()))?,
    );
    match tokio_tungstenite::connect_async(request).await {
        Ok((stream, _)) => Ok(stream),
        Err(WsError::Http(response))
            if response.status() == reqwest::StatusCode::UNAUTHORIZED
                || response.status() == reqwest::StatusCode::FORBIDDEN =>
        {
            Err(ConnectError::Unauthorized)
        }
        Err(err) => Err(ConnectError::Other(err.to_string())),
    }
}

/// Devuelve `true` si el hilo debe terminar (canal cerrado).
async fn run_ws<S>(
    stream: tokio_tungstenite::WebSocketStream<S>,
    commands: &mut UnboundedReceiver<SocialCmd>,
    events: &std::sync::mpsc::Sender<SocialEvent>,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut write, mut read) = stream.split();
    let hello = ClientMsg::Hello {
        device: hostname(),
        now_playing: None,
    };
    if send_msg(&mut write, &hello).await.is_err() {
        return false;
    }
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_now: Option<Option<TrackRef>> = None;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let _ = send_msg(&mut write, &ClientMsg::Ping { t0: now_ms() }).await;
            }
            command = commands.recv() => {
                let Some(command) = command else { return true };
                let message = match command {
                    SocialCmd::Refresh => continue,
                    SocialCmd::Join(room) => ClientMsg::Join { room },
                    SocialCmd::Leave => ClientMsg::Leave { room: String::new() },
                    SocialCmd::Control(action, position_ms) => {
                        ClientMsg::Control { action, position_ms }
                    }
                    SocialCmd::QueueAdd(track) => ClientMsg::QueueAdd { track },
                    SocialCmd::QueueRemove(item) => ClientMsg::QueueRemove { item },
                    SocialCmd::NowPlaying(track, playing) => {
                        last_now = Some(track.clone());
                        ClientMsg::Presence { now_playing: track, playing }
                    }
                };
                if send_msg(&mut write, &message).await.is_err() {
                    return false;
                }
            }
            incoming = read.next() => {
                let Some(Ok(message)) = incoming else { return false };
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    Message::Close(_) => return false,
                    _ => continue,
                };
                for line in text.lines() {
                    let Ok(message) = serde_json::from_str::<ServerMsg>(line) else {
                        continue;
                    };
                    match message {
                        ServerMsg::Welcome { server_time, user } => {
                            let _ = server_time;
                            let _ = events.send(SocialEvent::Status(format!(
                                "sesión con {}",
                                user.name
                            )));
                        }
                        ServerMsg::Pong { t0, t1 } => {
                            let now = now_ms();
                            let rtt = now - t0;
                            let offset = ((t1 - t0) + (t1 - now)) / 2;
                            let _ = (rtt, offset);
                        }
                        ServerMsg::RoomState { state, members } => {
                            let _ = events.send(SocialEvent::Room(Room {
                                id: state.room_id.clone(),
                                name: String::new(),
                                host_id: String::new(),
                                is_public: true,
                                members,
                                state: state.clone(),
                            }));
                            let _ = events.send(SocialEvent::State(state));
                        }
                        ServerMsg::Queue { items } => {
                            let _ = events.send(SocialEvent::Queue(items));
                        }
                        ServerMsg::StartAt { track, at_ms } => {
                            let _ = events.send(SocialEvent::State(RoomState {
                                playing: true,
                                position_ms: 0,
                                anchor_ms: at_ms,
                                track: Some(track),
                                ..Default::default()
                            }));
                        }
                        ServerMsg::Presence { presences } => {
                            let _ = events.send(SocialEvent::Presence(presences));
                        }
                        ServerMsg::Chat { msg } => {
                            let _ = events.send(SocialEvent::Chat(msg));
                        }
                        ServerMsg::Error { message } => {
                            let _ = events.send(SocialEvent::Error(message));
                        }
                    }
                }
                let _ = &last_now;
            }
        }
    }
}

async fn send_msg<S>(write: &mut S, message: &ClientMsg) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let Ok(text) = encode_line(message) else {
        return Err(());
    };
    write.send(Message::Text(text.into())).await.map_err(|_| ())
}

async fn refresh_rooms(
    client: &reqwest::Client,
    base: &str,
    client_key: &str,
    token: &str,
    events: &std::sync::mpsc::Sender<SocialEvent>,
) {
    match client
        .get(format!("{base}/api/v1/rooms"))
        .bearer_auth(token)
        .header(CLIENT_KEY_HEADER, client_key)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            if let Ok(rooms) = response.json::<Vec<Room>>().await {
                let _ = events.send(SocialEvent::Rooms(rooms));
            }
        }
        Ok(response) => {
            let _ = events.send(SocialEvent::Error(format!(
                "salas: HTTP {}",
                response.status()
            )));
        }
        Err(err) => {
            let _ = events.send(SocialEvent::Error(format!("salas: {err}")));
        }
    }
}

async fn estimate_clock(client: &reqwest::Client, base: &str, client_key: &str) -> Option<i64> {
    let mut best: Option<(i64, i64)> = None;
    for _ in 0..5 {
        let t0 = now_ms();
        let response = client
            .get(format!("{base}/api/v1/time"))
            .header(CLIENT_KEY_HEADER, client_key)
            .send()
            .await
            .ok()?;
        let time: TimeResponse = response.json().await.ok()?;
        let t1 = now_ms();
        let rtt = t1 - t0;
        let offset = ((time.t - t0) + (time.t - t1)) / 2;
        if best.is_none_or(|(best_rtt, _)| rtt < best_rtt) {
            best = Some((rtt, offset));
        }
    }
    best.map(|(_, offset)| offset)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|text| text.trim().to_string())
        .ok()
        .filter(|text| !text.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "spote".into())
}
