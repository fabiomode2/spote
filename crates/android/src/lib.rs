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
use spote_core::spotify_api;
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
    #[error("catalog: {detail}")]
    Catalog { detail: String },
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

    fn catalog(err: impl std::fmt::Display) -> Self {
        Self::Catalog {
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
    pub art_url: Option<String>,
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
    pub surface_alt: String,
    pub bg_deep: String,
    pub text: String,
    pub muted: String,
    pub border: String,
    pub primary: String,
    pub secondary: String,
    pub accent: String,
    pub ok: String,
    pub error: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogTrackDto {
    /// Clave canónica (`isrc:…` o `sha1:…`), igual que en la TUI.
    pub key: String,
    pub provider: String,
    /// Id del proveedor (`deezer` numérico o `dz:…`).
    pub provider_id: String,
    pub title: String,
    pub artists: String,
    pub artist_list: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub art_url: Option<String>,
    pub isrc: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogAlbumDto {
    pub provider: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub release_year: Option<u32>,
    pub track_count: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogArtistDto {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub followers: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogSearchDto {
    pub provider: String,
    pub tracks: Vec<CatalogTrackDto>,
    pub albums: Vec<CatalogAlbumDto>,
    pub artists: Vec<CatalogArtistDto>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogAlbumInfoDto {
    pub provider: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub release_date: Option<String>,
    pub year: Option<u32>,
    pub tracks: Vec<CatalogTrackDto>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CatalogArtistProfileDto {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub genres: Vec<String>,
    pub followers: u64,
    pub popularity: u32,
    pub top_tracks: Vec<CatalogTrackDto>,
    pub albums: Vec<CatalogAlbumDto>,
    pub bio: Option<String>,
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
        let rows = lib
            .all_tracks(limit as usize)
            .map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    pub fn search(&self, query: String, limit: u32) -> Result<Vec<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib
            .search(&query, limit as usize)
            .map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    /// Búsqueda mixta en el catálogo (Spotify con respaldo Deezer), igual que
    /// la TUI. Devuelve pistas, álbumes y artistas reales, no derivados.
    pub fn search_catalog(
        &self,
        query: String,
        limit: u32,
    ) -> Result<CatalogSearchDto, SpoteError> {
        let result =
            spotify_api::search_all(&query, limit as usize).map_err(SpoteError::catalog)?;
        Ok(CatalogSearchDto {
            provider: result.provider,
            tracks: result.tracks.iter().map(catalog_track_dto).collect(),
            albums: result.albums.iter().map(catalog_album_dto).collect(),
            artists: result.artists.iter().map(catalog_artist_dto).collect(),
        })
    }

    /// Ficha completa de un álbum del catálogo, con su tracklist.
    pub fn album_info(
        &self,
        provider: String,
        id: String,
    ) -> Result<CatalogAlbumInfoDto, SpoteError> {
        let info = spotify_api::album(&provider, &id).map_err(SpoteError::catalog)?;
        Ok(catalog_album_info_dto(&info))
    }

    /// Perfil de artista por nombre, como el que abre la TUI desde cualquier
    /// crédito. Incluye top tracks y discografía.
    pub fn artist_profile(&self, name: String) -> Result<CatalogArtistProfileDto, SpoteError> {
        let profile = spotify_api::artist(&name).map_err(SpoteError::catalog)?;
        Ok(catalog_artist_profile_dto(&profile))
    }

    /// Perfil de artista por id de proveedor (el que viene de la búsqueda).
    pub fn artist_profile_by_id(
        &self,
        provider: String,
        id: String,
    ) -> Result<CatalogArtistProfileDto, SpoteError> {
        let profile = spotify_api::artist_by_id(&provider, &id).map_err(SpoteError::catalog)?;
        Ok(catalog_artist_profile_dto(&profile))
    }

    pub fn artist_albums(
        &self,
        provider: String,
        id: String,
    ) -> Result<Vec<CatalogAlbumDto>, SpoteError> {
        let albums = spotify_api::albums_of(&provider, &id).map_err(SpoteError::catalog)?;
        Ok(albums.iter().map(catalog_album_dto).collect())
    }

    /// Busca un álbum por artista + título y devuelve su ficha completa.
    /// Es el mismo emparejamiento que usa la TUI al abrir un álbum local.
    pub fn find_album(
        &self,
        album: String,
        artist: String,
    ) -> Result<Option<CatalogAlbumInfoDto>, SpoteError> {
        let query = format!("{artist} {album}");
        let results = spotify_api::search_all(&query, 8).map_err(SpoteError::catalog)?;
        let wanted = util::normalize(&album);
        let candidate = results
            .albums
            .into_iter()
            .filter(|item| {
                let got = util::normalize(&item.title);
                got == wanted || got.contains(&wanted) || wanted.contains(&got)
            })
            .max_by_key(|item| {
                let artist_match =
                    util::normalize(&item.artist).contains(&util::normalize(&artist));
                (artist_match, item.track_count)
            });
        let Some(candidate) = candidate else {
            return Ok(None);
        };
        let info =
            spotify_api::album(&candidate.provider, &candidate.id).map_err(SpoteError::catalog)?;
        Ok(Some(catalog_album_info_dto(&info)))
    }

    /// Metadatos de una pista local: elige la coincidencia que más se parece
    /// en título, artista y duración, no simplemente la primera.
    pub fn find_track_metadata(
        &self,
        artist: String,
        title: String,
    ) -> Result<Option<CatalogTrackDto>, SpoteError> {
        let query = format!("{artist} {title}");
        let tracks = spotify_api::search(&query, 8).map_err(SpoteError::catalog)?;
        let wanted_title = util::normalize(&title);
        let wanted_artist = util::normalize(&artist);
        let best = tracks
            .into_iter()
            .filter_map(|track| {
                let got_title = util::normalize(&track.title);
                let got_artist = util::normalize(&track.artists);
                let mut score = 0i32;
                if got_title == wanted_title {
                    score += 60;
                } else if got_title.contains(&wanted_title) || wanted_title.contains(&got_title) {
                    score += 30;
                }
                if got_artist == wanted_artist {
                    score += 40;
                } else if got_artist.contains(&wanted_artist) || wanted_artist.contains(&got_artist)
                {
                    score += 20;
                }
                if score >= 30 {
                    Some((score, track))
                } else {
                    None
                }
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, track)| track);
        Ok(best.as_ref().map(catalog_track_dto))
    }

    /// Tracklist real de un álbum local, agrupado por el core (incluye
    /// colaboraciones y respeta la identidad `album_artist`).
    pub fn album_tracks(&self, album: String, artist: String) -> Result<Vec<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib
            .album_tracks(&album, &artist)
            .map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    /// Busca la copia local de una pista del catálogo por metadatos. Evita
    /// duplicar una canción ya descargada de otra fuente.
    pub fn find_track_by_meta(
        &self,
        artist: String,
        title: String,
        duration_ms: u64,
    ) -> Result<Option<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let row = lib
            .find_by_meta(&artist, &title, duration_ms)
            .map_err(SpoteError::library)?;
        Ok(row.as_ref().map(TrackDto::from))
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

    /// Pistas sin portada para el backfill de metadatos.
    pub fn tracks_without_art(&self, limit: u32) -> Result<Vec<TrackDto>, SpoteError> {
        let lib = self.lib()?;
        let rows = lib
            .tracks_without_art(limit as usize)
            .map_err(SpoteError::library)?;
        Ok(rows.iter().map(TrackDto::from).collect())
    }

    /// Asocia una portada local y/o URL a una pista existente.
    pub fn set_track_art(
        &self,
        key: String,
        art_path: Option<String>,
        art_url: Option<String>,
    ) -> Result<(), SpoteError> {
        let mut lib = self.lib()?;
        lib.set_art(
            &key,
            art_path.as_deref().map(std::path::Path::new),
            art_url.as_deref(),
        )
        .map_err(SpoteError::library)
    }

    /// Completa metadatos sin pisar valores mejores con vacíos.
    pub fn update_track_metadata(
        &self,
        key: String,
        artist: String,
        title: String,
        album: Option<String>,
        duration_ms: u64,
        art_url: Option<String>,
    ) -> Result<(), SpoteError> {
        let mut lib = self.lib()?;
        lib.update_metadata(
            &key,
            &artist,
            &title,
            album.as_deref(),
            duration_ms,
            art_url.as_deref(),
        )
        .map_err(SpoteError::library)
    }

    /// Número de parejas de canciones duplicadas (mismo título/artista/duración).
    pub fn duplicate_count(&self) -> Result<i64, SpoteError> {
        let lib = self.lib()?;
        lib.duplicate_count().map_err(SpoteError::library)
    }

    /// Fusiona duplicados conservando la copia con audio y moviendo listas/fuentes.
    pub fn merge_duplicates(&self) -> Result<u32, SpoteError> {
        let mut lib = self.lib()?;
        let merged = lib.merge_duplicates().map_err(SpoteError::library)?;
        Ok(merged as u32)
    }

    /// Copia de seguridad consistente antes de operaciones destructivas.
    pub fn backup_library(&self, path: String) -> Result<(), SpoteError> {
        let lib = self.lib()?;
        lib.backup_to(std::path::Path::new(&path))
            .map_err(SpoteError::library)
    }

    /// Crea una playlist vacía y devuelve su id.
    pub fn create_playlist(&self, name: String) -> Result<i64, SpoteError> {
        let mut lib = self.lib()?;
        lib.create_playlist(&name).map_err(SpoteError::library)
    }

    pub fn delete_playlist(&self, playlist_id: i64) -> Result<(), SpoteError> {
        let mut lib = self.lib()?;
        lib.delete_playlist(playlist_id)
            .map_err(SpoteError::library)
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

    /// Indexa un fichero conservando la clave canónica del catálogo. Así una
    /// descarga mantiene la misma identidad que la pista remota y el álbum
    /// sigue marcándola como disponible.
    pub fn import_local_with_key(
        &self,
        key: String,
        path: String,
        title: String,
        artist: String,
        album: Option<String>,
        duration_ms: u64,
        art_path: Option<String>,
    ) -> Result<String, SpoteError> {
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

    /// Añade a la biblioteca una pista que se ha escuchado en streaming, sin
    /// audio local. Queda como fuente pendiente para poder resolverla al
    /// reproducir y para que aparezca en Álbumes/Artistas.
    pub fn remember_track(
        &self,
        key: String,
        title: String,
        artist: String,
        album: Option<String>,
        duration_ms: u64,
        art_url: Option<String>,
        provider: String,
        provider_id: String,
    ) -> Result<(), SpoteError> {
        let track = TrackRef {
            key: key.clone(),
            title,
            artist,
            album,
            duration_ms,
            art_url: art_url.clone(),
            source_hint: Some(spote_protocol::SourceHint {
                provider: provider.clone(),
                id: provider_id.clone(),
                duration_ms,
            }),
        };
        let mut lib = self.lib()?;
        let id = lib.upsert_track(&track).map_err(SpoteError::library)?;
        lib.set_source(
            id,
            &provider,
            Some(&provider_id),
            None,
            SourceStatus::Pending,
            None,
        )
        .map_err(SpoteError::library)?;
        if let Some(url) = art_url {
            lib.set_art(&key, None, Some(&url))
                .map_err(SpoteError::library)?;
        }
        Ok(())
    }

    /// Quita una pista de la biblioteca sin borrar el archivo ni el historial.
    pub fn remove_from_library(&self, key: String) -> Result<(), SpoteError> {
        let mut lib = self.lib()?;
        lib.remove_from_library(&key)
            .map_err(SpoteError::library)
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
            surface_alt: hex(palette.surface_alt),
            bg_deep: hex(palette.bg_deep),
            text: hex(palette.text),
            muted: hex(palette.muted),
            border: hex(palette.border),
            primary: hex(palette.primary),
            secondary: hex(palette.secondary),
            accent: hex(palette.accent),
            ok: hex(palette.ok),
            error: hex(palette.error),
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

fn catalog_track_dto(track: &spotify_api::SpotifyTrack) -> CatalogTrackDto {
    let key = track
        .isrc
        .as_ref()
        .map(|isrc| format!("isrc:{}", isrc.to_lowercase()))
        .unwrap_or_else(|| util::fallback_key(&track.artists, &track.title, track.duration_ms));
    CatalogTrackDto {
        key,
        provider: track.provider.clone(),
        provider_id: track.id.clone(),
        title: track.title.clone(),
        artists: track.artists.clone(),
        artist_list: split_artists(&track.artists),
        album: Some(track.album.clone()).filter(|album| !album.is_empty()),
        duration_ms: track.duration_ms,
        art_url: track.art_url.clone(),
        isrc: track.isrc.clone(),
    }
}

fn split_artists(artists: &str) -> Vec<String> {
    artists
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn catalog_album_dto(album: &spotify_api::AlbumSummary) -> CatalogAlbumDto {
    CatalogAlbumDto {
        provider: album.provider.clone(),
        id: album.id.clone(),
        title: album.title.clone(),
        artist: album.artist.clone(),
        cover_url: album.cover_url.clone(),
        release_year: album.release_year.map(u32::from),
        track_count: album.track_count,
    }
}

fn catalog_artist_dto(artist: &spotify_api::ArtistSummary) -> CatalogArtistDto {
    CatalogArtistDto {
        provider: artist.provider.clone(),
        id: artist.id.clone(),
        name: artist.name.clone(),
        image_url: artist.image_url.clone(),
        followers: artist.followers,
    }
}

fn catalog_album_info_dto(info: &spotify_api::AlbumInfo) -> CatalogAlbumInfoDto {
    CatalogAlbumInfoDto {
        provider: info.provider.clone(),
        id: info.id.clone(),
        title: info.title.clone(),
        artist: info.artist.clone(),
        cover_url: info.cover_url.clone(),
        release_date: info.release_date.clone(),
        year: info
            .release_date
            .as_deref()
            .and_then(|date| date.trim().get(..4))
            .and_then(|year| year.parse::<u32>().ok()),
        tracks: info.tracks.iter().map(catalog_track_dto).collect(),
    }
}

fn catalog_artist_profile_dto(profile: &spotify_api::ArtistProfile) -> CatalogArtistProfileDto {
    CatalogArtistProfileDto {
        provider: profile.provider.clone(),
        id: profile.id.clone(),
        name: profile.name.clone(),
        image_url: profile.image_url.clone(),
        genres: profile.genres.clone(),
        followers: profile.followers,
        popularity: profile.popularity,
        top_tracks: profile.top_tracks.iter().map(catalog_track_dto).collect(),
        albums: profile.albums.iter().map(catalog_album_dto).collect(),
        bio: profile.bio.clone(),
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
            art_url: row.track.art_url.clone(),
            path: row.path.as_ref().map(|path| path.display().to_string()),
            added_at: row.added_at,
        }
    }
}

fn hex((r, g, b): spote_core::artwork::Rgb) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}
