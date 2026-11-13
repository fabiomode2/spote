//! Pantalla de álbumes: agrupa la biblioteca por artista+álbum, abre el
//! tracklist y permite bajar el álbum entero o guardarlo como lista.

use super::*;
use spote_core::library::AlbumRow;

impl App {
    pub(super) fn open_album_selected(&mut self) {
        let Some(album) = self.albums.get(self.album_selected).cloned() else {
            return;
        };
        self.album_open = Some(album);
        self.selected = 0;
        self.tab = Tab::Albums;
        self.refresh_view();
    }

    /// Abre el álbum por nombre desde cualquier nombre clicable de la UI.
    pub(super) fn open_album_by_name(&mut self, album: &str, artist: &str) {
        // La pestaña de álbumes ya no existe: en «ahora» o biblioteca
        // `self.albums` puede estar vacío. Se consulta la agrupación actual.
        let candidates = if self.albums.is_empty() {
            self.lib.albums().unwrap_or_default()
        } else {
            self.albums.clone()
        };
        let found = candidates
            .iter()
            .find(|row| {
                row.album.eq_ignore_ascii_case(album)
                    && (artist.is_empty() || row.artist.eq_ignore_ascii_case(artist))
            })
            .cloned();
        if let Some(row) = found {
            let tracks = self
                .lib
                .album_tracks(&row.album, &row.artist)
                .unwrap_or_default()
                .into_iter()
                .map(|track| track.track)
                .collect();
            self.album_page = Some(AlbumPage {
                provider: "local".into(),
                id: format!("{}:{}", row.artist, row.album),
                title: row.album,
                artist: row.artist,
                cover_url: row.art_url,
                cover_path: row.art_path,
                release_date: None,
                tracks,
                selected: 0,
            });
            self.album_page_loading = false;
            self.album_page_status.clear();
            self.overlay = Overlay::Album;
        } else {
            // No está descargado: se busca la ficha en el catálogo, igual que
            // al pulsar un álbum en los resultados de búsqueda.
            self.album_page = Some(AlbumPage {
                provider: "search".into(),
                id: album.to_string(),
                title: album.to_string(),
                artist: artist.to_string(),
                cover_url: None,
                cover_path: None,
                release_date: None,
                tracks: Vec::new(),
                selected: 0,
            });
            self.album_page_loading = true;
            self.album_page_status = format!("buscando «{album}»…");
            self.meta_worker
                .album_search(album.to_string(), artist.to_string());
            self.overlay = Overlay::Album;
        }
    }

    /// Abre un álbum del catálogo remoto (resultado de búsqueda o artista).
    pub(super) fn open_album_summary(&mut self, album: &spote_core::spotify_api::AlbumSummary) {
        self.album_page = Some(AlbumPage {
            provider: album.provider.clone(),
            id: album.id.clone(),
            title: album.title.clone(),
            artist: album.artist.clone(),
            cover_url: album.cover_url.clone(),
            cover_path: None,
            release_date: album.release_year.map(|year| year.to_string()),
            tracks: Vec::new(),
            selected: 0,
        });
        self.album_page_loading = true;
        self.album_page_status = format!("cargando {}…", album.title);
        self.meta_worker.album(album.provider.clone(), album.id.clone());
        self.overlay = Overlay::Album;
    }

    pub(super) fn open_album_info(&mut self, info: spote_core::spotify_api::AlbumInfo) {
        self.album_page = Some(AlbumPage {
            provider: info.provider,
            id: info.id,
            title: info.title,
            artist: info.artist,
            cover_url: info.cover_url,
            cover_path: None,
            release_date: info.release_date,
            tracks: info
                .tracks
                .iter()
                .map(super::artist::spotify_track_ref)
                .collect(),
            selected: 0,
        });
        self.album_page_loading = false;
        self.album_page_status.clear();
    }

    pub(crate) fn request_album_page_art(&mut self, page: &AlbumPage) {
        let key = format!("albumpage:{}:{}", page.provider, page.id);
        if self.thumbs.contains_key(&key) || self.thumb_requests.contains(&key) {
            return;
        }
        self.thumb_requests.insert(key.clone());
        self.art_worker.request(crate::fetch::ArtRequest {
            key,
            url: page.cover_url.clone(),
            local: page.cover_path.clone(),
            prefer_url: true,
            w: 8,
            h: 4,
            mode: ArtMode::Half,
        });
    }

    pub(super) fn close_album_page(&mut self) {
        self.overlay = Overlay::None;
        self.album_page = None;
        self.album_page_loading = false;
        self.album_page_status.clear();
    }

    pub(super) fn handle_album_key(&mut self, key: KeyEvent) {
        let len = self.album_page.as_ref().map(|page| page.tracks.len()).unwrap_or(0);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_album_page(),
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(page) = &mut self.album_page {
                    page.selected = page.selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(page) = &mut self.album_page {
                    if len > 0 {
                        page.selected = (page.selected + 1).min(len - 1);
                    }
                }
            }
            KeyCode::Enter => self.play_album_page_track(),
            KeyCode::Char('a') => {
                if let Some(track) = self
                    .album_page
                    .as_ref()
                    .and_then(|page| page.tracks.get(page.selected))
                    .cloned()
                {
                    self.queue_track(track, None);
                }
            }
            KeyCode::Char('d') => self.download_album_page(),
            KeyCode::Char('s') => self.save_album_page_playlist(),
            _ => {}
        }
    }

    pub(super) fn play_album_page_track(&mut self) {
        let Some(track) = self
            .album_page
            .as_ref()
            .and_then(|page| page.tracks.get(page.selected))
            .cloned()
        else {
            return;
        };
        self.play_external_track(track);
    }

    /// Reproduce una pista que puede no estar descargada. Es el mismo camino
    /// que el top de artistas: si hay fichero local suena ya; si no, se resuelve.
    pub(super) fn play_external_track(&mut self, track: TrackRef) {
        if let Some(path) = self.lib.local_path(&track.key).filter(|path| path.exists()) {
            self.play_local(track, Some(path));
            return;
        }
        self.lib.upsert_track(&track).ok();
        if let Some(url) = &track.art_url {
            self.lib.set_art(&track.key, None, Some(url)).ok();
        }
        self.start_resolve(&track);
        self.pending_play = Some(track.key.clone());
        self.now.track = Some(track.clone());
        self.now.loading = true;
        self.now.duration_ms = track.duration_ms;
        self.set_status("descargando…");
        self.lyrics = None;
        self.load_lyrics(&track);
    }

    pub(super) fn download_album_page(&mut self) {
        let Some(page) = self.album_page.clone() else { return };
        let mut queued = 0;
        for track in page.tracks {
            if self
                .lib
                .local_path(&track.key)
                .is_some_and(|path| path.exists())
            {
                continue;
            }
            self.lib.upsert_track(&track).ok();
            if let Some(url) = &track.art_url {
                self.lib.set_art(&track.key, None, Some(url)).ok();
            }
            self.download_track(&track);
            queued += 1;
        }
        self.toast(
            if queued == 0 {
                "el álbum ya está descargado".to_string()
            } else {
                format!("{queued} pistas en descarga")
            },
            ToastKind::Info,
        );
    }

    pub(super) fn save_album_page_playlist(&mut self) {
        let Some(page) = self.album_page.clone() else { return };
        if page.tracks.is_empty() {
            self.toast("el álbum no tiene pistas", ToastKind::Warn);
            return;
        }
        let name = format!("{} — {}", page.artist, page.title);
        let id = match self.lib.create_playlist_full(
            &name,
            &format!("álbum · {} pistas", page.tracks.len()),
            None,
            page.cover_url.as_deref(),
        ) {
            Ok(id) => id,
            Err(err) => {
                self.toast(format!("no pude crear la lista: {err}"), ToastKind::Error);
                return;
            }
        };
        for track in &page.tracks {
            self.lib.upsert_track(track).ok();
            self.lib.add_to_playlist(id, &track.key).ok();
        }
        self.toast(format!("«{name}» guardado como lista"), ToastKind::Ok);
    }

    pub(super) fn close_album(&mut self) {
        self.album_open = None;
        self.view.clear();
        self.selected = 0;
        self.refresh_view();
    }

    pub(super) fn move_album_selection(&mut self, delta: i32) {
        if self.album_open.is_some() {
            self.move_selection(delta);
            return;
        }
        if self.albums.is_empty() {
            return;
        }
        let next = (self.album_selected as i32 + delta)
            .clamp(0, self.albums.len() as i32 - 1) as usize;
        self.album_selected = next;
    }

    /// Baja todas las pistas que falten del álbum seleccionado.
    pub(super) fn download_album(&mut self) {
        let album = self.album_open.clone().or_else(|| self.albums.get(self.album_selected).cloned());
        let Some(album) = album else { return };
        let tracks = self
            .lib
            .album_tracks(&album.album, &album.artist)
            .unwrap_or_default();
        let mut queued = 0usize;
        for row in &tracks {
            if row.playable() {
                continue;
            }
            self.lib.upsert_track(&row.track).ok();
            if let Some(url) = &row.track.art_url {
                self.lib.set_art(&row.track.key, None, Some(url)).ok();
            }
            self.download_track(&row.track);
            queued += 1;
        }
        if queued == 0 {
            self.toast("el álbum ya está descargado", ToastKind::Info);
        } else {
            self.toast(
                format!("{} · {} pistas en descarga", album.album, queued),
                ToastKind::Ok,
            );
        }
    }

    /// Guarda el álbum como una lista local con todas sus pistas.
    pub(super) fn save_album_playlist(&mut self) {
        let album = self.album_open.clone().or_else(|| self.albums.get(self.album_selected).cloned());
        let Some(album) = album else { return };
        let tracks = self
            .lib
            .album_tracks(&album.album, &album.artist)
            .unwrap_or_default();
        if tracks.is_empty() {
            self.toast("el álbum no tiene pistas", ToastKind::Warn);
            return;
        }
        let name = format!("{} — {}", album.artist, album.album);
        let description = format!("álbum · {} pistas", tracks.len());
        let cover_url = album.art_url.as_deref();
        let id = match self.lib.create_playlist_full(
            &name,
            &description,
            None,
            cover_url,
        ) {
            Ok(id) => id,
            Err(err) => {
                self.toast(format!("no pude crear la lista: {err}"), ToastKind::Error);
                return;
            }
        };
        for row in &tracks {
            self.lib.add_to_playlist(id, &row.track.key).ok();
        }
        self.lib_filter = LibFilter::Playlist(id);
        self.tab = Tab::Library;
        self.selected = 0;
        self.refresh_view();
        self.toast(format!("«{name}» guardado como lista"), ToastKind::Ok);
    }

    /// Enter en la lista de álbumes o en el tracklist.
    pub(super) fn album_enter(&mut self) {
        if self.album_open.is_some() {
            if let Some(row) = self.selected_row().cloned() {
                self.play_local(row.track, row.path);
            }
        } else {
            self.open_album_selected();
        }
    }

    /// Portada sintética del álbum para pedir la miniatura al worker.
    /// Pide la miniatura del álbum usando la portada local si existe. Evita
    /// depender de la red para álbumes ya ingestados.
    pub(crate) fn request_album_thumb(&mut self, album: &AlbumRow) {
        let key = format!("album:{}:{}", album.artist, album.album);
        if self.thumbs.contains_key(&key) || self.thumb_requests.contains(&key) {
            return;
        }
        self.thumb_requests.insert(key.clone());
        self.art_worker.request(crate::fetch::ArtRequest {
            key,
            url: album.art_url.clone(),
            local: album.art_path.clone(),
            prefer_url: true,
            w: 6,
            h: 3,
            mode: ArtMode::Half,
        });
    }

    /// Si el álbum no tiene portada, pide metadatos de su primera pista (una
    /// vez por álbum) para que la miniatura se rellene sola.
    pub(crate) fn ensure_album_art(&mut self, album: &AlbumRow) {
        if album.art_path.is_some() || album.art_url.is_some() {
            return;
        }
        if self.metadata_requests.len() >= 3 {
            return;
        }
        if let Ok(tracks) = self.lib.album_tracks(&album.album, &album.artist) {
            if let Some(row) = tracks.first() {
                self.ensure_missing_art(row);
            }
        }
    }

    pub(crate) fn album_thumb_track(album: &AlbumRow) -> TrackRef {
        TrackRef {
            key: format!("album:{}:{}", album.artist, album.album),
            title: album.album.clone(),
            artist: album.artist.clone(),
            album: Some(album.album.clone()),
            duration_ms: album.duration_ms,
            art_url: album.art_url.clone(),
            source_hint: None,
        }
    }
}
