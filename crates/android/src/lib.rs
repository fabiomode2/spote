//! Puente UniFFI entre el core de spote y la app Android.
//!
//! Este crate no tiene UI ni lógica de producto: solo abre la librería con el
//! `SPOTE_HOME` que le pasa Kotlin y expone DTOs planos. La app puede usar el
//! mismo SQLite/FTS5, la misma paleta y el mismo contrato `TrackRef` que la TUI.

uniffi::setup_scaffolding!();

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use spote_core::artwork::{decode_image, Palette};
use spote_core::config;
use spote_core::library::{Library, SourceStatus, TrackRow};
use spote_core::util;
use spote_protocol::TrackRef;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SpoteError {
    #[error("library: {detail}")]
    Library { detail: String },
    #[error("io: {detail}")]
    Io { detail: String },
    #[error("image: {detail}")]
    Image { detail: String },
}

impl SpoteError {
    fn library(err: impl std::fmt::Display) -> Self {
        Self::Library {
            detail: err.to_string(),
        }
    }

    fn io(err: impl std::fmt::Display) -> Self {
        Self::Io {
            detail: err.to_string(),
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TrackDto {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub favorite: bool,
    pub play_count: u32,
    pub downloaded: bool,
    pub provider: Option<String>,
    pub art_path: Option<String>,
    pub path: Option<String>,
    pub added_at: i64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct LibraryStatsDto {
    pub tracks: u64,
    pub ready: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PaletteDto {
    pub background: String,
    pub surface: String,
    pub text: String,
    pub primary: String,
    pub secondary: String,
    pub accent: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AlbumDto {
    pub album: String,
    pub artist: String,
    pub track_count: i64,
    pub ready_count: i64,
    pub duration_ms: u64,
    pub art_path: Option<String>,
    pub art_url: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PlaylistDto {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub track_count: i64,
    pub cover_key: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(uniffi::Object)]
pub struct SpoteEngine {
    home: PathBuf,
    library: Mutex<Library>,
}

#[uniffi::export]
impl SpoteEngine {
    /// Abre (o crea) la librería dentro de `home`, que en Android será
    /// `context.filesDir`. El core usa `SPOTE_HOME` para config/data/cache.
    #[uniffi::constructor]
    pub fn new(home: String) -> Result<Arc<Self>, SpoteError> {
        let home = PathBuf::from(home);
        std::fs::create_dir_all(&home).map_err(SpoteError::io)?;
        std::env::set_var("SPOTE_HOME", &home);

        let db = config::db_path();
        let library = Library::open(&db).map_err(SpoteError::library)?;

        Ok(Arc::new(Self {
            home,
            library: Mutex::new(library),
        }))
    }

    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    pub fn home(&self) -> String {
        self.home.display().to_string()
    }

    pub fn stats(&self) -> Result<LibraryStatsDto, SpoteError> {
        let lib = self.lib()?;
        let tracks = lib.count().map_err(SpoteError::library)?;
        let ready = lib.ready_count().map_err(SpoteError::library)?;
        Ok(LibraryStatsDto {
            tracks: tracks.max(0) as u64,
            ready: ready.max(0) as u64,
        })
    }

    pub fn recent(&self, limit: u32) -> Result<Vec<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib.recent(limit as usize).map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    pub fn all_tracks(&self, limit: u32) -> Result<Vec<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib.all_tracks(limit as usize).map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    pub fn search(&self, query: String, limit: u32) -> Result<Vec<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib.search(&query, limit as usize).map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    pub fn albums(&self) -> Result<Vec<AlbumDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib.albums().map_err(SpoteError::library)?;
        Ok(rows
            .into_iter()
            .map(|row| AlbumDto {
                album: row.album,
                artist: row.artist,
                track_count: row.track_count,
                ready_count: row.ready_count,
                duration_ms: row.duration_ms,
                art_path: row.art_path.map(|p| p.display().to_string()),
                art_url: row.art_url,
            })
            .collect())
    }

    pub fn playlists(&self) -> Result<Vec<PlaylistDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib.playlists().map_err(SpoteError::library)?;
        Ok(rows
            .into_iter()
            .map(|row| PlaylistDto {
                id: row.id,
                name: row.name,
                description: row.description,
                track_count: row.count,
                cover_key: row.cover_key,
                cover_url: row.cover_url,
            })
            .collect())
    }

    pub fn playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib
            .playlist_tracks(playlist_id)
            .map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    pub fn toggle_favorite(&self, key: String) -> Result<bool, SpoteError> {
        let mut lib = self.lib()?;
        lib.toggle_favorite(&key).map_err(SpoteError::library)
    }

    /// Crea una playlist vacía y devuelve su id.
    pub fn create_playlist(&self, name: String) -> Result<i64, SpoteError> {
        let mut lib = self.lib()?;
        lib.create_playlist(&name).map_err(SpoteError::library)
    }

    pub fn delete_playlist(&self, playlist_id: i64) -> Result<(), SpoteError> {
        let mut lib = self.lib()?;
        lib.delete_playlist(playlist_id).map_err(SpoteError::library)
    }

    pub fn add_to_playlist(&self, playlist_id: i64, key: String) -> Result<(), SpoteError> {
        let mut lib = self.lib()?;
        lib.add_to_playlist(playlist_id, &key)
            .map_err(SpoteError::library)
    }

    pub fn remove_from_playlist(&self, playlist_id: i64, key: String) -> Result<(), SpoteError> {
        let mut lib = self.lib()?;
        lib.remove_from_playlist(playlist_id, &key)
            .map_err(SpoteError::library)
    }

    /// Indexa un fichero que Kotlin ya ha copiado al almacenamiento privado.
    /// Devuelve la clave canónica; la UI recarga la biblioteca después.
    pub fn import_local(
        &self,
        path: String,
        title: String,
        artist: String,
        album: Option<String>,
        duration_ms: u64,
        art_path: Option<String>,
    ) -> Result<String, SpoteError> {
        let key = util::fallback_key(&artist, &title, duration_ms);
        let track = TrackRef {
            key: key.clone(),
            title,
            artist,
            album,
            duration_ms,
            art_url: None,
            source_hint: None,
        };
        let mut lib = self.lib()?;
        let id = lib.upsert_track(&track).map_err(SpoteError::library)?;
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        lib.set_source(
            id,
            "local",
            None,
            Some(std::path::Path::new(&path)),
            SourceStatus::Ready,
            Some(size),
        )
        .map_err(SpoteError::library)?;
        if let Some(art) = art_path {
            lib.set_art(&key, Some(std::path::Path::new(&art)), None)
                .map_err(SpoteError::library)?;
        }
        Ok(key)
    }

    /// Inserta una pista de prueba. Sirve para el smoke test del puente antes
    /// de conectar el resolvedor y el descargador reales.
    pub fn upsert_demo(
        &self,
        title: String,
        artist: String,
        album: Option<String>,
        duration_ms: u64,
    ) -> Result<String, SpoteError> {
        let key = format!(
            "demo:{}:{}",
            artist.to_lowercase().replace(' ', "-"),
            title.to_lowercase().replace(' ', "-")
        );
        let track = TrackRef {
            key: key.clone(),
            title,
            artist,
            album,
            duration_ms,
            art_url: None,
            source_hint: None,
        };
        let mut lib = self.lib()?;
        lib.upsert_track(&track).map_err(SpoteError::library)?;
        Ok(key)
    }

    /// Extrae la paleta de una portada local. La UI de Compose la usa para
    /// teñir el tema y el fondo, igual que la TUI.
    pub fn palette(&self, image_path: String) -> Result<PaletteDto, SpoteError> {
        let bytes = std::fs::read(&image_path).map_err(SpoteError::io)?;
        let image = decode_image(&bytes).map_err(|err| SpoteError::Image {
            detail: err.to_string(),
        })?;
        let palette = Palette::from_image(&image);
        Ok(PaletteDto {
            background: hex(palette.bg),
            surface: hex(palette.surface),
            text: hex(palette.text),
            primary: hex(palette.primary),
            secondary: hex(palette.secondary),
            accent: hex(palette.accent),
        })
    }
}

impl SpoteEngine {
    fn lib(&self) -> Result<MutexGuard<'_, Library>, SpoteError> {
        self.library.lock().map_err(|_| SpoteError::Library {
            detail: "la librería quedó bloqueada".to_string(),
        })
    }
}

impl From<&TrackRow> for TrackDto {
    fn from(row: &TrackRow) -> Self {
        Self {
            key: row.track.key.clone(),
            title: row.track.title.clone(),
            artist: row.track.artist.clone(),
            album: row.track.album.clone(),
            duration_ms: row.track.duration_ms,
            favorite: row.favorite,
            play_count: row.play_count,
            downloaded: row.playable(),
            provider: row.provider.clone(),
            art_path: row.art_path.as_ref().map(|path| path.display().to_string()),
            path: row.path.as_ref().map(|path| path.display().to_string()),
            added_at: row.added_at,
        }
    }
}

fn hex((r, g, b): spote_core::artwork::Rgb) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}
