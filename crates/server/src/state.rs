//! Estado compartido del servidor: base de datos, configuración y el hub con
//! la presencia y las conexiones WebSocket abiertas en memoria.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::Duration,
};

use spote_protocol::{Presence, ServerMsg, TrackRef, User};
use tokio::sync::mpsc::UnboundedSender;

use crate::{config::Config, db::Db, error::AppError, service::now_ms};

/// Ventana mínima entre broadcasts de presencia "normales" (ms).
pub const PRESENCE_THROTTLE_MS: i64 = 1_000;
/// Conexiones WebSocket simultáneas por usuario y totales. Con cuatro amigos
/// sobra de largo, pero evita que una clave filtrada abra miles de sockets.
pub const MAX_WS_CONNS_PER_USER: usize = 8;
pub const MAX_WS_CONNS_TOTAL: usize = 128;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub config: Arc<Config>,
    pub hub: Arc<Hub>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self, AppError> {
        let db = Db::open(&config.db_path)?;
        // Un cierre sucio pudo dejar `online = 1`; al arrancar no hay conexiones.
        db.mark_all_offline()?;
        let hub = Hub::default();
        // Presencias conocidas de la DB: así /users y los broadcasts incluyen
        // también a los que están offline.
        hub.seed_presences(db.load_all_presences()?);
        Ok(Self {
            db: Arc::new(db),
            config: Arc::new(config),
            hub: Arc::new(hub),
        })
    }

    /// Tarea periódica que vacía la presencia "sucia" (la que el throttle dejó
    /// pendiente). Requiere runtime de tokio.
    pub fn spawn_background(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                state.hub.flush_presence();
            }
        });

        // Limpieza de sesiones caducadas. No toca nada más.
        let state = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(60 * 60));
            loop {
                ticker.tick().await;
                match state.db.delete_expired_sessions() {
                    Ok(removed) if removed > 0 => {
                        tracing::info!(removed, "sesiones caducadas eliminadas");
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "no se pudieron limpiar sesiones"),
                }
            }
        });
    }
}

struct Conn {
    user_id: String,
    tx: UnboundedSender<ServerMsg>,
    /// Salas a las que está suscrita esta conexión.
    rooms: HashSet<String>,
    /// Sala "activa" para los mensajes que no llevan `room` (control, cola...).
    active_room: Option<String>,
}

#[derive(Default)]
struct HubInner {
    conns: HashMap<String, Conn>,
    presence: HashMap<String, Presence>,
    last_presence_ms: i64,
    presence_dirty: bool,
    /// Salas con un arranque programado (handshake de `ready`).
    pending_starts: HashSet<String>,
    /// Generación del arranque por sala: invalida timeouts viejos.
    start_seq: HashMap<String, u64>,
    /// Historial en memoria para `prev` (no se persiste: es cosmético).
    history: HashMap<String, Vec<TrackRef>>,
}

/// Presencia y conexiones en memoria.
#[derive(Default)]
pub struct Hub {
    inner: RwLock<HubInner>,
}

impl Hub {
    fn read(&self) -> RwLockReadGuard<'_, HubInner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, HubInner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn seed_presences(&self, presences: Vec<Presence>) {
        let mut inner = self.write();
        for presence in presences {
            inner
                .presence
                .entry(presence.user.id.clone())
                .or_insert(presence);
        }
    }

    /// Inserta la presencia solo si no la había (no pisa un `online` vivo).
    pub fn ensure_presence(&self, presence: Presence) {
        self.write()
            .presence
            .entry(presence.user.id.clone())
            .or_insert(presence);
    }

    pub fn presence_of(&self, user_id: &str) -> Option<Presence> {
        self.read().presence.get(user_id).cloned()
    }

    pub fn snapshot(&self) -> Vec<Presence> {
        let mut presences: Vec<Presence> = self.read().presence.values().cloned().collect();
        presences.sort_by(|a, b| a.user.name.cmp(&b.user.name));
        presences
    }

    pub fn set_presence(&self, presence: Presence) {
        self.write()
            .presence
            .insert(presence.user.id.clone(), presence);
    }

    /// Registra una conexión. Devuelve `true` si el usuario pasa de offline a online.
    pub fn connect(
        &self,
        conn_id: &str,
        user: &User,
        tx: UnboundedSender<ServerMsg>,
        now_playing: Option<TrackRef>,
    ) -> bool {
        let mut inner = self.write();
        let was_online = inner
            .presence
            .get(&user.id)
            .map(|p| p.online)
            .unwrap_or(false);
        inner.presence.insert(
            user.id.clone(),
            Presence {
                user: user.clone(),
                online: true,
                last_seen: now_ms(),
                now_playing,
            },
        );
        inner.conns.insert(
            conn_id.to_string(),
            Conn {
                user_id: user.id.clone(),
                tx,
                rooms: HashSet::new(),
                active_room: None,
            },
        );
        !was_online
    }

    /// Límite de conexiones antes de aceptar un WebSocket nuevo.
    pub fn can_accept_connection(&self, user_id: &str) -> bool {
        let inner = self.read();
        if inner.conns.len() >= MAX_WS_CONNS_TOTAL {
            return false;
        }
        inner
            .conns
            .values()
            .filter(|conn| conn.user_id == user_id)
            .count()
            < MAX_WS_CONNS_PER_USER
    }

    /// Deja de suscribir todas las conexiones de un usuario a una sala.
    pub fn evict_user_from_room(&self, user_id: &str, room_id: &str) {
        let mut inner = self.write();
        for conn in inner.conns.values_mut() {
            if conn.user_id == user_id && conn.rooms.remove(room_id) {
                if conn.active_room.as_deref() == Some(room_id) {
                    conn.active_room = None;
                }
                let _ = conn.tx.send(ServerMsg::Error {
                    message: "removed from room".to_string(),
                });
            }
        }
    }

    /// Da de baja la conexión. Devuelve la presencia resultante del usuario.
    pub fn disconnect(&self, conn_id: &str) -> Option<Presence> {
        let mut inner = self.write();
        let conn = inner.conns.remove(conn_id)?;
        let still_connected = inner.conns.values().any(|c| c.user_id == conn.user_id);
        let presence = inner.presence.get_mut(&conn.user_id)?;
        if !still_connected {
            presence.online = false;
        }
        presence.last_seen = now_ms();
        Some(presence.clone())
    }

    pub fn join_room(&self, conn_id: &str, room_id: &str) {
        let mut inner = self.write();
        if let Some(conn) = inner.conns.get_mut(conn_id) {
            conn.rooms.insert(room_id.to_string());
            conn.active_room = Some(room_id.to_string());
        }
    }

    pub fn leave_room(&self, conn_id: &str, room_id: &str) {
        let mut inner = self.write();
        if let Some(conn) = inner.conns.get_mut(conn_id) {
            conn.rooms.remove(room_id);
            if conn.active_room.as_deref() == Some(room_id) {
                conn.active_room = None;
            }
        }
    }

    pub fn active_room(&self, conn_id: &str) -> Option<String> {
        self.read()
            .conns
            .get(conn_id)
            .and_then(|c| c.active_room.clone())
    }

    /// Envía un mensaje a todas las conexiones suscritas a la sala.
    pub fn broadcast_room(&self, room_id: &str, msg: ServerMsg) {
        let inner = self.read();
        for conn in inner.conns.values() {
            if conn.rooms.contains(room_id) {
                let _ = conn.tx.send(msg.clone());
            }
        }
    }

    /// Difunde la presencia a todos los conectados.
    ///
    /// Con `immediate = false` aplica el throttle de 1/s y deja el cambio
    /// pendiente para el siguiente tick (ver [`Hub::flush_presence`]). Los
    /// cambios de `now_playing`, conexiones y desconexiones van inmediatos.
    pub fn broadcast_presence(&self, immediate: bool) {
        let mut inner = self.write();
        let now = now_ms();
        if !immediate && now - inner.last_presence_ms < PRESENCE_THROTTLE_MS {
            inner.presence_dirty = true;
            return;
        }
        inner.last_presence_ms = now;
        inner.presence_dirty = false;
        let mut presences: Vec<Presence> = inner.presence.values().cloned().collect();
        presences.sort_by(|a, b| a.user.name.cmp(&b.user.name));
        let msg = ServerMsg::Presence { presences };
        for conn in inner.conns.values() {
            let _ = conn.tx.send(msg.clone());
        }
    }

    /// Emite la presencia pendiente si el throttle la dejó a medias.
    pub fn flush_presence(&self) {
        let dirty = self.read().presence_dirty;
        if dirty {
            self.broadcast_presence(true);
        }
    }

    pub fn push_history(&self, room_id: &str, track: &TrackRef) {
        let mut inner = self.write();
        let history = inner.history.entry(room_id.to_string()).or_default();
        history.push(track.clone());
        if history.len() > 32 {
            history.remove(0);
        }
    }

    pub fn pop_history(&self, room_id: &str) -> Option<TrackRef> {
        self.write()
            .history
            .get_mut(room_id)
            .and_then(|history| history.pop())
    }

    /// Marca un arranque pendiente. `None` si la sala ya tenía uno.
    pub fn begin_start(&self, room_id: &str) -> Option<u64> {
        let mut inner = self.write();
        if inner.pending_starts.contains(room_id) {
            return None;
        }
        inner.pending_starts.insert(room_id.to_string());
        Some(next_seq(&mut inner, room_id))
    }

    /// Arranque forzado (todos listos): invalida timeouts anteriores.
    pub fn force_start(&self, room_id: &str) -> u64 {
        let mut inner = self.write();
        inner.pending_starts.insert(room_id.to_string());
        next_seq(&mut inner, room_id)
    }

    /// Solo el primero que reclame la generación actual arranca la sala.
    pub fn claim_start(&self, room_id: &str, seq: u64) -> bool {
        let mut inner = self.write();
        if inner.start_seq.get(room_id).copied() != Some(seq) {
            return false;
        }
        inner.pending_starts.remove(room_id);
        true
    }
}

fn next_seq(inner: &mut HubInner, room_id: &str) -> u64 {
    let seq = inner.start_seq.entry(room_id.to_string()).or_insert(0);
    *seq += 1;
    *seq
}
