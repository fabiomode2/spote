//! Contrato compartido entre la TUI, el servidor social y los futuros clientes.
//!
//! Nada de esto puede depender de rutas locales ni de detalles de una UI. Es la
//! API pública del proyecto: si cambia, se cambia en los tres sitios a la vez y
//! se sube la versión de `/api/v1`.

use serde::{Deserialize, Serialize};

/// Unix timestamp en milisegundos.
pub type Ts = i64;

// ---------------------------------------------------------------------------
// Canción
// ---------------------------------------------------------------------------

/// Pista en la fuente que la resolvió (YouTube, SoundCloud, local...).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceHint {
    pub provider: String,
    pub id: String,
    #[serde(default)]
    pub duration_ms: u64,
}

/// Identidad canónica de una canción, independiente del proveedor de audio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrackRef {
    /// `isrc:…`, `sha1:…` o `spotify:track:…`
    pub key: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub art_url: Option<String>,
    #[serde(default)]
    pub source_hint: Option<SourceHint>,
}

impl TrackRef {
    pub fn new(key: impl Into<String>, title: impl Into<String>, artist: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            artist: artist.into(),
            ..Default::default()
        }
    }

    /// Texto de una línea para logs, notificaciones y tests.
    pub fn label(&self) -> String {
        if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} — {}", self.artist, self.title)
        }
    }

    /// `true` cuando la duración encaja dentro de la tolerancia de una sala.
    pub fn duration_matches(&self, other_ms: u64, tolerance_ms: u64) -> bool {
        if self.duration_ms == 0 || other_ms == 0 {
            return true;
        }
        self.duration_ms.abs_diff(other_ms) <= tolerance_ms
    }
}

// ---------------------------------------------------------------------------
// Usuarios y presencia
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Presence {
    pub user: User,
    pub online: bool,
    pub last_seen: Ts,
    #[serde(default)]
    pub now_playing: Option<TrackRef>,
}

// ---------------------------------------------------------------------------
// Salas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Guest,
    Host,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Member {
    pub user: User,
    #[serde(default)]
    pub role: Role,
    #[serde(default)]
    pub ready: bool,
}

/// Estado de reproducción de una sala. El servidor es la autoridad del tiempo:
/// el cliente reconstruye la posición con `anchor_ms`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RoomState {
    pub room_id: String,
    #[serde(default)]
    pub track: Option<TrackRef>,
    pub playing: bool,
    /// Posición de la pista en el instante `anchor_ms`.
    pub position_ms: u64,
    /// Reloj del servidor cuando se fijó `position_ms`.
    pub anchor_ms: Ts,
    #[serde(default)]
    pub updated_by: Option<String>,
    /// Cambio de pista programado (todos entran a la vez).
    #[serde(default)]
    pub start_at_ms: Option<Ts>,
}

impl RoomState {
    /// Posición esperada en el instante `server_now`.
    pub fn target_position(&self, server_now: Ts) -> u64 {
        if !self.playing {
            return self.position_ms;
        }
        let elapsed = (server_now - self.anchor_ms).max(0) as u64;
        self.position_ms.saturating_add(elapsed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueItem {
    pub id: String,
    pub track: TrackRef,
    pub added_by: String,
    pub position: i64,
    #[serde(default)]
    pub votes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub host_id: String,
    #[serde(default)]
    pub is_public: bool,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub state: RoomState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMsg {
    pub id: String,
    pub room_id: String,
    pub user_id: String,
    pub user_name: String,
    pub text: String,
    pub ts: Ts,
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Play,
    Pause,
    Next,
    Prev,
    Seek,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        device: String,
        #[serde(default)]
        now_playing: Option<TrackRef>,
    },
    Ping {
        t0: Ts,
    },
    Join {
        room: String,
    },
    Leave {
        room: String,
    },
    Control {
        action: ControlAction,
        #[serde(default)]
        position_ms: Option<u64>,
    },
    QueueAdd {
        track: TrackRef,
    },
    QueueRemove {
        item: String,
    },
    /// El cliente ya tiene los primeros segundos de la pista lista.
    Ready {
        track_key: String,
        buffered_ms: u64,
    },
    Presence {
        #[serde(default)]
        now_playing: Option<TrackRef>,
        #[serde(default)]
        playing: bool,
    },
    Chat {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        server_time: Ts,
        user: User,
    },
    Pong {
        t0: Ts,
        t1: Ts,
    },
    RoomState {
        state: RoomState,
        #[serde(default)]
        members: Vec<Member>,
    },
    Queue {
        items: Vec<QueueItem>,
    },
    StartAt {
        track: TrackRef,
        at_ms: Ts,
    },
    Presence {
        presences: Vec<Presence>,
    },
    Chat {
        msg: ChatMsg,
    },
    Error {
        message: String,
    },
}

/// Una línea de un protocolo enmarcado por saltos de línea (NDJSON).
pub fn encode_line<T: Serialize>(value: &T) -> serde_json::Result<String> {
    let mut s = serde_json::to_string(value)?;
    s.push('\n');
    Ok(s)
}

// ---------------------------------------------------------------------------
// REST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub name: String,
    #[serde(default)]
    pub invite: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
    pub invite: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
    #[serde(default = "default_true")]
    pub is_public: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlRequest {
    pub action: ControlAction,
    #[serde(default)]
    pub position_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeResponse {
    pub t: Ts,
}

/// Tolerancia de duración al resolver un `TrackRef` dentro de una sala.
pub const ROOM_DURATION_TOLERANCE_MS: u64 = 2_000;
/// Tolerancia al importar (no hay nadie sincronizado todavía).
pub const IMPORT_DURATION_TOLERANCE_MS: u64 = 4_000;
/// Ventana de espera a que todos los peers estén listos.
pub const READY_TIMEOUT_MS: u64 = 4_000;
/// Deriva a partir de la cual se hace seek en vez de corregir con velocidad.
pub const SEEK_THRESHOLD_MS: i64 = 400;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_position_only_advances_when_playing() {
        let state = RoomState {
            playing: true,
            position_ms: 10_000,
            anchor_ms: 1_000,
            ..Default::default()
        };
        assert_eq!(state.target_position(2_000), 11_000);
        assert_eq!(state.target_position(500), 10_000);
        let paused = RoomState {
            playing: false,
            ..state
        };
        assert_eq!(paused.target_position(99_999), 10_000);
    }

    #[test]
    fn duration_match_handles_unknown_durations() {
        let track = TrackRef {
            duration_ms: 214_000,
            ..TrackRef::default()
        };
        assert!(track.duration_matches(215_500, 2_000));
        assert!(!track.duration_matches(220_000, 2_000));
        assert!(track.duration_matches(0, 2_000));
    }

    #[test]
    fn websocket_messages_roundtrip() {
        let msg = ClientMsg::Control {
            action: ControlAction::Seek,
            position_ms: Some(42_000),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"control\""));
        assert_eq!(serde_json::from_str::<ClientMsg>(&json).unwrap(), msg);

        let server = ServerMsg::Welcome {
            server_time: 7,
            user: User {
                id: "u".into(),
                name: "n".into(),
                avatar: None,
            },
        };
        let json = serde_json::to_string(&server).unwrap();
        assert!(json.contains("\"type\":\"welcome\""));
        assert_eq!(serde_json::from_str::<ServerMsg>(&json).unwrap(), server);
    }

    #[test]
    fn track_ref_roundtrip_keeps_unknown_fields_optional() {
        let track = TrackRef::new("isrc:X", "Title", "Artist");
        let json = serde_json::to_string(&track).unwrap();
        // `None` serializa como null: el contrato no omite campos, así los tres
        // clientes ven la misma forma.
        assert!(json.contains("\"album\":null"));
        let parsed: TrackRef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, track);
    }
}
