//! Menú contextual (clic derecho) y acciones destructivas controladas.

use super::*;

impl App {
    /// Detiene y limpia la reproducción si la pista borrada es la actual.
    fn stop_current_if_key(&mut self, key: &str) {
        if self.now.track.as_ref().map(|track| track.key.as_str()) != Some(key) {
            return;
        }
        if let Some(player) = &self.player {
            player.stop();
        }
        self.streaming = false;
        self.pending_stream = None;
        self.pending_play = None;
        self.now.track = None;
        self.now.path = None;
        self.now.position_ms = 0;
        self.now.duration_ms = 0;
        self.now.playing = false;
        self.now.loading = false;
        self.now.ended = false;
        self.lyrics = None;
        self.lyrics_key.clear();
        self.lyrics_status.clear();
        self.art = None;
        self.art_key.clear();
        self.spectrum.stop();
        self.play_context.retain(|entry| entry.track.key != key);
        self.queue.retain(|entry| entry.track.key != key);
        if self.context_index >= self.play_context.len() {
            self.context_index = self.play_context.len().saturating_sub(1);
        }
    }

    pub(super) fn open_context(&mut self, action: &Action) {
        let menu = match action {
            Action::SelectRow(index) => self
                .view
                .get(*index)
                .map(|row| ContextMenu {
                    origin: ContextOrigin::Library,
                    index: *index,
                    key: row.track.key.clone(),
                    title: row.track.title.clone(),
                    artist: row.track.artist.clone(),
                    url: None,
                    path: row.path.clone(),
                    selected: 0,
                }),
            Action::SearchPlay(index) => match self.search_entry(*index) {
                Some(SearchEntry::Track(track_index)) => {
                    self.search.results.get(track_index).map(|item| {
                        let track = item.to_track();
                        ContextMenu {
                            origin: ContextOrigin::Search,
                            index: track_index,
                            key: track.key,
                            title: item.title.clone(),
                            artist: item.artist.clone(),
                            url: Some(item.url.clone()).filter(|url| !url.is_empty()),
                            path: None,
                            selected: 0,
                        }
                    })
                }
                Some(SearchEntry::Album(index)) => {
                    self.search.albums.get(index).map(|album| ContextMenu {
                        origin: ContextOrigin::SearchAlbum,
                        index,
                        key: String::new(),
                        title: album.title.clone(),
                        artist: album.artist.clone(),
                        url: album.cover_url.clone(),
                        path: None,
                        selected: 0,
                    })
                }
                Some(SearchEntry::Artist(index)) => {
                    self.search.artists.get(index).map(|artist| ContextMenu {
                        origin: ContextOrigin::SearchArtist,
                        index,
                        key: String::new(),
                        title: artist.name.clone(),
                        artist: String::new(),
                        url: artist.image_url.clone(),
                        path: None,
                        selected: 0,
                    })
                }
                _ => None,
            },
            Action::QueueRow(index) | Action::QueueRemove(index) => {
                self.queue.get(*index).map(|entry| ContextMenu {
                    origin: ContextOrigin::Queue,
                    index: *index,
                    key: entry.track.key.clone(),
                    title: entry.track.title.clone(),
                    artist: entry.track.artist.clone(),
                    url: None,
                    path: entry.path.clone(),
                    selected: 0,
                })
            }
            Action::OpenArtist(name) => Some(ContextMenu {
                origin: ContextOrigin::ArtistName,
                index: 0,
                key: String::new(),
                title: name.clone(),
                artist: String::new(),
                url: None,
                path: None,
                selected: 0,
            }),
            Action::OpenAlbum(album, artist) => Some(ContextMenu {
                origin: ContextOrigin::AlbumName,
                index: 0,
                key: String::new(),
                title: album.clone(),
                artist: artist.clone(),
                url: None,
                path: None,
                selected: 0,
            }),
            Action::NavMenu => Some(ContextMenu {
                origin: ContextOrigin::Nav,
                index: 0,
                key: String::new(),
                title: "vistas".into(),
                artist: "listas y paneles".into(),
                url: None,
                path: None,
                selected: 0,
            }),
            Action::QueueMenu => Some(ContextMenu {
                origin: ContextOrigin::QueueHeader,
                index: 0,
                key: String::new(),
                title: "cola".into(),
                artist: format!("{} pistas", self.queue_len()),
                url: None,
                path: None,
                selected: 0,
            }),
            _ => None,
        };
        if let Some(menu) = menu {
            self.context = Some(menu);
            self.overlay = Overlay::Context;
        }
    }

    pub(crate) fn context_menu(&self) -> Option<&ContextMenu> {
        self.context.as_ref()
    }

    pub(super) fn handle_context_key(&mut self, key: KeyEvent) {
        let Some(mut menu) = self.context.clone() else {
            self.overlay = Overlay::None;
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.context = None;
                self.overlay = Overlay::None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                menu.selected = menu.selected.saturating_sub(1);
                self.context = Some(menu);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                menu.selected = (menu.selected + 1).min(menu.actions().len().saturating_sub(1));
                self.context = Some(menu);
            }
            KeyCode::Enter => {
                self.context = Some(menu);
                self.apply_context();
            }
            // Segunda pulsación de Supr dentro del menú: borra el audio y la
            // fila de la biblioteca sin tener que bajar hasta la opción.
            KeyCode::Delete => {
                let actions = menu.actions();
                let target = [
                    ContextAction::DeleteFile,
                    ContextAction::RemoveLibrary,
                    ContextAction::RemoveQueue,
                ]
                .into_iter()
                .find(|action| actions.contains(action));
                if let Some(target) = target {
                    if let Some(index) = actions.iter().position(|action| *action == target) {
                        menu.selected = index;
                    }
                    self.context = Some(menu);
                    self.apply_context();
                }
            }
            _ => {}
        }
    }

    pub(super) fn apply_context(&mut self) {
        let Some(menu) = self.context.clone() else { return };
        let action = menu.action();
        match action {
            ContextAction::Play => {
                if menu.origin == ContextOrigin::Queue {
                    self.queue_selected = menu.index.min(self.queue_len().saturating_sub(1));
                    if let Some(entry) = self.queue.get(menu.index).cloned() {
                        self.queue.remove(menu.index);
                        self.play_local(entry.track, entry.path);
                    }
                } else if let Ok(Some(row)) = self.lib.track_by_key(&menu.key) {
                    self.play_local(row.track, row.path);
                } else if self.search.results.get(menu.index).is_some() {
                    self.search.selected = menu.index;
                    self.play_search_selection();
                }
            }
            ContextAction::Queue => {
                if let Ok(Some(row)) = self.lib.track_by_key(&menu.key) {
                    self.queue_track(row.track, menu.url.clone());
                } else if let Some(item) = self.search.results.get(menu.index) {
                    self.queue_track(item.to_track(), Some(item.url.clone()));
                }
            }
            ContextAction::Download => {
                if self.lib.local_path(&menu.key).is_none() {
                    if let Ok(Some(row)) = self.lib.track_by_key(&menu.key) {
                        self.download_track(&row.track);
                    } else if let Some(item) = self.search.results.get(menu.index).cloned() {
                        let track = item.to_track();
                        self.lib.upsert_track(&track).ok();
                        if let Some(url) = &track.art_url {
                            self.lib.set_art(&track.key, None, Some(url)).ok();
                        }
                        if item.provider == "spotify" || item.provider == "deezer" {
                            self.start_resolve(&track);
                        } else if !item.url.is_empty() {
                            self.start_download(&track, item.url.clone());
                        }
                        self.toast("descargando en segundo plano", ToastKind::Info);
                    }
                } else {
                    self.toast("ya está descargada", ToastKind::Info);
                }
            }
            ContextAction::Favorite => {
                self.toggle_favorite_key(&menu.key);
            }
            ContextAction::Artist => {
                self.open_artist();
            }
            ContextAction::Cover => {
                self.request_spotify_metadata_forced(&menu.key, true);
            }
            ContextAction::RemoveLibrary => {
                match self.lib.delete_track(&menu.key) {
                    Ok(_) => {
                        self.stop_current_if_key(&menu.key);
                        self.toast("pista quitada de la biblioteca", ToastKind::Ok);
                        self.refresh_library();
                    }
                    Err(err) => self.toast(format!("no pude quitarla: {err}"), ToastKind::Error),
                }
            }
            ContextAction::DeleteFile => {
                let path = menu.path.clone().or_else(|| self.lib.local_path(&menu.key));
                if let Some(path) = path {
                    match std::fs::remove_file(&path) {
                        Ok(()) => {
                            let cover = path.with_extension("jpg");
                            let _ = std::fs::remove_file(cover);
                            self.stop_current_if_key(&menu.key);
                            self.lib.delete_track(&menu.key).ok();
                            self.toast("audio borrado del disco", ToastKind::Ok);
                            self.refresh_library();
                        }
                        Err(err) => self.toast(format!("no pude borrar: {err}"), ToastKind::Error),
                    }
                } else {
                    self.toast("esta pista no tiene fichero local", ToastKind::Warn);
                }
            }
            ContextAction::RemoveQueue => {
                self.queue_selected = menu.index.min(self.queue_len().saturating_sub(1));
                self.remove_queue_selected();
            }
            ContextAction::Status => {
                let status = if menu.path.is_some() {
                    "descargada".to_string()
                } else {
                    "pendiente de descarga".to_string()
                };
                self.toast(format!("{}  ·  {status}", menu.title), ToastKind::Info);
            }
            ContextAction::NewPlaylist => {
                self.context = None;
                self.open_playlist_form();
                return;
            }
            ContextAction::ImportPlaylist => {
                self.context = None;
                self.open_import_playlist();
                return;
            }
            ContextAction::Layout => {
                self.context = None;
                self.overlay = Overlay::Layout;
                return;
            }
            ContextAction::RefreshLibrary => {
                self.refresh_library();
                self.toast("biblioteca actualizada", ToastKind::Info);
            }
            ContextAction::ClearQueue => {
                self.queue.clear();
                self.queue_selected = 0;
                self.toast("cola vaciada", ToastKind::Info);
            }
            ContextAction::Open => {
                if menu.origin == ContextOrigin::SearchAlbum {
                    if let Some(album) = self.search.albums.get(menu.index).cloned() {
                        self.context = None;
                        self.open_album_summary(&album);
                        return;
                    }
                } else if menu.origin == ContextOrigin::SearchArtist {
                    if let Some(artist) = self.search.artists.get(menu.index).cloned() {
                        self.context = None;
                        self.open_artist_summary(&artist);
                        return;
                    }
                } else if menu.origin == ContextOrigin::ArtistName {
                    self.context = None;
                    self.open_artist_by_name(&menu.title);
                    return;
                } else if menu.origin == ContextOrigin::AlbumName {
                    self.context = None;
                    self.open_album_by_name(&menu.title, &menu.artist);
                    return;
                }
            }
        }
        let keep_overlay = matches!(action, ContextAction::Artist);
        self.context = None;
        if !keep_overlay {
            self.overlay = Overlay::None;
        }
    }

    /// Descarga sin reproducir: lo usa el menú contextual.
    pub(super) fn download_track(&mut self, track: &TrackRef) {
        let hint = track.source_hint.clone().or_else(|| {
            self.lib.source_hint(&track.key).map(|(provider, id)| {
                spote_protocol::SourceHint {
                    provider,
                    id,
                    duration_ms: track.duration_ms,
                }
            })
        });
        if let Some(hint) = hint {
            self.lib.upsert_track(track).ok();
            if hint.provider == "spotify" || hint.provider == "deezer" {
                self.start_resolve(track);
            } else {
                self.downloader.download(
                    &track.key,
                    DownloadSource::Url(format!(
                        "https://www.youtube.com/watch?v={}",
                        hint.id
                    )),
                    self.cfg.media_dir(),
                );
            }
            self.toast("descargando en segundo plano", ToastKind::Info);
        } else if !track.artist.is_empty() && !track.title.is_empty() {
            // Sin hint ni URL: se busca la mejor fuente con artista+título.
            self.start_resolve(track);
            self.toast("buscando fuente para descargar…", ToastKind::Info);
        } else {
            self.toast("no hay fuente descargable para esta pista", ToastKind::Warn);
        }
    }
}
