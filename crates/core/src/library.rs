//! Librería local: SQLite con FTS5. Es la única fuente de verdad del cliente.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
}

impl TrackRow {
    pub fn playable(&self) -> bool {
        self.path.as_ref().is_some_and(|p| p.exists())
    }
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

pub struct Library {
    conn: Connection,
    path: PathBuf,
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
        };
        lib.configure()?;
        lib.migrate()?;
        Ok(lib)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mut lib = Self {
            conn,
            path: PathBuf::from(":memory:"),
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
              last_played INTEGER
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
              fetched_at INTEGER NOT NULL
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
            CREATE INDEX IF NOT EXISTS idx_track_artist ON track(artist);
            CREATE INDEX IF NOT EXISTS idx_track_added ON track(added_at DESC);
            CREATE INDEX IF NOT EXISTS idx_source_status ON source(status);
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
        Ok(())
    }

    /// Inserta o actualiza una canción canónica. Devuelve su id local.
    pub fn upsert_track(&mut self, track: &TrackRef) -> Result<i64> {
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
              art_url = coalesce(excluded.art_url, track.art_url)
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

    pub fn set_status(&mut self, track_id: i64, provider: &str, status: SourceStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE source SET status = ?1 WHERE track_id = ?2 AND provider = ?3",
            params![status.as_str(), track_id, provider],
        )?;
        Ok(())
    }

    pub fn track_by_key(&self, key: &str) -> Result<Option<TrackRow>> {
        let mut stmt = self.conn.prepare_cached(&format!("{SELECT_TRACK} WHERE t.key = ?1"))?;
        let row = stmt.query_row(params![key], row_to_track).optional()?;
        Ok(row)
    }

    pub fn track_by_id(&self, id: i64) -> Result<Option<TrackRow>> {
        let mut stmt = self.conn.prepare_cached(&format!("{SELECT_TRACK} WHERE t.id = ?1"))?;
        Ok(stmt.query_row(params![id], row_to_track).optional()?)
    }

    pub fn local_path(&self, key: &str) -> Option<PathBuf> {
        self.track_by_key(key)
            .ok()
            .flatten()
            .and_then(|row| row.path)
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
            .filter(|token| !token.is_empty())
            .collect();
        if tokens.is_empty() {
            return self.recent(limit);
        }
        let fts_query = tokens
            .iter()
            .map(|token| format!("\"{}\"*", token.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" ");
        let sql = format!(
            "{SELECT_TRACK} JOIN track_fts f ON f.rowid = t.id
             WHERE track_fts MATCH ?1 ORDER BY rank LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![fts_query, limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            // FTS5 no hace prefijos con acentos raros: fallback a LIKE.
            let like = format!("%{}%", query.trim());
            return self.like_search(&like, limit);
        }
        Ok(rows)
    }

    fn like_search(&self, pattern: &str, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} WHERE t.title LIKE ?1 OR t.artist LIKE ?1 OR t.album LIKE ?1
             ORDER BY t.artist LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!("{SELECT_TRACK} ORDER BY t.added_at DESC, t.id DESC LIMIT ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Orden alfabético, con las descargadas primero.
    pub fn all_tracks(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} ORDER BY (s.status = 'ready') DESC, t.artist COLLATE NOCASE, t.title LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn playable(&self, limit: usize) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} WHERE s.status = 'ready' AND s.path IS NOT NULL
             ORDER BY t.play_count DESC, t.artist LIMIT ?1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn favorites(&self) -> Result<Vec<TrackRow>> {
        let sql = format!("{SELECT_TRACK} WHERE t.favorite = 1 ORDER BY t.artist");
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
            .query_row("SELECT favorite FROM track WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?;
        Ok(value.unwrap_or(0) == 1)
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT count(*) FROM track", [], |r| r.get(0))?)
    }

    pub fn ready_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM source WHERE status = 'ready'",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn record_play(&mut self, key: &str, ms_played: u64, completed: bool) -> Result<()> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM track WHERE key = ?1", params![key], |r| r.get(0))
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
    pub fn lyrics(&self, key: &str) -> Result<Option<(String, bool, i64)>> {
        let row = self
            .conn
            .query_row(
                "SELECT l.lrc, l.synced, l.offset_ms FROM lyrics l
                 JOIN track t ON t.id = l.track_id WHERE t.key = ?1",
                params![key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0, r.get::<_, i64>(2)?)),
            )
            .optional()?;
        Ok(row)
    }

    pub fn save_lyrics(&mut self, key: &str, lrc: &str, synced: bool, source: &str) -> Result<()> {
        let Some(id) = self
            .conn
            .query_row("SELECT id FROM track WHERE key = ?1", params![key], |r| r.get::<_, i64>(0))
            .optional()?
        else {
            return Ok(());
        };
        self.conn.execute(
            r#"
            INSERT INTO lyrics(track_id, lrc, synced, source, fetched_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(track_id) DO UPDATE SET
              lrc = excluded.lrc, synced = excluded.synced,
              source = excluded.source, fetched_at = excluded.fetched_at
            "#,
            params![id, lrc, synced as i64, source, now_ms()],
        )?;
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
        Ok(self.lyrics(key)?.map(|(_, _, offset)| offset).unwrap_or(0))
    }

    // --------------------------------------------------------------- arte
    pub fn art_path(&self, key: &str) -> Result<Option<PathBuf>> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT art_path FROM track WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(value.map(PathBuf::from))
    }

    pub fn art_url(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT art_url FROM track WHERE key = ?1", params![key], |r| r.get(0))
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
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, (SELECT count(*) FROM playlist_item i WHERE i.playlist_id = p.id)
             FROM playlist p ORDER BY p.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    count: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn create_playlist(&mut self, name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO playlist(name, created_at) VALUES (?1, ?2)",
            params![name, now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackRow>> {
        let sql = format!(
            "{SELECT_TRACK} JOIN playlist_item i ON i.track_id = t.id
             WHERE i.playlist_id = ?1 ORDER BY i.position"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![playlist_id], row_to_track)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn add_to_playlist(&mut self, playlist_id: i64, key: &str) -> Result<()> {
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
            .query_row("SELECT value FROM setting WHERE key = ?1", params![key], |r| r.get(0))
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

    /// Borra canciones sin fichero ni descarga en curso.
    pub fn prune_missing(&mut self) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM source WHERE status = 'missing'",
            [],
        )?;
        Ok(removed)
    }
}

const SELECT_TRACK: &str = r#"
SELECT t.id, t.key, t.title, t.artist, t.album, t.duration_ms, t.art_url, t.art_path,
       s.path, s.status, s.provider, t.favorite, t.play_count
FROM track t
LEFT JOIN source s ON s.track_id = t.id
  AND s.id = (SELECT id FROM source x WHERE x.track_id = t.id
              ORDER BY (x.status = 'ready') DESC, x.id DESC LIMIT 1)
"#;

fn row_to_track(row: &Row<'_>) -> rusqlite::Result<TrackRow> {
    let art_path: Option<String> = row.get(7)?;
    let path: Option<String> = row.get(8)?;
    let status: Option<String> = row.get(9)?;
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
    })
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
    fn upsert_is_idempotent_and_updates_fields() {
        let mut lib = Library::open_in_memory().unwrap();
        let id1 = lib.upsert_track(&track("sha1:a", "One", "Artist")).unwrap();
        let id2 = lib.upsert_track(&track("sha1:a", "One (Remaster)", "Artist")).unwrap();
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
        lib.upsert_track(&track("sha1:c", "Nude", "Radiohead")).unwrap();
        assert_eq!(lib.search("arctic", 10).unwrap().len(), 2);
        assert_eq!(lib.search("505", 10).unwrap().len(), 1);
        assert_eq!(lib.search("radiohead", 10).unwrap().len(), 1);
        // Sin tokens útiles se comporta como "recientes", no como error.
        assert_eq!(lib.search("!!!", 10).unwrap().len(), 3);
        assert_eq!(lib.search("", 10).unwrap().len(), 3);
    }

    #[test]
    fn sources_prefer_ready_path() {
        let mut lib = Library::open_in_memory().unwrap();
        let id = lib.upsert_track(&track("sha1:a", "One", "Artist")).unwrap();
        lib.set_source(id, "youtube", Some("vid"), None, SourceStatus::Pending, None)
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
        lib.save_lyrics("sha1:a", "[00:01.00]hola", true, "lrclib")
            .unwrap();
        let (lrc, synced, _) = lib.lyrics("sha1:a").unwrap().unwrap();
        assert_eq!(lrc, "[00:01.00]hola");
        assert!(synced);
        lib.set_lyrics_offset("sha1:a", 250).unwrap();
        assert_eq!(lib.lyrics_offset("sha1:a").unwrap(), 250);
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
}
