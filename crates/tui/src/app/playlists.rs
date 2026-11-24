//! Formularios y acciones de listas.
use super::*;

impl App {
    pub(super) fn handle_overlay_key(&mut self, key: KeyEvent) {
        match self.overlay.clone() {
            Overlay::None => {}
            Overlay::Context => self.handle_context_key(key),
            Overlay::Layout => self.handle_layout_key(key),
            Overlay::Artist => {
                self.handle_artist_key(key);
            }
            Overlay::Album => {
                self.handle_album_key(key);
            }
            Overlay::Update => match key.code {
                KeyCode::Enter | KeyCode::Char('u') => {
                    if self.update_run.is_none() {
                        self.update_finished = None;
                        self.start_update();
                    }
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    self.update_dismissed = true;
                    self.overlay = Overlay::None;
                }
                _ => {}
            },
            Overlay::Audio => self.handle_audio_key(key),
            Overlay::Wrapped => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.overlay = Overlay::None,
                KeyCode::Char('a') | KeyCode::Char('A') => self.toggle_wrapped_period(),
                _ => {}
            },
            Overlay::Trim => match key.code {
                KeyCode::Esc | KeyCode::Enter => self.overlay = Overlay::None,
                KeyCode::Char('a') | KeyCode::Home => self.trim_start_here(),
                KeyCode::Char('b') | KeyCode::End => self.trim_end_here(),
                KeyCode::Char('c') | KeyCode::Backspace => self.clear_trim(),
                _ => {}
            },
            Overlay::Help | Overlay::Effects => match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.overlay = Overlay::None;
                }
                KeyCode::Char(c @ '1'..='9') if self.overlay == Overlay::Effects => {
                    self.viz.toggle_layer(c as usize - '1' as usize);
                }
                KeyCode::Char('e') if self.overlay == Overlay::Effects => {
                    self.overlay = Overlay::None;
                }
                _ => {}
            },
            Overlay::PlaylistForm => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::BackTab => {
                    self.playlist_form.field = (self.playlist_form.field + 1) % 3;
                }
                KeyCode::Enter => self.commit_playlist_form(),
                KeyCode::Backspace => {
                    self.playlist_form.editing().pop();
                }
                KeyCode::Char(c) => {
                    self.playlist_form.editing().push(c);
                }
                _ => {}
            },
            Overlay::ImportPlaylist => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Enter => self.commit_import_playlist(),
                KeyCode::Backspace => {
                    self.import_input.pop();
                }
                KeyCode::Char(c) => self.import_input.push(c),
                _ => {}
            },
            Overlay::ConfirmDeletePlaylist { id, .. } => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Enter => {
                    if let Err(err) = self.lib.delete_playlist(id) {
                        self.toast(format!("no pude borrar la lista: {err}"), ToastKind::Error);
                    } else {
                        if self.lib_filter == LibFilter::Playlist(id) {
                            self.lib_filter = LibFilter::All;
                        }
                        self.refresh_view();
                        self.toast("lista borrada", ToastKind::Ok);
                    }
                    self.overlay = Overlay::None;
                }
                _ => {}
            },
            Overlay::AddToPlaylist { key: track_key, selected } => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Up | KeyCode::Char('k') => {
                    let selected = selected.saturating_sub(1);
                    self.overlay = Overlay::AddToPlaylist { key: track_key, selected };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let len = self.lib.playlists().map(|lists| lists.len()).unwrap_or(0);
                    let selected = (selected + 1).min(len.saturating_sub(1));
                    self.overlay = Overlay::AddToPlaylist { key: track_key, selected };
                }
                KeyCode::Enter => {
                    if let Ok(lists) = self.lib.playlists() {
                        if let Some(list) = lists.get(selected) {
                            if let Err(err) = self.lib.add_to_playlist(list.id, &track_key) {
                                self.toast(format!("no pude añadir: {err}"), ToastKind::Error);
                            } else {
                                self.toast(
                                    format!("añadida a '{}'", list.name),
                                    ToastKind::Ok,
                                );
                                self.refresh_view();
                            }
                        }
                    }
                    self.overlay = Overlay::None;
                }
                _ => {}
            },
        }
    }


    fn commit_playlist_form(&mut self) {
        let name = self.playlist_form.name.trim().to_string();
        if name.is_empty() {
            self.toast("la lista necesita un nombre", ToastKind::Warn);
            return;
        }
        let description = self.playlist_form.description.trim().to_string();
        let cover = self.playlist_form.cover.trim().to_string();
        let cover_url = cover.starts_with("http").then(|| cover.clone());
        let cover_key = (!cover.is_empty() && !cover.starts_with("http")).then(|| cover.as_str());
        match self
            .lib
            .create_playlist_full(&name, &description, cover_key, cover_url.as_deref())
        {
            Ok(id) => {
                self.lib_filter = LibFilter::Playlist(id);
                self.tab = Tab::Library;
                self.selected = 0;
                self.playlist_form = PlaylistForm::default();
                self.overlay = Overlay::None;
                self.refresh_view();
                self.toast(format!("lista '{name}' creada"), ToastKind::Ok);
            }
            Err(err) => self.toast(format!("no pude crear la lista: {err}"), ToastKind::Error),
        }
    }


    fn commit_import_playlist(&mut self) {
        let input = self.import_input.trim().to_string();
        if input.is_empty() {
            return;
        }
        self.spotify_jobs.import_playlist(input);
        self.import_input.clear();
        self.overlay = Overlay::None;
        self.set_status("importando lista…");
        self.toast("importando de Spotify…", ToastKind::Info);
    }

    /// Pista sobre la que actuar (búsqueda o biblioteca) para añadir a lista.

    fn context_key(&self) -> Option<String> {
        match self.tab {
            Tab::Search => match self.search_entry(self.search.selected) {
                Some(SearchEntry::Track(index)) => self
                    .search
                    .results
                    .get(index)
                    .map(|item| item.to_track().key),
                _ => None,
            },
            Tab::Library | Tab::Now | Tab::Albums => {
                self.selected_row().map(|row| row.track.key.clone())
            }
            _ => None,
        }
    }


    pub fn open_playlist_form(&mut self) {
        self.playlist_form = PlaylistForm::default();
        self.overlay = Overlay::PlaylistForm;
    }


    pub fn open_import_playlist(&mut self) {
        self.import_input.clear();
        self.overlay = Overlay::ImportPlaylist;
    }


    pub fn open_add_to_playlist(&mut self) {
        let Some(key) = self.context_key() else {
            self.toast("selecciona una pista para añadirla a una lista", ToastKind::Warn);
            return;
        };
        if !self.ensure_track_key(&key) {
            self.toast("no encuentro esa pista en la biblioteca", ToastKind::Warn);
            return;
        }
        if self.lib.playlists().map(|lists| lists.is_empty()).unwrap_or(true) {
            self.toast("no hay listas: crea una con N", ToastKind::Warn);
            return;
        }
        self.overlay = Overlay::AddToPlaylist { key, selected: 0 };
    }


    pub fn open_delete_playlist(&mut self) {
        let LibFilter::Playlist(id) = self.lib_filter else {
            self.toast("abre una lista para borrarla (d)", ToastKind::Warn);
            return;
        };
        let name = self
            .lib
            .playlist(id)
            .ok()
            .flatten()
            .map(|list| list.name)
            .unwrap_or_else(|| "esta lista".into());
        self.overlay = Overlay::ConfirmDeletePlaylist { id, name };
    }

}
