//! Librería local: SQLite con FTS5. Es la única fuente de verdad del cliente.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};

use spote_protocol::TrackRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStatus {
    Pending,
    Downloading,
    Ready,
    Failed,
    Missing,
}

impl SourceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Downloading => "downloading",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Missing => "missing",
        }
    }

    pub fn from_str(text: &str) -> Self {
        match text {
            "downloading" => Self::Downloading,
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            "missing" => Self::Missing,
            _ => Self::Pending,
        }
    }
}

/// Fila de la librería ya unida con su fuente local.
#[derive(Debug, Clone)]
pub struct TrackRow {
    pub id: i64,
    pub track: TrackRef,
    pub path: Option<PathBuf>,
    pub status: SourceStatus,
    pub provider: Option<String>,
    pub favorite: bool,
    pub play_count: u32,
    pub art_path: Option<PathBuf>,
    /// Alta en la biblioteca (ms desde epoch), para la columna «añadida».
    pub added_at: i64,
    /// Artistas acreditados, ya separadas las colaboraciones. Vacío mientras
    /// la fila no se haya enriquecido.
    pub artists: Vec<String>,
    /// Artista del álbum: agrupa las pistas aunque cada una acredite a un
    /// colaborador distinto.
    pub album_artist: Option<String>,
}

impl TrackRow {
    pub fn playable(&self) -> bool {
        self.path.as_ref().is_some_and(|p| p.exists())
    }
}

/// Álbum agrupado de la biblioteca.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumRow {
    pub album: String,
    pub artist: String,
    pub track_count: i64,
    pub ready_count: i64,
    pub duration_ms: u64,
    pub art_path: Option<PathBuf>,
    pub art_url: Option<String>,
    pub favorite_count: u32,
}

impl AlbumRow {
    pub fn label(&self) -> String {
        format!("{} · {}", self.artist, self.album)
    }

    pub fn playable(&self) -> bool {
        self.ready_count > 0
    }
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub description: String,
    /// Clave de la pista cuya portada hace de portada automática.
    pub cover_key: Option<String>,
    /// Portada remota (playlist importada).
    pub cover_url: Option<String>,
}

/// Fila de letra cacheada. `source`/`aligned` se restauran para no repetir
/// el auto-ajuste sobre una letra que ya vino alineada por ASR.
#[derive(Debug, Clone)]
pub struct LyricsRow {
    pub lrc: String,
    pub synced: bool,
    pub source: String,
    pub offset_ms: i64,
    pub aligned: bool,
    /// Versión del alineador ASR que produjo estos tiempos. Si es vieja, la
    /// fila se vuelve a afinar cuando hay audio local: un resultado malo no
    /// debe quedarse pegado para siempre.
    pub align_version: i64,
    /// Versión de la política de fuentes que creó esta caché. Las filas
    /// antiguas se revalidan una vez; las nuevas no se vuelven a pedir en cada
    /// sesión solo por su etiqueta.
    pub policy_version: i64,
    /// Los tiempos vienen de captions del vídeo exacto; el texto es real.
    pub captions_timed: bool,
}

/// Se sube cuando cambia el alineado (scoring, relecturas, reparto). Las filas
/// guardadas con una versión anterior se re-alinean al reproducirlas.
pub const LYRICS_ALIGN_VERSION: i64 = 3;

/// Versión de la política de fuentes de letras. Al cambiarla se invalidan las
/// cachés antiguas (excepto las locales/manuales) para que nadie siga viendo
/// una letra inventada por captions/ASR de una versión anterior.
pub const LYRICS_POLICY_VERSION: i64 = 2;

/// Resumen de escucha para el Wrapped.
#[derive(Debug, Clone, Default)]
pub struct WrappedStats {
    pub all_time: bool,
    pub year: i64,
    pub plays: u64,
    pub ms_played: u64,
    pub distinct_tracks: u64,
    pub distinct_artists: u64,
    pub top_tracks: Vec<WrappedTrack>,
    pub top_artists: Vec<WrappedArtist>,
    pub top_albums: Vec<WrappedAlbum>,
    /// Milisegundos escuchados por mes (enero..diciembre) del periodo.
    pub by_month: [u64; 12],
}

#[derive(Debug, Clone)]
pub struct WrappedTrack {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub ms_played: u64,
    pub plays: u64,
    pub art_path: Option<PathBuf>,
    pub art_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WrappedArtist {
    pub artist: String,
    pub ms_played: u64,
    pub plays: u64,
}

#[derive(Debug, Clone)]
pub struct WrappedAlbum {
    pub album: String,
    pub artist: String,
    pub ms_played: u64,
    pub plays: u64,
    pub art_url: Option<String>,
}

pub struct Library {
    conn: Connection,
    path: PathBuf,
    /// La UI pregunta por estos totales varias veces por frame (barra, panel,
    /// leyenda). SQLite es rápido, pero no gratis a 120 fps.
    count_cache: Cell<Option<(Instant, i64)>>,
    ready_cache: Cell<Option<(Instant, i64)>>,
    playlists_cache: RefCell<Option<(Instant, Vec<Playlist>)>>,
}

impl Library {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)
            .with_context(|| format!("no se pudo abrir la librería {}", path.display()))?;
        let mut lib = Self {
            conn,
            path: path.to_path_buf(),
            count_cache: Cell::new(None),
            ready_cache: Cell::new(None),
            playlists_cache: RefCell::new(None),
        };
        lib.configure()?;
        lib.migrate()?;
        // Limpiezas silenciosas de datos viejos. Si fallan, la biblioteca
        // sigue funcionando igual. `merge_duplicates` NO se ejecuta aquí:
        // dos versiones de estudio/directo con duración parecida podían
        // fusionarse y perder fuente/letra/historial. Queda como operación
        // explícita.
        lib.clear_shared_art().ok();
        lib.clear_imported_albums().ok();
        Ok(lib)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut lib = Self {
            conn,
            path: PathBuf::from(":memory:"),
            count_cache: Cell::new(None),
            ready_cache: Cell::new(None),
            playlists_cache: RefCell::new(None),
        };
        lib.configure()?;
        lib.migrate()?;
        Ok(lib)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn configure(&mut self) -> Result<()> {
        self.conn.pragma_update(None, "journal_mode", "WAL").ok();
        self.conn.pragma_update(None, "synchronous", "NORMAL")?;
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        self.conn.busy_timeout(std::time::Duration::from_secs(3))?;
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS track(
              id          INTEGER PRIMARY KEY,
              key         TEXT NOT NULL UNIQUE,
              title       TEXT NOT NULL,
              artist      TEXT NOT NULL,
              album       TEXT,
              duration_ms INTEGER NOT NULL DEFAULT 0,
              art_url     TEXT,
              art_path    TEXT,
              added_at    INTEGER NOT NULL,
              favorite    INTEGER NOT NULL DEFAULT 0,
              play_count  INTEGER NOT NULL DEFAULT 0,
              last_played INTEGER,
              hidden      INTEGER NOT NULL DEFAULT 0,
              artists_json    TEXT,
              artists_version INTEGER NOT NULL DEFAULT 0,
              album_artist    TEXT
            );
            CREATE TABLE IF NOT EXISTS source(
              id          INTEGER PRIMARY KEY,
              track_id    INTEGER NOT NULL REFERENCES track(id) ON DELETE CASCADE,
              provider    TEXT NOT NULL,
              provider_id TEXT,
              url         TEXT,
              ext         TEXT,
              bitrate     INTEGER,
              status      TEXT NOT NULL DEFAULT 'pending',
              path        TEXT,
              size        INTEGER,
              UNIQUE(track_id, provider)
            );
            CREATE TABLE IF NOT EXISTS lyrics(
              track_id   INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
              lrc        TEXT NOT NULL,
              synced     INTEGER NOT NULL,
              source     TEXT NOT NULL,
              offset_ms  INTEGER NOT NULL DEFAULT 0,
              fetched_at INTEGER NOT NULL,
              policy_version INTEGER NOT NULL DEFAULT 0,
              captions_timed INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS playlist(
              id         INTEGER PRIMARY KEY,
              name       TEXT NOT NULL,
              created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlist_item(
              playlist_id INTEGER NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
              track_id    INTEGER NOT NULL REFERENCES track(id) ON DELETE CASCADE,
              position    INTEGER NOT NULL,
              PRIMARY KEY(playlist_id, track_id)
            );
            CREATE TABLE IF NOT EXISTS play_event(
              id         INTEGER PRIMARY KEY,
              track_id   INTEGER NOT NULL REFERENCES track(id) ON DELETE CASCADE,
              ts         INTEGER NOT NULL,
              ms_played  INTEGER NOT NULL,
              completed  INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS setting(
              key   TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS track_trim(
              track_id INTEGER PRIMARY KEY REFERENCES track(id) ON DELETE CASCADE,
              start_ms INTEGER NOT NULL DEFAULT 0,
              end_ms   INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_track_artist ON track(artist);
            CREATE INDEX IF NOT EXISTS idx_track_added ON track(added_at DESC);
            CREATE INDEX IF NOT EXISTS idx_source_status ON source(status);
            CREATE INDEX IF NOT EXISTS idx_play_event_ts ON play_event(ts);
            CREATE INDEX IF NOT EXISTS idx_play_event_track ON play_event(track_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS track_fts USING fts5(
              title, artist, album,
              content='track', content_rowid='id',
              tokenize='unicode61 remove_diacritics 2'
            );
            CREATE TRIGGER IF NOT EXISTS track_ai AFTER INSERT ON track BEGIN
              INSERT INTO track_fts(rowid, title, artist, album)
              VALUES (new.id, new.title, new.artist, coalesce(new.album, ''));
            END;
            CREATE TRIGGER IF NOT EXISTS track_ad AFTER DELETE ON track BEGIN
              INSERT INTO track_fts(track_fts, rowid, title, artist, album)
              VALUES ('delete', old.id, old.title, old.artist, coalesce(old.album, ''));
            END;
            CREATE TRIGGER IF NOT EXISTS track_au AFTER UPDATE ON track BEGIN
              INSERT INTO track_fts(track_fts, rowid, title, artist, album)
              VALUES ('delete', old.id, old.title, old.artist, coalesce(old.album, ''));
              INSERT INTO track_fts(rowid, title, artist, album)
              VALUES (new.id, new.title, new.artist, coalesce(new.album, ''));
            END;
            "#,
        )?;
        self.migrate_playlists()?;
        self.migrate_track_columns()?;
        self.migrate_lyrics()?;
        self.migrate_lyrics_policy()?;
        Ok(())
    }

    /// Columnas nuevas de track en bases ya existentes.
    fn migrate_track_columns(&mut self) -> Result<()> {
        let mut existing = std::collections::HashSet::new();
        {
            let mut stmt = self.conn.prepare("PRAGMA table_info(track)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            for row in rows {
                existing.insert(row?);
            }
        }
        if !existing.contains("hidden") {
            self.conn.execute(
                "ALTER TABLE track ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        // Multiarista: la lista de artistas se cachea en JSON. NULL significa
        // «sin enriquecer»; la versión permite invalidar cálculos antiguos.
        for (column, ddl) in [
            (
                "artists_json",
                "ALTER TABLE track ADD COLUMN artists_json TEXT",
            ),
            (
                "artists_version",
                "ALTER TABLE track ADD COLUMN artists_version INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "album_artist",
                "ALTER TABLE track ADD COLUMN album_artist TEXT",
            ),
        ] {
            if !existing.contains(column) {
                self.conn.execute(ddl, [])?;
            }
        }
        Ok(())
    }

    /// `aligned` llega después: una letra ya ajustada con ASR no debe volver a
    /// pasar por el estimador de offset al recargar desde caché.
    fn migrate_lyrics(&mut self) -> Result<()> {
        let mut existing = std::collections::HashSet::new();
        {
            let mut stmt = self.conn.prepare("PRAGMA table_info(lyrics)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            for row in rows {
                existing.insert(row?);
            }
        }
        if !existing.contains("aligned") {
            self.conn.execute(
                "ALTER TABLE lyrics ADD COLUMN aligned INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !existing.contains("align_version") {
            self.conn.execute(
                "ALTER TABLE lyrics ADD COLUMN align_version INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !existing.contains("policy_version") {
            self.conn.execute(
                "ALTER TABLE lyrics ADD COLUMN policy_version INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !existing.contains("captions_timed") {
            self.conn.execute(
                "ALTER TABLE lyrics ADD COLUMN captions_timed INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(())
    }

    /// Marca como actuales las cachés de fuentes fiables y deja las ambiguas
    /// (`plain`, `captions`, `approx`) para que se revaliden al reproducirlas.
    /// No borra filas: si no hay red, la letra sigue visible mientras se
    /// intenta mejorar.
    fn migrate_lyrics_policy(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE lyrics SET policy_version = ?1
             WHERE policy_version < ?1
               AND lower(source) IN ('local', 'manual', 'genius', 'lrclib', 'lrcmux', 'asr')",
            params![LYRICS_POLICY_VERSION],
        )?;
        tx.execute(
            "INSERT INTO setting(key, value) VALUES('lyrics_policy_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![LYRICS_POLICY_VERSION.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Columnas nuevas de playlist en bases ya existentes. SQLite no tiene
    /// `ADD COLUMN IF NOT EXISTS`, así que se mira `table_info` primero.
    fn migrate_playlists(&mut self) -> Result<()> {
        let mut existing = std::collections::HashSet::new();
        {
            let mut stmt = self.conn.prepare("PRAGMA table_info(playlist)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            for row in rows {
                existing.insert(row?);
            }
        }
        for (column, ddl) in [
            (
                "description",
                "ALTER TABLE playlist ADD COLUMN description TEXT NOT NULL DEFAULT ''",
            ),
            (
                "cover_key",
                "ALTER TABLE playlist ADD COLUMN cover_key TEXT",
            ),
            (
                "cover_url",
                "ALTER TABLE playlist ADD COLUMN cover_url TEXT",
            ),
        ] {
            if !existing.contains(column) {
                self.conn.execute(ddl, [])?;
            }
        }
        Ok(())
    }

    /// Inserta o actualiza una canción canónica. Devuelve su id local.
    pub fn upsert_track(&mut self, track: &TrackRef) -> Result<i64> {
        self.count_cache.set(None);
        let now = now_ms();
        self.conn.execute(
            r#"
            INSERT INTO track(key, title, artist, album, duration_ms, art_url, added_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(key) DO UPDATE SET
              title = excluded.title,
              artist = excluded.artist,
              album = coalesce(excluded.album, track.album),
              duration_ms = CASE WHEN excluded.duration_ms > 0
                                 THEN excluded.duration_ms ELSE track.duration_ms END,
              art_url = coalesce(excluded.art_url, track.art_url),
              hidden = 0
            "#,
            params![
                track.key,
                track.title,
                track.artist,
                track.album,
                track.duration_ms as i64,
                track.art_url,
                now
            ],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM track WHERE key = ?1",
            params![track.key],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Guarda los artistas ya separados y la versión del cálculo. UPDATE
    /// directo a propósito: `upsert_track` toca columnas de metadatos y no
    /// debe pisar un enriquecimiento que ya estaba hecho.
    pub fn update_artists(&self, key: &str, artists: &[String], version: i64) -> Result<()> {
        let encoded = encode_artists_json(artists);
        self.conn.execute(
            "UPDATE track SET artists_json = ?1, artists_version = ?2 WHERE key = ?3",
            params![encoded, version, key],
        )?;
        Ok(())
    }

    /// Artista del álbum, independiente del crédito de cada pista.
    pub fn set_album_artist(&self, key: &str, album_artist: &str) -> Result<()> {
        let value = album_artist.trim();
        let value = (!value.is_empty()).then_some(value);
        self.conn.execute(
            "UPDATE track SET album_artist = ?1 WHERE key = ?2",
            params![value, key],
        )?;
        Ok(())
    }

    /// Pistas pendientes de separar colaboraciones: nunca enriquecidas o con
    /// una versión antigua del cálculo.
    pub fn tracks_without_artists(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK}
             WHERE (t.artists_json IS NULL OR t.artists_version = 0) AND t.hidden = 0
             ORDER BY t.added_at DESC, t.id DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Registra una fuente (fichero local o remota) para una canción.
    pub fn set_source(
        &mut self,
        track_id: i64,
        provider: &str,
        provider_id: Option<&str>,
        path: Option<&Path>,
        status: SourceStatus,
        size: Option<u64>,
    ) -> Result<()> {
        self.ready_cache.set(None);
        let ext = path.and_then(|p| p.extension()).and_then(|e| e.to_str());
        self.conn.execute(
            r#"
            INSERT INTO source(track_id, provider, provider_id, status, path, size, ext)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(track_id, provider) DO UPDATE SET
              provider_id = coalesce(excluded.provider_id, source.provider_id),
              status = excluded.status,
              path = coalesce(excluded.path, source.path),
              size = coalesce(excluded.size, source.size),
              ext = coalesce(excluded.ext, source.ext)
            "#,
            params![
                track_id,
                provider,
                provider_id,
                status.as_str(),
                path.map(|p| p.to_string_lossy().to_string()),
                size.map(|s| s as i64),
                ext
            ],
        )?;
        Ok(())
    }

    /// Proveedor y id de la mejor fuente remota de una pista (para descargar
    /// pistas importadas que aún no tienen fichero).
    pub fn source_hint(&self, key: &str) -> Option<(String, String)> {
        self.conn
            .query_row(
                r#"SELECT s.provider, s.provider_id FROM source s
                   JOIN track t ON t.id = s.track_id
                   WHERE t.key = ?1 AND s.provider_id IS NOT NULL
                   ORDER BY (s.status = 'ready') DESC LIMIT 1"#,
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_status(
        &mut self,
        track_id: i64,
        provider: &str,
        status: SourceStatus,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE source SET status = ?1 WHERE track_id = ?2 AND provider = ?3",
            params![status.as_str(), track_id, provider],
        )?;
        // El contador de descargadas se cachea 250 ms para pintarlo cada frame.
        self.ready_cache.set(None);
        Ok(())
    }

    pub fn track_by_key(&self, key: &str) -> Result<Option<TrackRow>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("{SELECT_TRACK} WHERE t.key = ?1"))?;
        let row = stmt.query_row(params![key], row_to_track).optional()?;
        Ok(row)
    }

    /// Busca una pista local por artista/título tolerando diferencias de
    /// duración (MPRIS y el fichero no siempre coinciden al segundo). Sirve
    /// para reutilizar la clave y la portada locales cuando suena en Spotify.
    pub fn find_by_meta(
        &self,
        artist: &str,
        title: &str,
        duration_ms: u64,
    ) -> Result<Option<TrackRow>> {
        self.find_by_meta_within(artist, title, duration_ms, 8_000)
    }

    /// Igual que [`find_by_meta`](Self::find_by_meta) con una tolerancia de
    /// duración explícita. `find_by_meta` es para portada/letras (8 s); un
    /// reemplazo de reproducción va más justo para no cambiar una versión por
    /// otra (directo, remix) con el mismo título.
    fn find_by_meta_within(
        &self,
        artist: &str,
        title: &str,
        duration_ms: u64,
        tolerance_ms: u64,
    ) -> Result<Option<TrackRow>> {
        let artist = artist.trim().to_lowercase();
        let title = title.trim().to_lowercase();
        if title.is_empty() {
            return Ok(None);
        }
        let mut stmt = self.conn.prepare_cached(&format!(
            "{SELECT_TRACK}
             WHERE lower(t.title) = ?1
               AND lower(t.artist) = ?2
               AND t.hidden = 0
               AND (?3 = 0 OR t.duration_ms = 0 OR abs(t.duration_ms - ?3) < ?4)
             ORDER BY (s.status = 'ready') DESC, abs(t.duration_ms - ?3)
             LIMIT 1"
        ))?;
        let row = stmt
            .query_row(
                params![title, artist, duration_ms as i64, tolerance_ms as i64],
                row_to_track,
            )
            .optional()?;
        Ok(row)
    }

    /// Clave ya existente para la misma canción, aunque venga con otro
    /// proveedor (ISRC/Spotify) distinto. Evita que importar una lista cree
    /// una segunda fila de algo que ya está descargado en la biblioteca.
    pub fn canonical_key(&self, track: &TrackRef) -> String {
        self.find_by_meta_within(&track.artist, &track.title, track.duration_ms, 2_000)
            .ok()
            .flatten()
            .map(|row| row.track.key)
            .unwrap_or_else(|| track.key.clone())
    }

    /// Fusión explícita de duplicados. Cuenta las parejas que cumplen el
    /// mismo criterio antes de tocar nada (para `spote dedupe --dry-run`).
    pub fn duplicate_count(&self) -> Result<i64> {
        self.conn.query_row(
            r#"SELECT count(*)
               FROM track a JOIN track b ON a.id < b.id
               WHERE lower(trim(a.title)) = lower(trim(b.title))
                 AND lower(trim(a.artist)) = lower(trim(b.artist))
                 AND a.duration_ms > 0 AND b.duration_ms > 0
                 AND abs(a.duration_ms - b.duration_ms) <= 2000
                 AND a.hidden = 0 AND b.hidden = 0
                 AND (EXISTS (SELECT 1 FROM source s WHERE s.track_id = a.id AND s.status = 'ready')
                      OR EXISTS (SELECT 1 FROM source s WHERE s.track_id = b.id AND s.status = 'ready'))"#,
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    /// Copia consistente de la base para antes de una operación destructiva.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        let path = path.to_string_lossy();
        self.conn.execute("VACUUM INTO ?1", params![path])?;
        Ok(())
    }

    /// Fusiona la misma canción metida dos veces con claves distintas: lo
    /// normal es una fila local y otra creada al importar una lista con
    /// ISRC/Spotify. Conserva la que tiene fichero y mueve a ella fuentes,
    /// listas, letra e historial. No toca parejas sin ningún audio listo.
    pub fn merge_duplicates(&mut self) -> Result<usize> {
        let pairs: Vec<(i64, i64, i64, i64)> = {
            let mut stmt = self.conn.prepare(
                r#"SELECT a.id, b.id,
                     (SELECT count(*) FROM source s WHERE s.track_id = a.id AND s.status = 'ready'),
                     (SELECT count(*) FROM source s WHERE s.track_id = b.id AND s.status = 'ready')
                   FROM track a
                   JOIN track b ON a.id < b.id
                   WHERE lower(trim(a.title)) = lower(trim(b.title))
                     AND lower(trim(a.artist)) = lower(trim(b.artist))
                     AND a.duration_ms > 0 AND b.duration_ms > 0
                     AND a.hidden = 0 AND b.hidden = 0
                     AND abs(a.duration_ms - b.duration_ms) <= 2000
                     AND (EXISTS (SELECT 1 FROM source s WHERE s.track_id = a.id AND s.status = 'ready')
                          OR EXISTS (SELECT 1 FROM source s WHERE s.track_id = b.id AND s.status = 'ready'))"#,
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut merged = 0usize;
        for (a, b, ready_a, ready_b) in pairs {
            // La que tenga audio manda; si ambas lo tienen, se queda la que
            // esté en más listas (las entradas repetidas se colapsan solas).
            let keep = match (ready_a > 0, ready_b > 0) {
                (true, false) => a,
                (false, true) => b,
                (true, true) => {
                    let lists_a: i64 = self.conn.query_row(
                        "SELECT count(*) FROM playlist_item WHERE track_id = ?1",
                        params![a],
                        |row| row.get(0),
                    )?;
                    let lists_b: i64 = self.conn.query_row(
                        "SELECT count(*) FROM playlist_item WHERE track_id = ?1",
                        params![b],
                        |row| row.get(0),
                    )?;
                    if lists_b > lists_a {
                        b
                    } else {
                        a
                    }
                }
                (false, false) => continue,
            };
            let drop = if keep == a { b } else { a };
            // Cualquiera de las dos pudo fusionarse ya en otra pasada.
            let alive: i64 = self.conn.query_row(
                "SELECT count(*) FROM track WHERE id = ?1",
                params![drop],
                |row| row.get(0),
            )?;
            let keep_alive: i64 = self.conn.query_row(
                "SELECT count(*) FROM track WHERE id = ?1",
                params![keep],
                |row| row.get(0),
            )?;
            if alive == 0 || keep_alive == 0 {
                continue;
            }
            self.merge_track(keep, drop)?;
            merged += 1;
        }
        if merged > 0 {
            self.count_cache.set(None);
            self.ready_cache.set(None);
            self.playlists_cache.replace(None);
        }
        Ok(merged)
    }

    /// Quita portadas compartidas por pistas de álbumes distintos. Es el
    /// rastro de las listas importadas, que antes copiaban la carátula de la
    /// lista a todas sus canciones. Así cada fila vuelve a buscar la suya.
    pub fn clear_shared_art(&mut self) -> Result<usize> {
        let removed = self.conn.execute(
            r#"UPDATE track SET art_url = NULL
               WHERE art_url IS NOT NULL
                 AND EXISTS (
                   SELECT 1 FROM track other
                   WHERE other.art_url = track.art_url
                     AND coalesce(other.album, '') <> coalesce(track.album, '')
                 )"#,
            [],
        )?;
        Ok(removed)
    }

    /// Las listas importadas antiguas ponían el nombre de la lista como
    /// álbum de todas sus pistas (y su carátula como portada). Eso hacía que
    /// pareciesen un solo álbum. Se limpia para que el backfill busque los
    /// datos reales de cada tema.
    pub fn clear_imported_albums(&mut self) -> Result<usize> {
        self.playlists_cache.replace(None);
        let lists = self.playlists()?;
        let mut cleared = 0usize;
        for list in lists {
            cleared += self.conn.execute(
                r#"UPDATE track SET
                     album = NULL,
                     art_url = CASE
                       WHEN ?3 IS NOT NULL AND art_url = ?3 THEN NULL
                       ELSE art_url
                     END
                   WHERE id IN (
                     SELECT track_id FROM playlist_item WHERE playlist_id = ?1
                   )
                     AND lower(trim(coalesce(album, ''))) = lower(trim(?2))
                     AND EXISTS (
                       SELECT 1 FROM source s
                       WHERE s.track_id = track.id
                         AND s.provider IN ('spotify', 'deezer')
                         AND s.provider_id IS NOT NULL
                     )"#,
                params![list.id, list.name, list.cover_url],
            )?;
        }
        if cleared > 0 {
            self.count_cache.set(None);
            self.ready_cache.set(None);
            self.playlists_cache.replace(None);
        }
        Ok(cleared)
    }

    fn merge_track(&mut self, keep: i64, drop: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO source
               (track_id, provider, provider_id, url, ext, bitrate, status, path, size)
             SELECT ?1, provider, provider_id, url, ext, bitrate, status, path, size
             FROM source WHERE track_id = ?2",
            params![keep, drop],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO playlist_item (playlist_id, track_id, position)
             SELECT playlist_id, ?1, position FROM playlist_item WHERE track_id = ?2",
            params![keep, drop],
        )?;
        tx.execute(
            "UPDATE play_event SET track_id = ?1 WHERE track_id = ?2",
            params![keep, drop],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO lyrics
               (track_id, lrc, synced, source, offset_ms, fetched_at, aligned,
                align_version, policy_version, captions_timed)
             SELECT ?1, lrc, synced, source, offset_ms, fetched_at, aligned,
                    align_version, policy_version, captions_timed
             FROM lyrics WHERE track_id = ?2",
            params![keep, drop],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO track_trim (track_id, start_ms, end_ms)
             SELECT ?1, start_ms, end_ms FROM track_trim WHERE track_id = ?2",
            params![keep, drop],
        )?;
        // Unión de artistas de ambas filas. La versión más alta gana y el
        // álbum se queda con el primer `album_artist` no vacío.
        let (keep_json, keep_version, keep_album_artist): (Option<String>, i64, Option<String>) =
            tx.query_row(
                "SELECT artists_json, artists_version, album_artist FROM track WHERE id = ?1",
                params![keep],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let (drop_json, drop_version, drop_album_artist): (Option<String>, i64, Option<String>) =
            tx.query_row(
                "SELECT artists_json, artists_version, album_artist FROM track WHERE id = ?1",
                params![drop],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let mut artists = parse_artists_json(keep_json.as_deref());
        for artist in parse_artists_json(drop_json.as_deref()) {
            push_unique_artist(&mut artists, &artist);
        }
        let album_artist = keep_album_artist
            .filter(|value| !value.trim().is_empty())
            .or_else(|| drop_album_artist.filter(|value| !value.trim().is_empty()));
        tx.execute(
            "UPDATE track SET
               album = coalesce(album, (SELECT album FROM track WHERE id = ?2)),
               art_path = coalesce(art_path, (SELECT art_path FROM track WHERE id = ?2)),
               art_url = coalesce(art_url, (SELECT art_url FROM track WHERE id = ?2)),
               favorite = max(favorite, (SELECT favorite FROM track WHERE id = ?2)),
               play_count = play_count + (SELECT play_count FROM track WHERE id = ?2),
               last_played = nullif(max(
                 coalesce(last_played, 0),
                 coalesce((SELECT last_played FROM track WHERE id = ?2), 0)
               ), 0),
               added_at = min(added_at, (SELECT added_at FROM track WHERE id = ?2)),
               artists_json = ?3,
               artists_version = ?4,
               album_artist = ?5
             WHERE id = ?1",
            params![
                keep,
                drop,
                encode_artists_json(&artists),
                keep_version.max(drop_version),
                album_artist
            ],
        )?;
        // Las tablas hijas restantes caen por ON DELETE CASCADE.
        tx.execute("DELETE FROM track WHERE id = ?1", params![drop])?;
        tx.commit()?;
        Ok(())
    }

    pub fn track_by_id(&self, id: i64) -> Result<Option<TrackRow>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("{SELECT_TRACK} WHERE t.id = ?1"))?;
        Ok(stmt.query_row(params![id], row_to_track).optional()?)
    }

    pub fn local_path(&self, key: &str) -> Option<PathBuf> {
        self.track_by_key(key)
            .ok()
            .flatten()
            .and_then(|row| row.path)
            .filter(|path| path.exists())
    }

    /// Búsqueda full-text. Si la query es basura para FTS5, no explota.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<TrackRow>> {
        let tokens: Vec<String> = query
            .split_whitespace()
            .map(|token| {
                token
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect::<String>()
            })
            .filter(|token| !token.is_empty() && token.chars().any(|c| c.is_alphanumeric()))
            .collect();
        if query.trim().is_empty() {
            return self.recent(limit);
        }
        if tokens.is_empty() {
            // Una consulta solo de puntuación no debe devolver “recientes”:
            // el usuario está filtrando, no pidiendo la lista por defecto.
            return Ok(Vec::new());
        }
        let fts_query = tokens
            .iter()
            .map(|token| format!("\"{}\"*", token.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" ");
        let sql = format!(
            "{SELECT_TRACK} JOIN track_fts f ON f.rowid = t.id
             WHERE track_fts MATCH ?1 AND t.hidden = 0 ORDER BY rank LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![fts_query, limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            // FTS5 no hace prefijos con acentos raros: fallback a LIKE. Se
            // escapan los comodines para que "_" o "%" no devuelvan todo.
            let literal = query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let like = format!("%{literal}%");
            return self.like_search(&like, limit);
        }
        Ok(rows)
    }

    fn like_search(&self, pattern: &str, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} WHERE (t.title LIKE ?1 ESCAPE '\\' OR t.artist LIKE ?1 ESCAPE '\\'
               OR t.album LIKE ?1 ESCAPE '\\') AND t.hidden = 0 ORDER BY t.artist LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Álbumes de la biblioteca agrupados por artista+álbum, con su portada y
    /// cuántas pistas hay descargadas. Es la base de la pantalla de álbumes.
    pub fn albums(&self) -> Result<Vec<AlbumRow>> {
        let sql = r#"
            SELECT t.album,
                   coalesce(nullif(t.album_artist, ''), t.artist) AS artist,
                   count(*),
                   sum(CASE WHEN s.status = 'ready' THEN 1 ELSE 0 END),
                   sum(t.duration_ms),
                   max(coalesce(t.art_path, '')),
                   max(coalesce(t.art_url, '')),
                   sum(t.favorite)
            FROM track t
            LEFT JOIN source s ON s.track_id = t.id
              AND s.id = (SELECT id FROM source x WHERE x.track_id = t.id
                          ORDER BY (x.status = 'ready') DESC, x.id DESC LIMIT 1)
            WHERE t.album IS NOT NULL AND t.album <> '' AND t.hidden = 0
            GROUP BY t.album, coalesce(nullif(t.album_artist, ''), t.artist)
            ORDER BY artist COLLATE NOCASE, t.album COLLATE NOCASE
        "#;
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt
            .query_map([], |row| {
                let path: String = row.get(5)?;
                let url: String = row.get(6)?;
                Ok(AlbumRow {
                    album: row.get(0)?,
                    artist: row.get(1)?,
                    track_count: row.get(2)?,
                    ready_count: row.get(3)?,
                    duration_ms: row.get::<_, i64>(4)?.max(0) as u64,
                    art_path: (!path.is_empty()).then(|| PathBuf::from(path)),
                    art_url: (!url.is_empty()).then_some(url),
                    favorite_count: row.get::<_, i64>(7)?.max(0) as u32,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Pistas de un álbum concreto, en orden de inserción.
    pub fn album_tracks(&self, album: &str, artist: &str) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} WHERE t.album = ?1
               AND coalesce(nullif(t.album_artist, ''), t.artist) = ?2
               AND t.hidden = 0 ORDER BY t.id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![album, artist], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} WHERE t.hidden = 0 ORDER BY t.added_at DESC, t.id DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// De más reciente a más antigua: lo último que entró en la biblioteca
    /// queda arriba. Antes iba por artista, así que "anterior" y "siguiente"
    /// aterrizaban siempre en el mismo extremo de la lista.
    pub fn all_tracks(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} WHERE t.hidden = 0 ORDER BY t.added_at DESC, t.id DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn playable(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} WHERE s.status = 'ready' AND s.path IS NOT NULL AND t.hidden = 0
             ORDER BY t.play_count DESC, t.artist LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn favorites(&self) -> Result<Vec<TrackRow>> {
        let sql = format!("{SELECT_TRACK} WHERE t.favorite = 1 AND t.hidden = 0 ORDER BY t.artist");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Alterna favorito y devuelve el estado nuevo.
    pub fn toggle_favorite(&mut self, key: &str) -> Result<bool> {
        self.conn.execute(
            "UPDATE track SET favorite = 1 - favorite WHERE key = ?1",
            params![key],
        )?;
        let value: Option<i64> = self
            .conn
            .query_row(
                "SELECT favorite FROM track WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or(0) == 1)
    }

    pub fn count(&self) -> Result<i64> {
        if let Some((at, value)) = self.count_cache.get() {
            if at.elapsed() < Duration::from_millis(250) {
                return Ok(value);
            }
        }
        let value =
            self.conn
                .query_row("SELECT count(*) FROM track WHERE hidden = 0", [], |r| {
                    r.get(0)
                })?;
        self.count_cache.set(Some((Instant::now(), value)));
        Ok(value)
    }

    pub fn ready_count(&self) -> Result<i64> {
        if let Some((at, value)) = self.ready_cache.get() {
            if at.elapsed() < Duration::from_millis(250) {
                return Ok(value);
            }
        }
        let value = self.conn.query_row(
            "SELECT count(DISTINCT s.track_id) FROM source s JOIN track t ON t.id = s.track_id WHERE s.status = 'ready' AND t.hidden = 0",
            [],
            |r| r.get(0),
        )?;
        self.ready_cache.set(Some((Instant::now(), value)));
        Ok(value)
    }

    /// Estadísticas de escucha para el Wrapped. `all_time=false` limita al
    /// año natural actual (hora local de SQLite).
    pub fn wrapped_stats(&self, all_time: bool) -> Result<WrappedStats> {
        let year: i64 = self
            .conn
            .query_row(
                "SELECT CAST(strftime('%Y','now','localtime') AS INTEGER)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let period = if all_time {
            "1=1".to_string()
        } else {
            format!(
                "CAST(strftime('%Y', e.ts / 1000, 'unixepoch', 'localtime') AS INTEGER) = {year}"
            )
        };
        let totals: (i64, i64, i64, i64) = self.conn.query_row(
            &format!(
                "SELECT coalesce(sum(e.ms_played),0), count(*),
                        count(DISTINCT e.track_id), count(DISTINCT t.artist)
                 FROM play_event e JOIN track t ON t.id = e.track_id
                 WHERE {period}"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let mut stats = WrappedStats {
            all_time,
            year,
            plays: totals.1.max(0) as u64,
            ms_played: totals.0.max(0) as u64,
            distinct_tracks: totals.2.max(0) as u64,
            distinct_artists: totals.3.max(0) as u64,
            ..WrappedStats::default()
        };

        let top_tracks_sql = format!(
            "SELECT t.key, t.title, t.artist, t.album, sum(e.ms_played), count(*),
                    t.art_path, t.art_url
             FROM play_event e JOIN track t ON t.id = e.track_id
             WHERE {period}
             GROUP BY t.id ORDER BY sum(e.ms_played) DESC, count(*) DESC LIMIT 8"
        );
        let mut stmt = self.conn.prepare(&top_tracks_sql)?;
        stats.top_tracks = stmt
            .query_map([], |row| {
                let path: Option<String> = row.get(6)?;
                Ok(WrappedTrack {
                    key: row.get(0)?,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    album: row.get(3)?,
                    ms_played: row.get::<_, i64>(4)?.max(0) as u64,
                    plays: row.get::<_, i64>(5)?.max(0) as u64,
                    art_path: path.map(PathBuf::from),
                    art_url: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let top_artists_sql = format!(
            "SELECT t.artist, sum(e.ms_played), count(*)
             FROM play_event e JOIN track t ON t.id = e.track_id
             WHERE {period}
             GROUP BY lower(t.artist) ORDER BY sum(e.ms_played) DESC LIMIT 6"
        );
        let mut stmt = self.conn.prepare(&top_artists_sql)?;
        stats.top_artists = stmt
            .query_map([], |row| {
                Ok(WrappedArtist {
                    artist: row.get(0)?,
                    ms_played: row.get::<_, i64>(1)?.max(0) as u64,
                    plays: row.get::<_, i64>(2)?.max(0) as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let top_albums_sql = format!(
            "SELECT t.album, t.artist, sum(e.ms_played), count(*),
                    max(coalesce(t.art_url, ''))
             FROM play_event e JOIN track t ON t.id = e.track_id
             WHERE {period} AND t.album IS NOT NULL AND t.album <> ''
             GROUP BY lower(t.album), lower(t.artist)
             ORDER BY sum(e.ms_played) DESC LIMIT 6"
        );
        let mut stmt = self.conn.prepare(&top_albums_sql)?;
        stats.top_albums = stmt
            .query_map([], |row| {
                let url: String = row.get(4)?;
                Ok(WrappedAlbum {
                    album: row.get(0)?,
                    artist: row.get(1)?,
                    ms_played: row.get::<_, i64>(2)?.max(0) as u64,
                    plays: row.get::<_, i64>(3)?.max(0) as u64,
                    art_url: (!url.is_empty()).then_some(url),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let by_month_sql = format!(
            "SELECT CAST(strftime('%m', e.ts / 1000, 'unixepoch', 'localtime') AS INTEGER),
                    sum(e.ms_played)
             FROM play_event e WHERE {period}
             GROUP BY 1"
        );
        let mut stmt = self.conn.prepare(&by_month_sql)?;
        for row in stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))? {
            let (month, ms) = row?;
            let index = (month - 1).clamp(0, 11) as usize;
            stats.by_month[index] = ms.max(0) as u64;
        }
        Ok(stats)
    }

    pub fn record_play(&mut self, key: &str, ms_played: u64, completed: bool) -> Result<()> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM track WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(id) = id else { return Ok(()) };
        self.conn.execute(
            "INSERT INTO play_event(track_id, ts, ms_played, completed) VALUES (?1, ?2, ?3, ?4)",
            params![id, now_ms(), ms_played as i64, completed as i64],
        )?;
        self.conn.execute(
            "UPDATE track SET play_count = play_count + 1, last_played = ?1 WHERE id = ?2",
            params![now_ms(), id],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------- letras
    pub fn lyrics(&self, key: &str) -> Result<Option<LyricsRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT l.lrc, l.synced, l.source, l.offset_ms, l.aligned,
                        l.align_version, l.policy_version, l.captions_timed
                 FROM lyrics l JOIN track t ON t.id = l.track_id WHERE t.key = ?1",
                params![key],
                |r| {
                    Ok(LyricsRow {
                        lrc: r.get(0)?,
                        synced: r.get::<_, i64>(1)? != 0,
                        source: r.get(2)?,
                        offset_ms: r.get(3)?,
                        aligned: r.get::<_, i64>(4)? != 0,
                        align_version: r.get(5)?,
                        policy_version: r.get(6)?,
                        captions_timed: r.get::<_, i64>(7)? != 0,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn save_lyrics(
        &mut self,
        key: &str,
        lrc: &str,
        synced: bool,
        source: &str,
        aligned: bool,
        captions_timed: bool,
    ) -> Result<()> {
        let Some(id) = self
            .conn
            .query_row("SELECT id FROM track WHERE key = ?1", params![key], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
        else {
            return Ok(());
        };
        let version = if aligned { LYRICS_ALIGN_VERSION } else { 0 };
        self.conn.execute(
            r#"
            INSERT INTO lyrics(track_id, lrc, synced, source, aligned,
                               align_version, policy_version, fetched_at,
                               captions_timed)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(track_id) DO UPDATE SET
              lrc = excluded.lrc, synced = excluded.synced,
              source = excluded.source, aligned = excluded.aligned,
              align_version = excluded.align_version,
              policy_version = excluded.policy_version,
              fetched_at = excluded.fetched_at,
              captions_timed = excluded.captions_timed
            "#,
            params![
                id,
                lrc,
                synced as i64,
                source,
                aligned as i64,
                version,
                LYRICS_POLICY_VERSION,
                now_ms(),
                captions_timed as i64
            ],
        )?;
        Ok(())
    }

    /// Borra la letra cacheada de una pista (para volver a buscarla).
    pub fn delete_lyrics(&mut self, key: &str) -> Result<()> {
        let Some(row) = self.track_by_key(key)? else {
            return Ok(());
        };
        self.conn
            .execute("DELETE FROM lyrics WHERE track_id = ?1", params![row.id])?;
        Ok(())
    }

    pub fn set_lyrics_offset(&mut self, key: &str, offset_ms: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE lyrics SET offset_ms = ?1 WHERE track_id = (SELECT id FROM track WHERE key = ?2)",
            params![offset_ms, key],
        )?;
        Ok(())
    }

    pub fn lyrics_offset(&self, key: &str) -> Result<i64> {
        Ok(self.lyrics(key)?.map(|row| row.offset_ms).unwrap_or(0))
    }

    // -------------------------------------------------------------- recorte
    /// Recorte persistente de una pista: intro a saltar y final opcional.
    pub fn trim(&self, key: &str) -> Result<(u64, Option<u64>)> {
        let row = self
            .conn
            .query_row(
                "SELECT t.start_ms, t.end_ms FROM track_trim t
                 JOIN track k ON k.id = t.track_id WHERE k.key = ?1",
                params![key],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?.max(0) as u64,
                        r.get::<_, Option<i64>>(1)?,
                    ))
                },
            )
            .optional()?;
        Ok(row
            .map(|(start, end)| (start, end.map(|v| v.max(0) as u64)))
            .unwrap_or((0, None)))
    }

    pub fn set_trim(&mut self, key: &str, start_ms: u64, end_ms: Option<u64>) -> Result<()> {
        let Some(id) = self
            .conn
            .query_row("SELECT id FROM track WHERE key = ?1", params![key], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
        else {
            return Ok(());
        };
        self.conn.execute(
            r#"INSERT INTO track_trim(track_id, start_ms, end_ms) VALUES (?1, ?2, ?3)
               ON CONFLICT(track_id) DO UPDATE SET start_ms = excluded.start_ms, end_ms = excluded.end_ms"#,
            params![id, start_ms as i64, end_ms.map(|v| v as i64)],
        )?;
        Ok(())
    }

    pub fn clear_trim(&mut self, key: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM track_trim WHERE track_id = (SELECT id FROM track WHERE key = ?1)",
            params![key],
        )?;
        Ok(())
    }

    // --------------------------------------------------------------- arte
    pub fn art_path(&self, key: &str) -> Result<Option<PathBuf>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT art_path FROM track WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(value.map(PathBuf::from))
    }

    pub fn art_url(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT art_url FROM track WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_art(&mut self, key: &str, path: Option<&Path>, url: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE track SET art_path = coalesce(?1, art_path), art_url = coalesce(?2, art_url) WHERE key = ?3",
            params![
                path.map(|p| p.to_string_lossy().to_string()),
                url,
                key
            ],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------- playlists
    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        if let Some((at, rows)) = self.playlists_cache.borrow().as_ref() {
            if at.elapsed() < Duration::from_millis(250) {
                return Ok(rows.clone());
            }
        }
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, (SELECT count(*) FROM playlist_item i JOIN track t ON t.id = i.track_id
                    WHERE i.playlist_id = p.id AND t.hidden = 0),
                    p.description, p.cover_key, p.cover_url
             FROM playlist p ORDER BY p.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    count: row.get(2)?,
                    description: row.get(3)?,
                    cover_key: row.get(4)?,
                    cover_url: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        *self.playlists_cache.borrow_mut() = Some((Instant::now(), rows.clone()));
        Ok(rows)
    }

    pub fn create_playlist(&mut self, name: &str) -> Result<i64> {
        self.create_playlist_full(name, "", None, None)
    }

    /// Crea una lista con descripción y portada (automática si `cover_key` es
    /// la primera pista y `cover_url` es None).
    pub fn create_playlist_full(
        &mut self,
        name: &str,
        description: &str,
        cover_key: Option<&str>,
        cover_url: Option<&str>,
    ) -> Result<i64> {
        *self.playlists_cache.borrow_mut() = None;
        self.conn.execute(
            "INSERT INTO playlist(name, description, cover_key, cover_url, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, description, cover_key, cover_url, now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_playlist(&mut self, playlist_id: i64) -> Result<()> {
        *self.playlists_cache.borrow_mut() = None;
        self.conn
            .execute("DELETE FROM playlist WHERE id = ?1", params![playlist_id])?;
        Ok(())
    }

    /// Portada de una lista: la guardada, o la primera pista con arte.
    pub fn playlist_cover(&self, playlist_id: i64) -> Option<String> {
        if let Ok(Some(url)) = self
            .conn
            .query_row(
                "SELECT cover_url FROM playlist WHERE id = ?1 AND cover_url IS NOT NULL",
                params![playlist_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
        {
            return Some(url);
        }
        if let Ok(Some(key)) = self
            .conn
            .query_row(
                "SELECT p.cover_key FROM playlist p WHERE p.id = ?1 AND p.cover_key IS NOT NULL",
                params![playlist_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
        {
            return Some(key);
        }
        let key: Option<String> = self
            .conn
            .query_row(
                r#"SELECT t.key FROM playlist_item i JOIN track t ON t.id = i.track_id
                   WHERE i.playlist_id = ?1
                     AND (t.art_path IS NOT NULL OR t.art_url IS NOT NULL)
                   ORDER BY i.position LIMIT 1"#,
                params![playlist_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten();
        if key.is_some() {
            return key;
        }
        // Último recurso: el jpg que yt-dlp deja junto al audio.
        self.playlist_tracks(playlist_id)
            .ok()?
            .into_iter()
            .find(|row| {
                row.path
                    .as_ref()
                    .map(|path| path.with_extension("jpg").exists())
                    .unwrap_or(false)
            })
            .map(|row| row.track.key)
    }

    /// Metadatos de una lista, para la cabecera de la biblioteca.
    pub fn playlist(&self, playlist_id: i64) -> Result<Option<Playlist>> {
        Ok(self
            .conn
            .query_row(
                "SELECT p.id, p.name,
                        (SELECT count(*) FROM playlist_item i WHERE i.playlist_id = p.id),
                        p.description, p.cover_key, p.cover_url
                 FROM playlist p WHERE p.id = ?1",
                params![playlist_id],
                |row| {
                    Ok(Playlist {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        count: row.get(2)?,
                        description: row.get(3)?,
                        cover_key: row.get(4)?,
                        cover_url: row.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} JOIN playlist_item i ON i.track_id = t.id
             WHERE i.playlist_id = ?1 AND t.hidden = 0 ORDER BY i.position"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![playlist_id], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn add_to_playlist(&mut self, playlist_id: i64, key: &str) -> Result<()> {
        *self.playlists_cache.borrow_mut() = None;
        self.conn.execute(
            r#"
            INSERT OR IGNORE INTO playlist_item(playlist_id, track_id, position)
            SELECT ?1, t.id, coalesce((SELECT max(position) + 1 FROM playlist_item WHERE playlist_id = ?1), 0)
            FROM track t WHERE t.key = ?2
            "#,
            params![playlist_id, key],
        )?;
        Ok(())
    }

    pub fn remove_from_playlist(&mut self, playlist_id: i64, key: &str) -> Result<()> {
        *self.playlists_cache.borrow_mut() = None;
        self.conn.execute(
            "DELETE FROM playlist_item WHERE playlist_id = ?1
             AND track_id = (SELECT id FROM track WHERE key = ?2)",
            params![playlist_id, key],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ settings
    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT value FROM setting WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO setting(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Saca una pista de la biblioteca sin borrar la fila: conserva historial
    /// (Wrapped), letra y fuentes. El audio sigue en disco. Es lo que hace el
    /// borrado rápido; para eliminar de verdad está [`delete_track`].
    pub fn remove_from_library(&mut self, key: &str) -> Result<()> {
        self.count_cache.set(None);
        self.ready_cache.set(None);
        *self.playlists_cache.borrow_mut() = None;
        self.conn
            .execute("UPDATE track SET hidden = 1 WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// Borra una pista de la biblioteca (fuentes, listas, letras y eventos
    /// caen en cascada). Devuelve la ruta del audio para poder borrarla del
    /// disco si el usuario lo pide. Solo para «borrar del disco»: el atajo
    /// normal usa [`remove_from_library`] para no perder el historial.
    pub fn delete_track(&mut self, key: &str) -> Result<Option<PathBuf>> {
        self.count_cache.set(None);
        self.ready_cache.set(None);
        *self.playlists_cache.borrow_mut() = None;
        let path: Option<String> = self
            .conn
            .query_row(
                "SELECT s.path FROM source s JOIN track t ON t.id = s.track_id
                 WHERE t.key = ?1 AND s.path IS NOT NULL
                 ORDER BY (s.status = 'ready') DESC LIMIT 1",
                params![key],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        self.conn
            .execute("DELETE FROM track WHERE key = ?1", params![key])?;
        Ok(path.map(PathBuf::from))
    }

    /// Pistas a las que les falta portada, para el backfill desde Spotify.
    pub fn tracks_without_art(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK}
             WHERE t.art_path IS NULL AND t.art_url IS NULL AND t.hidden = 0
             ORDER BY t.added_at DESC LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Actualiza metadatos que llegan de Spotify/Deezer sin pisar lo que ya
    /// había con datos peores (álbum vacío, duración 0...).
    pub fn update_metadata(
        &mut self,
        key: &str,
        artist: &str,
        title: &str,
        album: Option<&str>,
        duration_ms: u64,
        art_url: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            r#"UPDATE track SET
                 artist = CASE WHEN ?2 <> '' THEN ?2 ELSE artist END,
                 title = CASE WHEN ?3 <> '' THEN ?3 ELSE title END,
                 album = coalesce(?4, album),
                 duration_ms = CASE WHEN ?5 > 0 THEN ?5 ELSE duration_ms END,
                 art_url = coalesce(?6, art_url)
               WHERE key = ?1"#,
            params![key, artist, title, album, duration_ms as i64, art_url],
        )?;
        Ok(())
    }

    /// Borra canciones sin fichero ni descarga en curso.
    pub fn prune_missing(&mut self) -> Result<usize> {
        let removed = self
            .conn
            .execute("DELETE FROM source WHERE status = 'missing'", [])?;
        Ok(removed)
    }
}

const SELECT_TRACK: &str = r#"
SELECT t.id, t.key, t.title, t.artist, t.album, t.duration_ms, t.art_url, t.art_path,
       s.path, s.status, s.provider, t.favorite, t.play_count, t.added_at,
       t.artists_json, t.album_artist
FROM track t
LEFT JOIN source s ON s.track_id = t.id
  AND s.id = (SELECT id FROM source x WHERE x.track_id = t.id
              ORDER BY (x.status = 'ready') DESC, x.id DESC LIMIT 1)
"#;

fn row_to_track(row: &Row<'_>) -> rusqlite::Result<TrackRow> {
    let art_path: Option<String> = row.get(7)?;
    let path: Option<String> = row.get(8)?;
    let status: Option<String> = row.get(9)?;
    let artists_json: Option<String> = row.get(14)?;
    let album_artist: Option<String> = row.get(15)?;
    Ok(TrackRow {
        id: row.get(0)?,
        track: TrackRef {
            key: row.get(1)?,
            title: row.get(2)?,
            artist: row.get(3)?,
            album: row.get(4)?,
            duration_ms: row.get::<_, i64>(5)?.max(0) as u64,
            art_url: row.get(6)?,
            source_hint: None,
        },
        path: path.map(PathBuf::from),
        status: SourceStatus::from_str(status.as_deref().unwrap_or("pending")),
        provider: row.get(10)?,
        favorite: row.get::<_, i64>(11)? != 0,
        play_count: row.get::<_, i64>(12)?.max(0) as u32,
        art_path: art_path.map(PathBuf::from),
        added_at: row.get::<_, i64>(13).unwrap_or(0),
        artists: parse_artists_json(artists_json.as_deref()),
        album_artist: album_artist.filter(|value| !value.trim().is_empty()),
    })
}

/// Separa un crédito de artista en artistas individuales. Solo reconoce
/// marcadores explícitos de colaboración (`feat.`, `ft.`, `featuring`) y el
/// punto y coma; nunca comas, `&` ni «and», que forman parte de muchos
/// nombres («Earth, Wind & Fire», «Tyler, The Creator»). Conserva el orden y
/// elimina duplicados sin distinguir mayúsculas.
pub fn split_artist_credit(artist: &str) -> Vec<String> {
    const MARKERS: [&str; 5] = [" feat. ", " feat ", " ft. ", " ft ", " featuring "];
    let source = artist.trim();
    if source.is_empty() {
        return Vec::new();
    }
    let bytes = source.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let rest = &source[index..];
        let mut cut = None;
        for marker in MARKERS {
            if let Some(head) = rest.get(..marker.len()) {
                if head.eq_ignore_ascii_case(marker) {
                    cut = Some(index + marker.len());
                    break;
                }
            }
        }
        if cut.is_none() && bytes[index] == b';' {
            cut = Some(index + 1);
        }
        match cut {
            Some(after) => {
                push_unique_artist(&mut parts, &source[start..index]);
                start = after;
                index = after;
            }
            None => {
                index += source[index..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(1);
            }
        }
    }
    push_unique_artist(&mut parts, &source[start..]);
    parts
}

fn push_unique_artist(artists: &mut Vec<String>, artist: &str) {
    let artist = artist.trim();
    if artist.is_empty() {
        return;
    }
    let folded = artist.to_lowercase();
    if artists.iter().any(|known| known.to_lowercase() == folded) {
        return;
    }
    artists.push(artist.to_string());
}

/// Lee la lista de artistas cacheada en JSON. Una fila sin enriquecer (NULL)
/// o un JSON corrupto devuelven una lista vacía.
pub fn parse_artists_json(s: Option<&str>) -> Vec<String> {
    let Some(s) = s else { return Vec::new() };
    serde_json::from_str(s).unwrap_or_default()
}

/// Serializa la lista de artistas. Una lista vacía se guarda como NULL para
/// que la fila siga contando como pendiente de enriquecer.
pub fn encode_artists_json(artists: &[String]) -> Option<String> {
    if artists.is_empty() {
        return None;
    }
    serde_json::to_string(artists).ok()
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str, title: &str, artist: &str) -> TrackRef {
        TrackRef {
            key: key.into(),
            title: title.into(),
            artist: artist.into(),
            album: Some("Album".into()),
            duration_ms: 200_000,
            art_url: None,
            source_hint: None,
        }
    }

    #[test]
    fn albums_group_tracks_and_keep_cover() {
        let mut lib = Library::open_in_memory().unwrap();
        let mut first = track("k1", "One", "Artist");
        first.album = Some("Album".into());
        first.art_url = Some("https://example.com/a.jpg".into());
        let mut second = track("k2", "Two", "Artist");
        second.album = Some("Album".into());
        let mut other = track("k3", "Three", "Other");
        other.album = Some("Another".into());
        for item in [&first, &second, &other] {
            lib.upsert_track(item).unwrap();
        }
        let albums = lib.albums().unwrap();
        assert_eq!(albums.len(), 2);
        let album = albums.iter().find(|a| a.album == "Album").unwrap();
        assert_eq!(album.artist, "Artist");
        assert_eq!(album.track_count, 2);
        assert_eq!(album.art_url.as_deref(), Some("https://example.com/a.jpg"));
        assert_eq!(lib.album_tracks("Album", "Artist").unwrap().len(), 2);
    }

    #[test]
    fn wrapped_aggregates_play_events() {
        let mut lib = Library::open_in_memory().unwrap();
        let mut first = track("k1", "One", "Artist");
        first.album = Some("Album".into());
        let second = track("k2", "Two", "Artist");
        lib.upsert_track(&first).unwrap();
        lib.upsert_track(&second).unwrap();
        lib.record_play("k1", 60_000, true).unwrap();
        lib.record_play("k1", 30_000, true).unwrap();
        lib.record_play("k2", 20_000, false).unwrap();
        let stats = lib.wrapped_stats(false).unwrap();
        assert_eq!(stats.plays, 3);
        assert_eq!(stats.ms_played, 110_000);
        assert_eq!(stats.distinct_tracks, 2);
        assert_eq!(stats.top_tracks[0].key, "k1");
        assert_eq!(stats.top_tracks[0].ms_played, 90_000);
        assert_eq!(stats.top_artists[0].artist, "Artist");
        assert_eq!(stats.top_albums[0].album, "Album");
        assert!(stats.by_month.iter().any(|ms| *ms > 0));
    }

    #[test]
    fn hide_keeps_history_and_upsert_restores() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("k", "Song", "Artist")).unwrap();
        lib.record_play("k", 60_000, true).unwrap();
        lib.remove_from_library("k").unwrap();
        assert_eq!(lib.count().unwrap(), 0);
        assert!(lib.all_tracks(10).unwrap().is_empty());
        // El historial sigue alimentando Wrapped aunque la pista esté oculta.
        assert_eq!(lib.wrapped_stats(true).unwrap().plays, 1);
        // Volver a importarla/reproducirla la devuelve a la biblioteca.
        lib.upsert_track(&track("k", "Song", "Artist")).unwrap();
        assert_eq!(lib.count().unwrap(), 1);
    }

    #[test]
    fn delete_track_removes_sources_and_playlists() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("k", "Song", "Artist")).unwrap();
        let list = lib.create_playlist("List").unwrap();
        lib.add_to_playlist(list, "k").unwrap();
        assert!(lib.delete_track("k").unwrap().is_none());
        assert_eq!(lib.playlist_tracks(list).unwrap().len(), 0);
        assert!(lib.track_by_key("k").unwrap().is_none());
    }

    #[test]
    fn playlists_create_delete_and_auto_cover() {
        let mut lib = Library::open_in_memory().unwrap();
        let mut with_art = track("isrc:abc", "Song", "Artist");
        with_art.art_url = Some("https://example.com/cover.jpg".into());
        lib.upsert_track(&with_art).unwrap();
        let id = lib
            .create_playlist_full("Road trip", "para el coche", None, None)
            .unwrap();
        lib.add_to_playlist(id, "isrc:abc").unwrap();
        let list = lib.playlist(id).unwrap().unwrap();
        assert_eq!(list.description, "para el coche");
        assert_eq!(lib.playlist_cover(id).as_deref(), Some("isrc:abc"));
        lib.delete_playlist(id).unwrap();
        assert!(lib.playlist(id).unwrap().is_none());
        assert!(lib.playlist_tracks(id).unwrap().is_empty());
    }

    #[test]
    fn upsert_is_idempotent_and_updates_fields() {
        let mut lib = Library::open_in_memory().unwrap();
        let id1 = lib.upsert_track(&track("sha1:a", "One", "Artist")).unwrap();
        let id2 = lib
            .upsert_track(&track("sha1:a", "One (Remaster)", "Artist"))
            .unwrap();
        assert_eq!(id1, id2);
        assert_eq!(lib.count().unwrap(), 1);
        assert_eq!(
            lib.track_by_key("sha1:a").unwrap().unwrap().track.title,
            "One (Remaster)"
        );
    }

    #[test]
    fn search_finds_by_artist_and_title() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("sha1:a", "Do I Wanna Know", "Arctic Monkeys"))
            .unwrap();
        lib.upsert_track(&track("sha1:b", "505", "Arctic Monkeys"))
            .unwrap();
        lib.upsert_track(&track("sha1:c", "Nude", "Radiohead"))
            .unwrap();
        assert_eq!(lib.search("arctic", 10).unwrap().len(), 2);
        assert_eq!(lib.search("505", 10).unwrap().len(), 1);
        assert_eq!(lib.search("radiohead", 10).unwrap().len(), 1);
        // Sin tokens útiles se comporta como "recientes", no como error.
        assert_eq!(lib.search("!!!", 10).unwrap().len(), 0);
        assert_eq!(lib.search("", 10).unwrap().len(), 3);
    }

    #[test]
    fn imported_album_names_and_cover_are_cleaned() {
        let mut lib = Library::open_in_memory().unwrap();
        let list = lib
            .create_playlist_full("Mix", "", None, Some("https://cover/mix.jpg"))
            .unwrap();
        let mut imported = track("isrc:uno", "Uno", "Artista");
        imported.album = Some("Mix".into());
        imported.art_url = Some("https://cover/mix.jpg".into());
        let id = lib.upsert_track(&imported).unwrap();
        lib.set_source(
            id,
            "spotify",
            Some("uno"),
            None,
            SourceStatus::Pending,
            None,
        )
        .unwrap();
        lib.add_to_playlist(list, "isrc:uno").unwrap();
        let mut local = track("sha1:dos", "Dos", "Artista");
        local.album = Some("Mix".into());
        lib.upsert_track(&local).unwrap();
        lib.add_to_playlist(list, "sha1:dos").unwrap();

        assert_eq!(lib.clear_imported_albums().unwrap(), 1);
        let imported = lib.track_by_key("isrc:uno").unwrap().unwrap();
        assert!(imported.track.album.is_none());
        assert!(imported.track.art_url.is_none());
        let local = lib.track_by_key("sha1:dos").unwrap().unwrap();
        assert_eq!(local.track.album.as_deref(), Some("Mix"));
    }

    #[test]
    fn shared_art_across_albums_is_cleared() {
        let mut lib = Library::open_in_memory().unwrap();
        let playlist_cover = Some("https://cover/playlist.jpg".to_string());
        let mut a = track("sha1:a", "A", "X");
        a.album = Some("Album A".into());
        a.art_url = playlist_cover.clone();
        let mut b = track("sha1:b", "B", "X");
        b.album = Some("Album B".into());
        b.art_url = playlist_cover;
        for item in [&a, &b] {
            lib.upsert_track(item).unwrap();
        }
        // Dos pistas del mismo álbum sí pueden compartir carátula.
        let album_cover = Some("https://cover/album.jpg".to_string());
        let mut c = track("sha1:c", "C", "X");
        c.album = Some("Album C".into());
        c.art_url = album_cover.clone();
        let mut d = track("sha1:d", "D", "X");
        d.album = Some("Album C".into());
        d.art_url = album_cover;
        for item in [&c, &d] {
            lib.upsert_track(item).unwrap();
        }

        assert_eq!(lib.clear_shared_art().unwrap(), 2);
        assert!(lib
            .track_by_key("sha1:a")
            .unwrap()
            .unwrap()
            .track
            .art_url
            .is_none());
        assert!(lib
            .track_by_key("sha1:b")
            .unwrap()
            .unwrap()
            .track
            .art_url
            .is_none());
        assert_eq!(
            lib.track_by_key("sha1:c")
                .unwrap()
                .unwrap()
                .track
                .art_url
                .as_deref(),
            Some("https://cover/album.jpg")
        );
    }

    #[test]
    fn merge_duplicates_keeps_downloaded_and_moves_playlist() {
        let mut lib = Library::open_in_memory().unwrap();
        let local = lib
            .upsert_track(&track("sha1:local", "El Hueso", "Ralphie Choo"))
            .unwrap();
        lib.set_source(
            local,
            "local",
            None,
            Some(Path::new("/tmp/el-hueso.opus")),
            SourceStatus::Ready,
            None,
        )
        .unwrap();
        let imported = lib
            .upsert_track(&track("isrc:abc", "El Hueso", "Ralphie Choo"))
            .unwrap();
        lib.set_source(
            imported,
            "spotify",
            Some("abc"),
            None,
            SourceStatus::Pending,
            None,
        )
        .unwrap();
        let list = lib.create_playlist("Lista").unwrap();
        lib.add_to_playlist(list, "isrc:abc").unwrap();

        assert_eq!(lib.duplicate_count().unwrap(), 1);
        assert_eq!(lib.merge_duplicates().unwrap(), 1);
        assert!(lib.track_by_key("isrc:abc").unwrap().is_none());
        assert!(lib.track_by_key("sha1:local").unwrap().is_some());
        let rows = lib.playlist_tracks(list).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].track.key, "sha1:local");
        assert_eq!(rows[0].status, SourceStatus::Ready);
        assert_eq!(lib.ready_count().unwrap(), 1);
        assert_eq!(
            lib.canonical_key(&track("isrc:otro", "El Hueso", "Ralphie Choo")),
            "sha1:local"
        );
    }

    #[test]
    fn canonical_key_does_not_swap_a_different_duration() {
        let mut lib = Library::open_in_memory().unwrap();
        let local = track("sha1:local", "El Hueso", "Ralphie Choo");
        lib.upsert_track(&local).unwrap();

        // Misma pareja título/artista, pero 5 s más: puede ser el directo o el
        // videoclip. Para reproducir no se cambia por la copia local.
        let mut other = track("isrc:otro", "El Hueso", "Ralphie Choo");
        other.duration_ms = local.duration_ms + 5_000;
        assert_eq!(lib.canonical_key(&other), "isrc:otro");

        let same = track("isrc:igual", "El Hueso", "Ralphie Choo");
        assert_eq!(lib.canonical_key(&same), "sha1:local");
    }

    #[test]
    fn sources_prefer_ready_path() {
        let mut lib = Library::open_in_memory().unwrap();
        let id = lib.upsert_track(&track("sha1:a", "One", "Artist")).unwrap();
        lib.set_source(
            id,
            "youtube",
            Some("vid"),
            None,
            SourceStatus::Pending,
            None,
        )
        .unwrap();
        lib.set_source(
            id,
            "youtube",
            Some("vid"),
            Some(Path::new("/tmp/one.opus")),
            SourceStatus::Ready,
            Some(123),
        )
        .unwrap();
        let row = lib.track_by_key("sha1:a").unwrap().unwrap();
        assert_eq!(row.status, SourceStatus::Ready);
        assert_eq!(row.path.as_deref(), Some(Path::new("/tmp/one.opus")));
        assert_eq!(lib.ready_count().unwrap(), 1);
    }

    #[test]
    fn playlists_keep_order_and_dedupe() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("sha1:a", "A", "X")).unwrap();
        lib.upsert_track(&track("sha1:b", "B", "X")).unwrap();
        let list = lib.create_playlist("Roadtrip").unwrap();
        lib.add_to_playlist(list, "sha1:a").unwrap();
        lib.add_to_playlist(list, "sha1:b").unwrap();
        lib.add_to_playlist(list, "sha1:a").unwrap();
        let rows = lib.playlist_tracks(list).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].track.key, "sha1:a");
        assert_eq!(rows[1].track.key, "sha1:b");
        lib.remove_from_playlist(list, "sha1:a").unwrap();
        assert_eq!(lib.playlist_tracks(list).unwrap().len(), 1);
    }

    #[test]
    fn lyrics_and_favorite_roundtrip() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("sha1:a", "A", "X")).unwrap();
        assert!(lib.toggle_favorite("sha1:a").unwrap());
        assert_eq!(lib.favorites().unwrap().len(), 1);
        assert!(!lib.toggle_favorite("sha1:a").unwrap());
        assert_eq!(lib.favorites().unwrap().len(), 0);
        lib.save_lyrics("sha1:a", "[00:01.00]hola", true, "lrclib", false, false)
            .unwrap();
        let row = lib.lyrics("sha1:a").unwrap().unwrap();
        assert_eq!(row.lrc, "[00:01.00]hola");
        assert!(row.synced);
        assert_eq!(row.source, "lrclib");
        assert!(!row.aligned);
        assert_eq!(row.align_version, 0);
        assert_eq!(row.policy_version, super::LYRICS_POLICY_VERSION);
        assert!(!row.captions_timed);
        lib.save_lyrics("sha1:a", "[00:01.00]hola", true, "lrclib", true, true)
            .unwrap();
        let row = lib.lyrics("sha1:a").unwrap().unwrap();
        assert!(row.aligned);
        assert!(row.captions_timed);
        assert_eq!(row.align_version, super::LYRICS_ALIGN_VERSION);
        lib.set_lyrics_offset("sha1:a", 250).unwrap();
        assert_eq!(lib.lyrics_offset("sha1:a").unwrap(), 250);
    }

    #[test]
    fn lyrics_policy_marks_trusted_and_leaves_ambiguous_for_retry() {
        let mut lib = Library::open_in_memory().unwrap();
        for (key, source) in [
            ("sha1:trusted", "lrclib"),
            ("sha1:plain", "plain"),
            ("sha1:captions", "captions"),
            ("sha1:asr", "asr"),
            ("sha1:manual", "manual"),
        ] {
            lib.upsert_track(&track(key, key, "X")).unwrap();
            lib.save_lyrics(key, "[00:01.00]hola", true, source, false, false)
                .unwrap();
        }
        lib.conn
            .execute("UPDATE lyrics SET policy_version = 0", [])
            .unwrap();
        lib.migrate_lyrics_policy().unwrap();
        let version = |key: &str| lib.lyrics(key).unwrap().unwrap().policy_version;
        assert_eq!(version("sha1:trusted"), super::LYRICS_POLICY_VERSION);
        assert_eq!(version("sha1:asr"), super::LYRICS_POLICY_VERSION);
        assert_eq!(version("sha1:manual"), super::LYRICS_POLICY_VERSION);
        assert_eq!(version("sha1:plain"), 0);
        assert_eq!(version("sha1:captions"), 0);
        // No se borra nada: si no hay red, la letra sigue disponible.
        assert_eq!(
            lib.lyrics("sha1:plain").unwrap().unwrap().lrc,
            "[00:01.00]hola"
        );
        assert_eq!(
            lib.lyrics("sha1:captions").unwrap().unwrap().lrc,
            "[00:01.00]hola"
        );
    }

    #[test]
    fn trim_roundtrip_is_persistent_and_reversible() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("sha1:trim", "A", "X")).unwrap();
        assert_eq!(lib.trim("sha1:trim").unwrap(), (0, None));
        lib.set_trim("sha1:trim", 12_500, Some(180_000)).unwrap();
        assert_eq!(lib.trim("sha1:trim").unwrap(), (12_500, Some(180_000)));
        lib.clear_trim("sha1:trim").unwrap();
        assert_eq!(lib.trim("sha1:trim").unwrap(), (0, None));
    }

    #[test]
    fn settings_roundtrip() {
        let mut lib = Library::open_in_memory().unwrap();
        assert!(lib.setting("viz").is_none());
        lib.set_setting("viz", "aurora").unwrap();
        assert_eq!(lib.setting("viz").as_deref(), Some("aurora"));
        lib.set_setting("viz", "void").unwrap();
        assert_eq!(lib.setting("viz").as_deref(), Some("void"));
    }

    #[test]
    fn migrations_add_artist_columns_to_old_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE track(
                  id          INTEGER PRIMARY KEY,
                  key         TEXT NOT NULL UNIQUE,
                  title       TEXT NOT NULL,
                  artist      TEXT NOT NULL,
                  album       TEXT,
                  duration_ms INTEGER NOT NULL DEFAULT 0,
                  art_url     TEXT,
                  art_path    TEXT,
                  added_at    INTEGER NOT NULL,
                  favorite    INTEGER NOT NULL DEFAULT 0,
                  play_count  INTEGER NOT NULL DEFAULT 0,
                  last_played INTEGER,
                  hidden      INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO track(key, title, artist, album, added_at)
                VALUES ('old:1', 'Vieja', 'Artista', 'Álbum', 1234);
                "#,
            )
            .unwrap();
        }
        let lib = Library::open(&path).unwrap();
        let mut stmt = lib.conn.prepare("PRAGMA table_info(track)").unwrap();
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for column in ["artists_json", "artists_version", "album_artist", "hidden"] {
            assert!(columns.iter().any(|name| name == column), "falta {column}");
        }
        let row = lib.track_by_key("old:1").unwrap().unwrap();
        assert_eq!(row.id, 1);
        assert_eq!(row.track.key, "old:1");
        assert!(row.artists.is_empty());
        assert!(row.album_artist.is_none());
        let hidden: i64 = lib
            .conn
            .query_row("SELECT hidden FROM track WHERE key = 'old:1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(hidden, 0);
    }

    #[test]
    fn split_artist_credit_only_cuts_explicit_markers() {
        assert_eq!(split_artist_credit("A feat. B"), vec!["A", "B"]);
        assert_eq!(split_artist_credit("A FEAT B"), vec!["A", "B"]);
        assert_eq!(split_artist_credit("A ft. B; C"), vec!["A", "B", "C"]);
        assert_eq!(split_artist_credit("A featuring B feat. a"), vec!["A", "B"]);
        // Comas, «&» y «and» forman parte del nombre: no se tocan.
        assert_eq!(
            split_artist_credit("Tyler, The Creator"),
            vec!["Tyler, The Creator"]
        );
        assert_eq!(
            split_artist_credit("Earth, Wind & Fire"),
            vec!["Earth, Wind & Fire"]
        );
        assert!(split_artist_credit("   ").is_empty());
    }

    #[test]
    fn update_artists_roundtrips_and_is_idempotent() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("k", "Song", "A feat. B")).unwrap();
        let artists = vec!["A".to_string(), "B".to_string()];
        lib.update_artists("k", &artists, 1).unwrap();
        let row = lib.track_by_key("k").unwrap().unwrap();
        assert_eq!(row.artists, artists);
        assert_eq!(
            lib.tracks_without_artists(10)
                .unwrap()
                .iter()
                .filter(|row| row.track.key == "k")
                .count(),
            0
        );
        lib.update_artists("k", &artists, 1).unwrap();
        let row = lib.track_by_key("k").unwrap().unwrap();
        assert_eq!(row.artists, artists);
        let version: i64 = lib
            .conn
            .query_row(
                "SELECT artists_version FROM track WHERE key = 'k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn upsert_does_not_wipe_enriched_artists() {
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("k", "Song", "A feat. B")).unwrap();
        let artists = vec!["A".to_string(), "B".to_string()];
        lib.update_artists("k", &artists, 3).unwrap();
        lib.set_album_artist("k", "A").unwrap();
        lib.upsert_track(&track("k", "Song (Remaster)", "A feat. B"))
            .unwrap();
        let row = lib.track_by_key("k").unwrap().unwrap();
        assert_eq!(row.track.title, "Song (Remaster)");
        assert_eq!(row.artists, artists);
        assert_eq!(row.album_artist.as_deref(), Some("A"));
        let version: i64 = lib
            .conn
            .query_row(
                "SELECT artists_version FROM track WHERE key = 'k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn albums_group_by_album_artist() {
        let mut lib = Library::open_in_memory().unwrap();
        for (key, title, artist) in [
            ("k1", "One", "Main Artist feat. Guest A"),
            ("k2", "Two", "Main Artist feat. Guest B"),
        ] {
            let mut item = track(key, title, artist);
            item.album = Some("Joint".into());
            lib.upsert_track(&item).unwrap();
            lib.set_album_artist(key, "Main Artist").unwrap();
        }
        let albums = lib.albums().unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].album, "Joint");
        assert_eq!(albums[0].artist, "Main Artist");
        assert_eq!(albums[0].track_count, 2);
        assert_eq!(lib.album_tracks("Joint", "Main Artist").unwrap().len(), 2);
    }

    #[test]
    fn merge_track_unions_artists_and_keeps_album_artist() {
        let mut lib = Library::open_in_memory().unwrap();
        let keep = lib.upsert_track(&track("k1", "Song", "A feat. B")).unwrap();
        let drop = lib.upsert_track(&track("k2", "Song", "A feat. C")).unwrap();
        lib.update_artists("k1", &["A".into(), "B".into()], 1)
            .unwrap();
        lib.update_artists("k2", &["a".into(), "C".into()], 2)
            .unwrap();
        lib.set_album_artist("k2", "A").unwrap();
        lib.merge_track(keep, drop).unwrap();
        let row = lib.track_by_id(keep).unwrap().unwrap();
        assert_eq!(row.artists, vec!["A", "B", "C"]);
        assert_eq!(row.album_artist.as_deref(), Some("A"));
        let version: i64 = lib
            .conn
            .query_row(
                "SELECT artists_version FROM track WHERE id = ?1",
                [keep],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);
    }
}
