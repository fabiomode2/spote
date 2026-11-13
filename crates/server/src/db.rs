//! Persistencia SQLite del servidor social.
//!
//! Una única conexión protegida por `Mutex`: el servidor atiende a cuatro
//! amigos, no hace falta un pool. Todas las consultas son cortas y síncronas;
//! los handlers nunca esperan (`await`) con el lock cogido.

use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{params, Connection, OptionalExtension, Row};
use spote_protocol::{ChatMsg, Member, Presence, QueueItem, Role, Room, RoomState, TrackRef, User};
use uuid::Uuid;

use crate::{error::AppError, service::now_ms};

/// Esquema completo. Todo idempotente: se puede ejecutar en cada arranque.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    avatar      TEXT,
    invite_code TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token      TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS friends (
    user_id    TEXT NOT NULL,
    target_id  TEXT NOT NULL,
    accepted   INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, target_id)
);

CREATE TABLE IF NOT EXISTS rooms (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    host_id    TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    is_public  INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS room_members (
    room_id   TEXT NOT NULL,
    user_id   TEXT NOT NULL,
    role      TEXT NOT NULL DEFAULT 'guest',
    ready     INTEGER NOT NULL DEFAULT 0,
    joined_at INTEGER NOT NULL,
    PRIMARY KEY (room_id, user_id)
);

CREATE TABLE IF NOT EXISTS room_states (
    room_id     TEXT PRIMARY KEY,
    track_json  TEXT,
    position_ms INTEGER NOT NULL DEFAULT 0,
    playing     INTEGER NOT NULL DEFAULT 0,
    anchor_ms   INTEGER NOT NULL DEFAULT 0,
    updated_by  TEXT,
    start_at_ms INTEGER,
    updated_at  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS queue_items (
    id         TEXT PRIMARY KEY,
    room_id    TEXT NOT NULL,
    track_json TEXT NOT NULL,
    added_by   TEXT NOT NULL,
    position   INTEGER NOT NULL,
    votes      INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chat (
    id      TEXT PRIMARY KEY,
    room_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    text    TEXT NOT NULL,
    ts      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS play_events (
    id         TEXT PRIMARY KEY,
    room_id    TEXT,
    user_id    TEXT NOT NULL,
    track_json TEXT NOT NULL,
    ts         INTEGER NOT NULL,
    ms_played  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS user_presence (
    user_id     TEXT PRIMARY KEY,
    online      INTEGER NOT NULL DEFAULT 0,
    last_seen   INTEGER NOT NULL DEFAULT 0,
    now_playing TEXT,
    playing     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_chat_room_ts ON chat(room_id, ts);
CREATE INDEX IF NOT EXISTS idx_queue_room_position ON queue_items(room_id, position);
CREATE INDEX IF NOT EXISTS idx_room_members_user ON room_members(user_id);
CREATE INDEX IF NOT EXISTS idx_play_events_room_ts ON play_events(room_id, ts);
"#;

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let conn = Connection::open(path)?;
        Self::configure(&conn, true)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    /// Solo para tests que no quieran fichero; el servidor siempre usa [`Db::open`].
    pub fn open_in_memory() -> Result<Self, AppError> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn, false)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn configure(conn: &Connection, wal: bool) -> Result<(), AppError> {
        conn.busy_timeout(Duration::from_secs(5))?;
        if wal {
            // `journal_mode` devuelve fila, por eso query_row y no execute.
            let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
            conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        }
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(())
    }

    fn migrate(&self) -> Result<(), AppError> {
        let conn = self.lock()?;
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.conn
            .lock()
            .map_err(|_| AppError::Internal("database lock poisoned".to_string()))
    }

    // ----------------------------------------------------------------- users

    /// Login idempotente por nombre: reutiliza el usuario y emite token nuevo.
    /// Devuelve `(user, token, invite_code)`.
    pub fn login(&self, name: &str, invite_code: &str) -> Result<(User, String, String), AppError> {
        let conn = self.lock()?;
        let existing = conn
            .query_row(
                "SELECT id, name, avatar FROM users WHERE name = ?1",
                [name],
                map_user,
            )
            .optional()?;

        let (user, invite) = match existing {
            Some(user) => {
                let code: String = conn.query_row(
                    "SELECT invite_code FROM users WHERE id = ?1",
                    [&user.id],
                    |row| row.get(0),
                )?;
                (user, code)
            }
            None => {
                let id = Uuid::new_v4().to_string();
                let now = now_ms();
                conn.execute(
                    "INSERT INTO users (id, name, avatar, invite_code, created_at)
                     VALUES (?1, ?2, NULL, ?3, ?4)",
                    params![id, name, invite_code, now],
                )?;
                conn.execute(
                    "INSERT INTO user_presence (user_id, online, last_seen, now_playing, playing)
                     VALUES (?1, 0, ?2, NULL, 0)
                     ON CONFLICT(user_id) DO NOTHING",
                    params![id, now],
                )?;
                (
                    User {
                        id,
                        name: name.to_string(),
                        avatar: None,
                    },
                    invite_code.to_string(),
                )
            }
        };

        let token = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO sessions (token, user_id, created_at) VALUES (?1, ?2, ?3)",
            params![token, user.id, now_ms()],
        )?;
        Ok((user, token, invite))
    }

    pub fn user_by_token(&self, token: &str) -> Result<Option<User>, AppError> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT u.id, u.name, u.avatar
                 FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token = ?1",
                [token],
                map_user,
            )
            .optional()?)
    }

    pub fn user_by_id(&self, user_id: &str) -> Result<Option<User>, AppError> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT id, name, avatar FROM users WHERE id = ?1",
                [user_id],
                map_user,
            )
            .optional()?)
    }

    pub fn list_users(&self) -> Result<Vec<User>, AppError> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT id, name, avatar FROM users ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], map_user)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Amistad mutua aceptada (sin flujo de peticiones: cuatro amigos).
    pub fn add_friend(&self, user_id: &str, target_id: &str) -> Result<User, AppError> {
        if user_id == target_id {
            return Err(AppError::BadRequest(
                "no puedes añadirte a ti mismo".to_string(),
            ));
        }
        let conn = self.lock()?;
        let target = conn
            .query_row(
                "SELECT id, name, avatar FROM users WHERE id = ?1",
                [target_id],
                map_user,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound("user not found".to_string()))?;
        let now = now_ms();
        for (a, b) in [(user_id, target_id), (target_id, user_id)] {
            conn.execute(
                "INSERT INTO friends (user_id, target_id, accepted, created_at)
                 VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(user_id, target_id) DO UPDATE SET accepted = 1",
                params![a, b, now],
            )?;
        }
        Ok(target)
    }

    // --------------------------------------------------------------- presence

    pub fn upsert_presence(&self, presence: &Presence, playing: bool) -> Result<(), AppError> {
        let now_playing = match &presence.now_playing {
            Some(track) => Some(serde_json::to_string(track)?),
            None => None,
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO user_presence (user_id, online, last_seen, now_playing, playing)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user_id) DO UPDATE SET
                 online = excluded.online,
                 last_seen = excluded.last_seen,
                 now_playing = excluded.now_playing,
                 playing = excluded.playing",
            params![
                presence.user.id,
                presence.online as i64,
                presence.last_seen,
                now_playing,
                playing as i64
            ],
        )?;
        Ok(())
    }

    pub fn load_presence(&self, user_id: &str) -> Result<Option<Presence>, AppError> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT u.id, u.name, u.avatar, p.online, p.now_playing, p.last_seen
                 FROM user_presence p JOIN users u ON u.id = p.user_id
                 WHERE p.user_id = ?1",
                [user_id],
                map_presence,
            )
            .optional()?)
    }

    /// Al arrancar no hay nadie conectado: cualquier `online` de la DB es resto
    /// de un cierre sucio.
    pub fn mark_all_offline(&self) -> Result<(), AppError> {
        let conn = self.lock()?;
        conn.execute("UPDATE user_presence SET online = 0", [])?;
        Ok(())
    }

    pub fn load_all_presences(&self) -> Result<Vec<Presence>, AppError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT u.id, u.name, u.avatar, p.online, p.now_playing, p.last_seen
             FROM user_presence p JOIN users u ON u.id = p.user_id
             ORDER BY u.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], map_presence)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ------------------------------------------------------------------ rooms

    pub fn create_room(
        &self,
        name: &str,
        is_public: bool,
        host_id: &str,
    ) -> Result<Room, AppError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        tx.execute(
            "INSERT INTO rooms (id, name, host_id, created_at, is_public)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name, host_id, now, is_public as i64],
        )?;
        tx.execute(
            "INSERT INTO room_members (room_id, user_id, role, ready, joined_at)
             VALUES (?1, ?2, 'host', 0, ?3)",
            params![id, host_id, now],
        )?;
        tx.execute(
            "INSERT INTO room_states (room_id, position_ms, playing, anchor_ms, updated_at)
             VALUES (?1, 0, 0, 0, ?2)",
            params![id, now],
        )?;
        tx.commit()?;
        room_by_id(&conn, &id)?.ok_or_else(|| AppError::Internal("room vanished".to_string()))
    }

    pub fn room(&self, room_id: &str) -> Result<Option<Room>, AppError> {
        let conn = self.lock()?;
        room_by_id(&conn, room_id)
    }

    /// Salas públicas más aquellas en las que el usuario es miembro u host.
    pub fn rooms_visible(&self, user_id: &str) -> Result<Vec<Room>, AppError> {
        let conn = self.lock()?;
        let ids: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT id FROM rooms
                 WHERE is_public = 1 OR host_id = ?1
                    OR id IN (SELECT room_id FROM room_members WHERE user_id = ?1)
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([user_id], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut rooms = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(room) = room_by_id(&conn, &id)? {
                rooms.push(room);
            }
        }
        Ok(rooms)
    }

    pub fn join_room(&self, room_id: &str, user_id: &str) -> Result<Room, AppError> {
        let conn = self.lock()?;
        let host_id: String = conn
            .query_row(
                "SELECT host_id FROM rooms WHERE id = ?1",
                [room_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound("room not found".to_string()))?;
        let role = if host_id == user_id { "host" } else { "guest" };
        conn.execute(
            "INSERT INTO room_members (room_id, user_id, role, ready, joined_at)
             VALUES (?1, ?2, ?3, 0, ?4)
             ON CONFLICT(room_id, user_id) DO NOTHING",
            params![room_id, user_id, role, now_ms()],
        )?;
        room_by_id(&conn, room_id)?.ok_or_else(|| AppError::NotFound("room not found".to_string()))
    }

    pub fn leave_room(&self, room_id: &str, user_id: &str) -> Result<bool, AppError> {
        let conn = self.lock()?;
        let removed = conn.execute(
            "DELETE FROM room_members WHERE room_id = ?1 AND user_id = ?2",
            params![room_id, user_id],
        )?;
        Ok(removed > 0)
    }

    pub fn is_member(&self, room_id: &str, user_id: &str) -> Result<bool, AppError> {
        let conn = self.lock()?;
        let found = conn
            .query_row(
                "SELECT 1 FROM room_members WHERE room_id = ?1 AND user_id = ?2",
                params![room_id, user_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(found)
    }

    pub fn members(&self, room_id: &str) -> Result<Vec<Member>, AppError> {
        let conn = self.lock()?;
        members_for(&conn, room_id)
    }

    pub fn set_ready(&self, room_id: &str, user_id: &str, ready: bool) -> Result<(), AppError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE room_members SET ready = ?1 WHERE room_id = ?2 AND user_id = ?3",
            params![ready as i64, room_id, user_id],
        )?;
        Ok(())
    }

    pub fn reset_ready(&self, room_id: &str) -> Result<(), AppError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE room_members SET ready = 0 WHERE room_id = ?1",
            [room_id],
        )?;
        Ok(())
    }

    pub fn save_state(&self, state: &RoomState) -> Result<(), AppError> {
        let track_json = match &state.track {
            Some(track) => Some(serde_json::to_string(track)?),
            None => None,
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO room_states
                 (room_id, track_json, position_ms, playing, anchor_ms, updated_by, start_at_ms, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(room_id) DO UPDATE SET
                 track_json = excluded.track_json,
                 position_ms = excluded.position_ms,
                 playing = excluded.playing,
                 anchor_ms = excluded.anchor_ms,
                 updated_by = excluded.updated_by,
                 start_at_ms = excluded.start_at_ms,
                 updated_at = excluded.updated_at",
            params![
                state.room_id,
                track_json,
                state.position_ms as i64,
                state.playing as i64,
                state.anchor_ms,
                state.updated_by,
                state.start_at_ms,
                now_ms()
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------ queue

    pub fn queue_add(
        &self,
        room_id: &str,
        track: &TrackRef,
        added_by: &str,
    ) -> Result<QueueItem, AppError> {
        let conn = self.lock()?;
        let position: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM queue_items WHERE room_id = ?1",
            [room_id],
            |row| row.get(0),
        )?;
        let item = QueueItem {
            id: Uuid::new_v4().to_string(),
            track: track.clone(),
            added_by: added_by.to_string(),
            position,
            votes: 0,
        };
        conn.execute(
            "INSERT INTO queue_items (id, room_id, track_json, added_by, position, votes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                item.id,
                room_id,
                serde_json::to_string(&item.track)?,
                item.added_by,
                item.position,
                item.votes,
                now_ms()
            ],
        )?;
        Ok(item)
    }

    pub fn queue_list(&self, room_id: &str) -> Result<Vec<QueueItem>, AppError> {
        let conn = self.lock()?;
        queue_for(&conn, room_id)
    }

    /// Quita un item y renumera el resto para no dejar huecos.
    pub fn queue_remove(&self, room_id: &str, item_id: &str) -> Result<bool, AppError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM queue_items WHERE room_id = ?1 AND id = ?2",
            params![room_id, item_id],
        )?;
        if removed > 0 {
            renumber_queue(&tx, room_id)?;
        }
        tx.commit()?;
        Ok(removed > 0)
    }

    /// Saca el primer item de la cola (por posición) y renumera.
    pub fn queue_pop_front(&self, room_id: &str) -> Result<Option<QueueItem>, AppError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let item = tx
            .query_row(
                "SELECT id, track_json, added_by, position, votes
                 FROM queue_items WHERE room_id = ?1
                 ORDER BY position ASC, created_at ASC LIMIT 1",
                [room_id],
                map_queue_item,
            )
            .optional()?;
        let Some(item) = item else {
            tx.commit()?;
            return Ok(None);
        };
        tx.execute("DELETE FROM queue_items WHERE id = ?1", [&item.id])?;
        renumber_queue(&tx, room_id)?;
        tx.commit()?;
        Ok(Some(item))
    }

    // ------------------------------------------------------------------- chat

    pub fn chat_add(&self, room_id: &str, user: &User, text: &str) -> Result<ChatMsg, AppError> {
        let msg = ChatMsg {
            id: Uuid::new_v4().to_string(),
            room_id: room_id.to_string(),
            user_id: user.id.clone(),
            user_name: user.name.clone(),
            text: text.to_string(),
            ts: now_ms(),
        };
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO chat (id, room_id, user_id, text, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![msg.id, msg.room_id, msg.user_id, msg.text, msg.ts],
        )?;
        Ok(msg)
    }

    /// Historial en orden cronológico (más antiguo primero).
    pub fn chat_history(&self, room_id: &str, limit: u32) -> Result<Vec<ChatMsg>, AppError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.room_id, c.user_id, u.name, c.text, c.ts
             FROM chat c JOIN users u ON u.id = c.user_id
             WHERE c.room_id = ?1
             ORDER BY c.ts DESC, c.rowid DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![room_id, limit.clamp(1, 200) as i64], |row| {
            Ok(ChatMsg {
                id: row.get(0)?,
                room_id: row.get(1)?,
                user_id: row.get(2)?,
                user_name: row.get(3)?,
                text: row.get(4)?,
                ts: row.get(5)?,
            })
        })?;
        let mut messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        messages.reverse();
        Ok(messages)
    }

    // ------------------------------------------------------------- play events

    pub fn log_play_event(
        &self,
        room_id: &str,
        user_id: &str,
        track: &TrackRef,
        ms_played: u64,
    ) -> Result<(), AppError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO play_events (id, room_id, user_id, track_json, ts, ms_played)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                Uuid::new_v4().to_string(),
                room_id,
                user_id,
                serde_json::to_string(track)?,
                now_ms(),
                ms_played as i64
            ],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers libres: reciben `&Connection` para poder usarse dentro de transacciones
// sin volver a coger el mutex (deadlock).
// ---------------------------------------------------------------------------

fn map_user(row: &Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        name: row.get(1)?,
        avatar: row.get(2)?,
    })
}

fn map_presence(row: &Row<'_>) -> rusqlite::Result<Presence> {
    let now_playing: Option<String> = row.get(4)?;
    Ok(Presence {
        user: User {
            id: row.get(0)?,
            name: row.get(1)?,
            avatar: row.get(2)?,
        },
        online: row.get::<_, i64>(3)? != 0,
        last_seen: row.get(5)?,
        now_playing: parse_track(now_playing),
    })
}

fn parse_track(json: Option<String>) -> Option<TrackRef> {
    let raw = json?;
    match serde_json::from_str::<TrackRef>(&raw) {
        Ok(track) => Some(track),
        Err(error) => {
            tracing::warn!(%error, "track_json corrupto, se ignora");
            None
        }
    }
}

fn map_queue_item(row: &Row<'_>) -> rusqlite::Result<QueueItem> {
    let track_json: String = row.get(1)?;
    Ok(QueueItem {
        id: row.get(0)?,
        track: serde_json::from_str(&track_json).unwrap_or_default(),
        added_by: row.get(2)?,
        position: row.get(3)?,
        votes: row.get(4)?,
    })
}

fn room_by_id(conn: &Connection, room_id: &str) -> Result<Option<Room>, AppError> {
    let head = conn
        .query_row(
            "SELECT id, name, host_id, is_public FROM rooms WHERE id = ?1",
            [room_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            },
        )
        .optional()?;
    let Some((id, name, host_id, is_public)) = head else {
        return Ok(None);
    };
    Ok(Some(Room {
        members: members_for(conn, &id)?,
        state: state_for(conn, &id)?,
        id,
        name,
        host_id,
        is_public,
    }))
}

fn members_for(conn: &Connection, room_id: &str) -> Result<Vec<Member>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.name, u.avatar, m.role, m.ready
         FROM room_members m JOIN users u ON u.id = m.user_id
         WHERE m.room_id = ?1
         ORDER BY m.joined_at ASC",
    )?;
    let rows = stmt.query_map([room_id], |row| {
        let role: String = row.get(3)?;
        Ok(Member {
            user: User {
                id: row.get(0)?,
                name: row.get(1)?,
                avatar: row.get(2)?,
            },
            role: if role == "host" {
                Role::Host
            } else {
                Role::Guest
            },
            ready: row.get::<_, i64>(4)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn state_for(conn: &Connection, room_id: &str) -> Result<RoomState, AppError> {
    let row = conn
        .query_row(
            "SELECT track_json, position_ms, playing, anchor_ms, updated_by, start_at_ms
             FROM room_states WHERE room_id = ?1",
            [room_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)? != 0,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((track_json, position_ms, playing, anchor_ms, updated_by, start_at_ms)) = row else {
        return Ok(RoomState {
            room_id: room_id.to_string(),
            ..Default::default()
        });
    };
    Ok(RoomState {
        room_id: room_id.to_string(),
        track: parse_track(track_json),
        playing,
        position_ms: position_ms.max(0) as u64,
        anchor_ms,
        updated_by,
        start_at_ms,
    })
}

fn queue_for(conn: &Connection, room_id: &str) -> Result<Vec<QueueItem>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, track_json, added_by, position, votes
         FROM queue_items WHERE room_id = ?1
         ORDER BY position ASC, created_at ASC",
    )?;
    let rows = stmt.query_map([room_id], map_queue_item)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn renumber_queue(conn: &Connection, room_id: &str) -> Result<(), AppError> {
    let ids: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id FROM queue_items WHERE room_id = ?1
             ORDER BY position ASC, created_at ASC",
        )?;
        let rows = stmt.query_map([room_id], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE queue_items SET position = ?1 WHERE id = ?2",
            params![index as i64, id],
        )?;
    }
    Ok(())
}
