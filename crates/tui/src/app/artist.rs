//! Perfil de artista (Spotify/Deezer) y backfill de metadatos/portada.

use super::*;

impl App {
    /// Abre el perfil por nombre. Lo usan los nombres clicables de toda la UI.
    pub(super) fn open_artist_by_name(&mut self, name: &str) {
        let name = name.split(',').next().unwrap_or(name).trim().to_string();
        if name.is_empty() {
            return;
        }
        self.artist = None;
        self.artist_art = None;
        self.artist_selected = 0;
        self.artist_loading = true;
        self.artist_status = format!("buscando {name}…");
        self.meta_worker.artist(name);
        self.overlay = Overlay::Artist;
    }

    /// Abre un artista de la búsqueda por su id de proveedor.
    pub(super) fn open_artist_summary(&mut self, artist: &spote_core::spotify_api::ArtistSummary) {
        self.artist = None;
        self.artist_art = None;
        self.artist_selected = 0;
        self.artist_loading = true;
        self.artist_status = format!("buscando {}…", artist.name);
        self.meta_worker.artist_by_id(
            artist.provider.clone(),
            artist.id.clone(),
            artist.name.clone(),
        );
        self.overlay = Overlay::Artist;
    }

    /// Abre el perfil del artista de la pista sonando o de la seleccionada.
    pub(super) fn open_artist(&mut self) {
        let name = if let Some(menu) = &self.context {
            menu.artist.clone()
        } else {
            match self.tab {
                Tab::Search => match self.search_entry(self.search.selected) {
                    Some(SearchEntry::Track(index)) => {
                        self.search.results.get(index).map(|item| item.artist.clone())
                    }
                    Some(SearchEntry::Album(index)) => {
                        self.search.albums.get(index).map(|album| album.artist.clone())
                    }
                    Some(SearchEntry::Artist(index)) => {
                        self.search.artists.get(index).map(|artist| artist.name.clone())
                    }
                    None => None,
                },
                Tab::Library => self.selected_row().map(|row| row.track.artist.clone()),
                Tab::Albums => self
                    .selected_row()
                    .map(|row| row.track.artist.clone())
                    .or_else(|| {
                        self.album_open
                            .as_ref()
                            .map(|album| album.artist.clone())
                    })
                    .or_else(|| {
                        self.albums
                            .get(self.album_selected)
                            .map(|album| album.artist.clone())
                    }),
                Tab::Room => self
                    .room_state
                    .as_ref()
                    .and_then(|state| state.track.as_ref())
                    .map(|track| track.artist.clone())
                    .or_else(|| self.now.track.as_ref().map(|track| track.artist.clone())),
                _ => self.now.track.as_ref().map(|track| track.artist.clone()),
            }
            .or_else(|| self.now.track.as_ref().map(|track| track.artist.clone()))
            .unwrap_or_default()
        };
        // Varios artistas separados por coma: el perfil es el primero.
        let name = name.split(',').next().unwrap_or(&name).trim().to_string();
        if name.is_empty() {
            self.toast("no hay artista seleccionado", ToastKind::Warn);
            return;
        }
        self.artist = None;
        self.artist_art = None;
        self.artist_selected = 0;
        self.artist_loading = true;
        self.artist_status = format!("buscando {name}…");
        self.meta_worker.artist(name);
        self.overlay = Overlay::Artist;
    }

    pub(super) fn handle_artist_key(&mut self, key: KeyEvent) {
        let len = self
            .artist
            .as_ref()
            .map(|profile| profile.top_tracks.len())
            .unwrap_or(0);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
            KeyCode::Up | KeyCode::Char('k') => {
                self.artist_selected = self.artist_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.artist_selected = (self.artist_selected + 1).min(len - 1);
                }
            }
            KeyCode::Enter => self.play_artist_selected(),
            KeyCode::Char('a') => {
                if let Some(profile) = self.artist.clone() {
                    if let Some(track) = profile.top_tracks.get(self.artist_selected) {
                        self.queue_track(spotify_track_ref(track), None);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn play_artist_selected(&mut self) {
        let Some(profile) = self.artist.clone() else { return };
        let Some(track) = profile.top_tracks.get(self.artist_selected) else {
            return;
        };
        self.overlay = Overlay::None;
        self.play_external_track(spotify_track_ref(track));
    }

    /// Refresca la biblioteca sin vaciarla si SQLite está ocupada. Antes un
    /// error puntual se convertía en `Vec::default()` y la vista se quedaba en
    /// blanco hasta cambiar de pestaña.
    pub(super) fn refresh_library(&mut self) {
        // `refresh_view` ya conserva la vista anterior solo si hubo un error
        // real; un filtro vacío debe quedarse vacío. Antes `R` rescataba las
        // canciones de otro filtro y la biblioteca parecía aleatoria.
        self.refresh_view();
        self.thumbs
            .retain(|(key, _, _), _| self.view.iter().any(|row| &row.track.key == key));
        self.thumb_requests
            .retain(|(key, _, _)| self.view.iter().any(|row| &row.track.key == key));
    }

    /// Pide a Spotify los metadatos/portada de una pista concreta.
    pub(super) fn request_spotify_metadata_for_key(&mut self, key: &str) {
        self.request_spotify_metadata_forced(key, false);
    }

    pub(super) fn request_spotify_metadata_forced(&mut self, key: &str, force: bool) {
        if self.metadata_requests.contains(key) {
            return;
        }
        if !force && self.metadata_tried.contains(key) {
            return;
        }
        let Ok(Some(row)) = self.lib.track_by_key(key) else {
            return;
        };
        self.metadata_tried.insert(key.to_string());
        if force {
            self.metadata_manual.insert(key.to_string());
        }
        self.metadata_requests.insert(key.to_string());
        self.meta_worker.metadata(
            key.to_string(),
            row.track.artist.clone(),
            row.track.title.clone(),
        );
        // El backfill automático no debe llenar la pantalla de toasts.
        if force {
            self.toast("buscando metadatos en Spotify…", ToastKind::Info);
        }
    }

    /// Si una fila no tiene portada, la pide sola (una vez). Se llama al pintar
    /// la lista: así se rellenan las canciones viejas sin tocar nada.
    pub fn ensure_missing_art(&mut self, row: &TrackRow) {
        if row.art_path.is_some() || row.track.art_url.is_some() {
            return;
        }
        if self.metadata_requests.contains(&row.track.key)
            || self.metadata_tried.contains(&row.track.key)
        {
            return;
        }
        // Solo unas pocas peticiones automáticas a la vez.
        if self.metadata_requests.len() >= 3 {
            return;
        }
        self.request_spotify_metadata_for_key(&row.track.key);
    }
}

pub(super) fn spotify_track_ref(track: &spote_core::spotify_api::SpotifyTrack) -> TrackRef {
    let key = track
        .isrc
        .as_ref()
        .map(|isrc| format!("isrc:{}", isrc.to_lowercase()))
        .unwrap_or_else(|| util::fallback_key(&track.artists, &track.title, track.duration_ms));
    TrackRef {
        key,
        title: track.title.clone(),
        artist: track.artists.clone(),
        album: Some(track.album.clone()).filter(|album| !album.is_empty()),
        duration_ms: track.duration_ms,
        art_url: track.art_url.clone(),
        source_hint: Some(spote_protocol::SourceHint {
            provider: track.provider.clone(),
            id: track.id.clone(),
            duration_ms: track.duration_ms,
        }),
    }
}
