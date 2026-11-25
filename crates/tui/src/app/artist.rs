//! Perfil de artista (Spotify/Deezer) y backfill de metadatos/portada.

use super::*;

impl App {
    /// Abre el perfil por nombre. Lo usan los nombres clicables de toda la UI.
    pub(super) fn open_artist_by_name(&mut self, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let previous = self.snapshot();
        let title = name.clone();
        self.artist = None;
        self.artist_art = None;
        self.artist_selected = 0;
        self.artist_loading = true;
        self.artist_status = format!("buscando {name}…");
        self.artist_request = name.clone();
        self.detail = Some(Detail::Artist);
        self.artist_album_scroll = 0;
        self.overlay = Overlay::None;
        self.meta_worker.artist(name);
        self.push_doc_page(DocKind::Artist, title, previous);
    }

    /// Abre un artista de la búsqueda por su id de proveedor.
    pub(super) fn open_artist_summary(&mut self, artist: &spote_core::spotify_api::ArtistSummary) {
        let previous = self.snapshot();
        let title = artist.name.clone();
        self.artist = None;
        self.artist_art = None;
        self.artist_selected = 0;
        self.artist_loading = true;
        self.artist_status = format!("buscando {}…", artist.name);
        self.artist_request = artist.name.clone();
        self.detail = Some(Detail::Artist);
        self.artist_album_scroll = 0;
        self.overlay = Overlay::None;
        self.meta_worker.artist_by_id(
            artist.provider.clone(),
            artist.id.clone(),
            artist.name.clone(),
        );
        self.push_doc_page(DocKind::Artist, title, previous);
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
                Tab::Library => self.selected_row().map(|row| crate::app::credits::row_primary_artist(&row)),
                Tab::Albums => self
                    .selected_row()
                    .map(|row| crate::app::credits::row_primary_artist(&row))
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
                    .or_else(|| Some(self.now_primary_artist())),
                _ => Some(self.now_primary_artist()),
            }
            .or_else(|| Some(self.now_primary_artist()))
            .unwrap_or_default()
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            self.toast("no hay artista seleccionado", ToastKind::Warn);
            return;
        }
        let previous = self.snapshot();
        let title = name.clone();
        self.artist = None;
        self.artist_art = None;
        self.artist_selected = 0;
        self.artist_loading = true;
        self.artist_status = format!("buscando {name}…");
        self.artist_request = name.clone();
        self.detail = Some(Detail::Artist);
        self.artist_album_scroll = 0;
        self.overlay = Overlay::None;
        self.meta_worker.artist(name);
        self.push_doc_page(DocKind::Artist, title, previous);
    }

    /// Enruta el teclado al panel de detalle activo. Devuelve `false` para
    /// dejar pasar atajos globales (espacio, tab, q…).
    pub fn handle_detail_key(&mut self, key: KeyEvent) -> bool {
        match self.detail {
            Some(Detail::Artist) => self.handle_artist_key(key),
            Some(Detail::Album) => self.handle_album_key(key),
            None => false,
        }
    }

    pub(super) fn handle_artist_key(&mut self, key: KeyEvent) -> bool {
        let len = self
            .artist
            .as_ref()
            .map(|profile| profile.top_tracks.len())
            .unwrap_or(0);
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.close_detail();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.artist_selected = self.artist_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if len > 0 {
                    self.artist_selected = (self.artist_selected + 1).min(len - 1);
                }
            }
            KeyCode::Left | KeyCode::Char('[') => self.scroll_artist_albums(-1),
            KeyCode::Right | KeyCode::Char(']') => self.scroll_artist_albums(1),
            KeyCode::Enter => self.play_artist_selected(),
            KeyCode::Char('a') => {
                if let Some(profile) = self.artist.clone() {
                    if let Some(track) = profile.top_tracks.get(self.artist_selected) {
                        self.queue_track(spotify_track_ref(track), None);
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// Mueve la tira horizontal de discografía. Cuenta las tarjetas visibles
    /// para no dejar huecos al final.
    pub fn scroll_artist_albums(&mut self, delta: i32) {
        let total = self
            .artist
            .as_ref()
            .map(|profile| profile.albums.len())
            .unwrap_or(0);
        if total == 0 {
            return;
        }
        let visible = self.detail_album_visible.max(1);
        let max = total.saturating_sub(visible);
        let next = (self.artist_album_scroll as i32 + delta).clamp(0, max as i32) as usize;
        if next != self.artist_album_scroll {
            self.artist_album_scroll = next;
            self.needs_redraw = true;
        }
    }

    /// Cierra el panel persistente de detalle sin tocar la pestaña actual.
    pub fn close_detail(&mut self) {
        // Si la ficha vive en una pestaña de documento, “volver” es un pop de
        // historial. Solo se limpia a mano cuando no hay pila (flujo antiguo).
        if self.nav_back() {
            return;
        }
        self.detail = None;
        self.overlay = Overlay::None;
        self.detail_albums_area = Rect::default();
        self.album_page = None;
        self.album_page_loading = false;
        self.album_page_status.clear();
        self.needs_redraw = true;
    }

    pub(super) fn play_artist_selected(&mut self) {
        let Some(profile) = self.artist.clone() else { return };
        let Some(track) = profile.top_tracks.get(self.artist_selected) else {
            return;
        };
        let entries: Vec<QueueEntry> = profile
            .top_tracks
            .iter()
            .map(|track| {
                let track = spotify_track_ref(track);
                let path = self
                    .lib
                    .local_path(&track.key)
                    .filter(|path| path.exists());
                QueueEntry { track, path }
            })
            .collect();
        self.set_context(entries, self.artist_selected);
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
