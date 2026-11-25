//! Teclado y ratón.
use super::*;

impl App {
    /// Entrar en una pestaña con Tab es equivalente a pulsarla en la barra:
    /// la biblioteca siempre arranca en «todo», no en el filtro que quedó.
    fn enter_tab(&mut self) {
        self.needs_redraw = true;
        if self.tab == Tab::Library {
            self.lib_filter = LibFilter::All;
            self.selected = 0;
        }
        self.refresh_view();
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        // Salida de emergencia: aunque un overlay se atasque o falte su
        // contenido, ctrl-c siempre cierra la app.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.persist_volume();
            self.should_quit = true;
            return;
        }
        if self.overlay != Overlay::None {
            self.handle_overlay_key(key);
            return;
        }
        // Buscador inline de la biblioteca: captura el teclado mientras tiene
        // el foco para no disparar atajos globales al escribir.
        if self.lib_search_focus {
            match key.code {
                KeyCode::Esc => {
                    if self.lib_query.is_empty() {
                        self.lib_search_focus = false;
                    } else {
                        self.lib_query.clear();
                        self.selected = 0;
                        self.refresh_view();
                    }
                }
                KeyCode::Enter => {
                    self.lib_search_focus = false;
                    if self.selected_row().is_some() {
                        self.play_row(self.selected);
                    }
                }
                KeyCode::Backspace => {
                    self.lib_query.pop();
                    self.selected = 0;
                    self.refresh_view();
                }
                KeyCode::Tab => {
                    self.lib_search_focus = false;
                    self.tab = self.tab.next();
                    self.enter_tab();
                }
                KeyCode::BackTab => {
                    self.lib_search_focus = false;
                    self.tab = self.tab.prev();
                    self.enter_tab();
                }
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !self.view.is_empty() {
                        self.selected = (self.selected + 1).min(self.view.len() - 1);
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.lib_query.clear();
                    self.selected = 0;
                    self.refresh_view();
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.persist_volume();
                    self.should_quit = true;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.lib_query.push(c);
                    self.selected = 0;
                    self.refresh_view();
                }
                _ => {}
            }
            self.needs_redraw = true;
            return;
        }
        // Buscador de discos guardados.
        if self.tab == Tab::Albums && self.album_search_focus {
            match key.code {
                KeyCode::Esc => {
                    if self.album_query.is_empty() {
                        self.album_search_focus = false;
                    } else {
                        self.album_query.clear();
                        self.album_selected = 0;
                        self.refresh_view();
                    }
                }
                KeyCode::Enter => {
                    self.album_search_focus = false;
                    self.open_album_selected();
                }
                KeyCode::Backspace => {
                    self.album_query.pop();
                    self.album_selected = 0;
                    self.refresh_view();
                }
                KeyCode::Tab => {
                    self.album_search_focus = false;
                    self.tab = self.tab.next();
                    self.enter_tab();
                }
                KeyCode::BackTab => {
                    self.album_search_focus = false;
                    self.tab = self.tab.prev();
                    self.enter_tab();
                }
                KeyCode::Up => {
                    self.album_selected = self.album_selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !self.albums.is_empty() {
                        self.album_selected = (self.album_selected + 1).min(self.albums.len() - 1);
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.album_query.clear();
                    self.album_selected = 0;
                    self.refresh_view();
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.persist_volume();
                    self.should_quit = true;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.album_query.push(c);
                    self.album_selected = 0;
                    self.refresh_view();
                }
                _ => {}
            }
            self.needs_redraw = true;
            return;
        }
        // En la vista de búsqueda se escribe directamente: no hace falta "/".
        if self.tab == Tab::Search {
            match key.code {
                KeyCode::Esc => {
                    if self.search.query.is_empty() {
                        self.tab = Tab::Now;
                    } else {
                        self.search.query.clear();
                        self.search.results.clear();
                        self.search.albums.clear();
                        self.search.artists.clear();
                        self.search.last_query_id = self.search.last_query_id.wrapping_add(1);
                        self.search.loading = false;
                    }
                    return;
                }
                KeyCode::Enter => {
                    if self.search_len() == 0 {
                        self.submit_search();
                    } else if self.search.last_query == self.search.query.trim()
                        && (self.search.last_query_id > 0)
                    {
                        self.activate_search_selection();
                    } else {
                        self.submit_search();
                    }
                    return;
                }
                KeyCode::Backspace => {
                    self.search.query.pop();
                    self.search.last_query_id = self.search.last_query_id.wrapping_add(1);
                    self.search.loading = false;
                    self.search.results.clear();
                    self.search.albums.clear();
                    self.search.artists.clear();
                    return;
                }
                KeyCode::Up => {
                    self.search.selected = self.search.selected.saturating_sub(1);
                    return;
                }
                KeyCode::Down => {
                    let len = self.search_len();
                    if len > 0 {
                        self.search.selected = (self.search.selected + 1).min(len - 1);
                    }
                    return;
                }
                KeyCode::Tab => {
                    self.tab = self.tab.next();
                    self.enter_tab();
                    return;
                }
                KeyCode::BackTab => {
                    self.tab = self.tab.prev();
                    self.enter_tab();
                    return;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.persist_volume();
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.search.query.push(c);
                    self.search.results.clear();
                    self.search.albums.clear();
                    self.search.artists.clear();
                    self.search.selected = 0;
                    self.search.message.clear();
                    self.search.last_query_id = self.search.last_query_id.wrapping_add(1);
                    self.search.loading = false;
                    return;
                }
                _ => {}
            }
        }

        // Búsqueda manual de letras: captura todo mientras está abierta.
        if self.lyrics_search.is_some() {
            self.handle_lyrics_search_key(key);
            return;
        }

        // La pestaña de audio tiene su propio teclado (bandas, preamp,
        // normalización). Las teclas globales que no consuma siguen abajo.
        if self.tab == Tab::Audio && self.handle_audio_tab_key(key) {
            return;
        }

        // Panel de artista/álbum: captura sus teclas, pero deja pasar las
        // globales (espacio, tab, q…).
        if self.detail.is_some() && self.handle_detail_key(key) {
            return;
        }

        let upper = key.modifiers.contains(KeyModifiers::SHIFT)
            || matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase());
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.persist_volume();
                self.should_quit = true
            }
            KeyCode::Char('q') if key.modifiers.is_empty() => {
                self.persist_volume();
                self.should_quit = true;
            }
            KeyCode::Char(' ') => self.toggle_play(),
            KeyCode::Esc => {
                if self.tab == Tab::Albums && self.album_open.is_some() {
                    self.close_album();
                } else if self.tab == Tab::Albums {
                    self.tab = Tab::Library;
                }
                self.fullscreen = Fullscreen::None;
                self.overlay = Overlay::None;
                self.center_focus = CenterFocus::Normal;
                self.focus = Pane::Center;
            }
            KeyCode::Tab => {
                self.tab = self.tab.next();
                self.focus = Pane::Center;
                if self.tab == Tab::Search {
                    self.search.query.clear();
                    self.search.results.clear();
                }
                self.enter_tab();
            }
            KeyCode::BackTab => {
                self.tab = self.tab.prev();
                self.focus = Pane::Center;
                self.enter_tab();
            }
            KeyCode::Char('/') => {
                if self.tab == Tab::Library {
                    self.lib_search_focus = true;
                } else if self.tab == Tab::Albums {
                    self.album_search_focus = true;
                } else {
                    self.tab = Tab::Search;
                    self.search.results.clear();
                    self.search.query.clear();
                }
                self.needs_redraw = true;
            }
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('e') if !upper => self.overlay = Overlay::Effects,
            KeyCode::Char('A') => self.open_audio(),
            KeyCode::Char('w') if !upper => self.open_mini(),
            KeyCode::Char('W') => self.open_wrapped(),
            KeyCode::Char('E') => self.overlay = Overlay::Layout,
            KeyCode::Char('g') if !upper => self.open_artist(),
            KeyCode::Delete => {
                if matches!(self.tab, Tab::Library | Tab::Now | Tab::Albums)
                    && self.selected_row().is_some()
                {
                    self.open_context(&Action::SelectRow(self.selected));
                }
            }
            KeyCode::Char('R') => {
                self.refresh_library();
                self.toast("biblioteca actualizada", ToastKind::Info);
            }
            KeyCode::Char('y') => self.copy_lyrics(),
            KeyCode::Char('l') if !upper => {
                if matches!(self.tab, Tab::Library | Tab::Search | Tab::Albums) {
                    self.open_add_to_playlist();
                } else {
                    self.focus = Pane::Center;
                    self.tab = Tab::Now;
                    self.center_focus = if self.center_focus == CenterFocus::Lyrics {
                        CenterFocus::Normal
                    } else {
                        CenterFocus::Lyrics
                    };
                }
            }
            KeyCode::Char('s') if !upper => {
                if self.tab == Tab::Albums {
                    self.save_album_playlist();
                } else {
                    self.tab = Tab::Room;
                    if let Some(social) = &self.social {
                        social.send(SocialCmd::Refresh);
                    }
                }
            }
            KeyCode::Char('L') => self.leave_room(),
            KeyCode::Char('c') if !upper => {
                self.center_focus = if self.center_focus == CenterFocus::Art {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Art
                };
            }
            KeyCode::Char('C') => {
                self.center_focus = if self.center_focus == CenterFocus::Lyrics {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Lyrics
                };
            }
            KeyCode::Char('G')
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics =>
            {
                self.toggle_generated_lyrics()
            }
            KeyCode::Char('D') => {
                self.lyrics_debug = !self.lyrics_debug;
                self.lyrics_debug_last = None;
                self.toast(
                    if self.lyrics_debug {
                        "debug de letra: on"
                    } else {
                        "debug de letra: off"
                    },
                    ToastKind::Info,
                );
            }
            KeyCode::Right => {
                if self.tab == Tab::Settings {
                    self.adjust_setting(self.settings_selected, 1);
                    return;
                }
                let step = if upper { 30_000 } else { 5_000 };
                self.seek_relative(step);
            }
            KeyCode::Left => {
                if self.tab == Tab::Settings {
                    self.adjust_setting(self.settings_selected, -1);
                    return;
                }
                let step = if upper { 30_000 } else { 5_000 };
                self.seek_relative(-step);
            }
            KeyCode::Up => {
                if self.tab == Tab::Settings {
                    self.settings_selected = self.settings_selected.saturating_sub(1);
                    return;
                }
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics {
                    self.scroll_lyrics(-1);
                    return;
                }
                if upper {
                    self.move_queue(-1);
                } else if self.focus == Pane::Side {
                    self.move_queue_selection(-1);
                } else if self.tab == Tab::Now || self.focus == Pane::Transport {
                    self.set_volume(self.now.volume + 0.05);
                } else if self.tab == Tab::Albums {
                    self.move_album_selection(-1);
                } else {
                    self.move_selection(-1);
                }
            }
            KeyCode::Down => {
                if self.tab == Tab::Settings {
                    self.settings_selected =
                        (self.settings_selected + 1).min(SettingRow::ALL.len() - 1);
                    return;
                }
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics {
                    self.scroll_lyrics(1);
                    return;
                }
                if upper {
                    self.move_queue(1);
                } else if self.focus == Pane::Side {
                    self.move_queue_selection(1);
                } else if self.tab == Tab::Now || self.focus == Pane::Transport {
                    self.set_volume(self.now.volume - 0.05);
                } else if self.tab == Tab::Albums {
                    self.move_album_selection(1);
                } else {
                    self.move_selection(1);
                }
            }
            KeyCode::PageUp => {
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics {
                    self.scroll_lyrics(-6);
                } else {
                    self.move_selection(-8);
                }
            }
            KeyCode::PageDown => {
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics {
                    self.scroll_lyrics(6);
                } else {
                    self.move_selection(8);
                }
            }
            KeyCode::Home if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics => {
                self.follow_current_lyric()
            }
            KeyCode::Home if self.tab == Tab::Now => self.trim_start_here(),
            KeyCode::End if self.tab == Tab::Now => self.trim_end_here(),
            KeyCode::Backspace => self.clear_trim(),
            KeyCode::Enter => {
                if self.tab == Tab::Settings {
                    self.toggle_setting(self.settings_selected);
                } else if self.tab == Tab::Room {
                    self.join_room(self.room_selected);
                } else if self.focus == Pane::Side {
                    self.play_queue_selected();
                } else if self.tab == Tab::Albums {
                    self.album_enter();
                } else if matches!(self.tab, Tab::Library | Tab::Now) {
                    // `play_row` crea el contexto para que next/prev/repeat
                    // funcionen también desde listas.
                    self.play_row(self.selected);
                }
            }
            KeyCode::Char('a') if !upper => match self.tab {
                Tab::Search => {
                    if let Some(SearchEntry::Track(index)) = self.search_entry(self.search.selected)
                    {
                        if let Some(item) = self.search.results.get(index).cloned() {
                            self.queue_track(item.to_track(), None);
                        }
                    }
                }
                Tab::Library | Tab::Now | Tab::Albums => {
                    if let Some(row) = self.selected_row().cloned() {
                        self.queue_track(row.track, None);
                    }
                }
                _ => {}
            },
            KeyCode::Char('M') => {
                self.cycle_art_mode();
            }
            KeyCode::Char('f') if !upper => self.toggle_favorite(),
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.fullscreen = if self.fullscreen == Fullscreen::Art {
                    Fullscreen::None
                } else {
                    Fullscreen::Art
                };
                self.needs_redraw = true;
            }
            KeyCode::Char('v') if !upper => {
                if self.show_viz {
                    self.viz.next_preset();
                } else {
                    self.show_viz = true;
                }
            }
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.fullscreen = if self.fullscreen == Fullscreen::Viz {
                    Fullscreen::None
                } else {
                    Fullscreen::Viz
                };
                self.needs_redraw = true;
            }
            KeyCode::Char('u') if !upper => self.toggle_loop(),
            KeyCode::Char('i') => self.set_zen(!self.zen),
            KeyCode::Char('T') if self.tab == Tab::Now => self.overlay = Overlay::Trim,
            KeyCode::Char('t')
                if self.tab == Tab::Now && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.overlay = Overlay::Trim
            }
            KeyCode::Char('N') => self.open_playlist_form(),
            KeyCode::Char('[') => self.cycle_filter(-1),
            KeyCode::Char(']') => self.cycle_filter(1),
            KeyCode::Char('I') => self.open_import_playlist(),
            KeyCode::Char('d') if !upper => {
                if self.tab == Tab::Albums {
                    self.download_album();
                } else {
                    self.open_delete_playlist();
                }
            }
            KeyCode::Char('x' | 'X') if self.tab == Tab::Albums => {
                if self.album_open.is_some() {
                    if upper {
                        self.open_delete_album();
                    } else {
                        self.remove_album_track_selected();
                    }
                } else {
                    self.open_delete_album();
                }
            }
            KeyCode::Char('x' | 'X') if upper && self.tab == Tab::Library => {
                self.remove_library_selected()
            }
            KeyCode::Char('x') if !upper => self.remove_queue_selected(),
            KeyCode::Char('z') => self.list_cards = !self.list_cards,
            KeyCode::Char('b') => {
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics {
                    self.open_lyrics_search();
                } else {
                    self.viz_background = !self.viz_background;
                }
            }
            KeyCode::Char('r') => {
                self.art_rotate = !self.art_rotate;
                self.cfg.ui.art_rotate = self.art_rotate;
                self.cfg.save().ok();
                self.toast(
                    if self.art_rotate {
                        "rotación de arte activada"
                    } else {
                        "rotación de arte desactivada"
                    },
                    ToastKind::Info,
                );
            }
            KeyCode::Char('t') => self.toggle_spotify(),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics {
                    self.adjust_lyrics_size(1);
                } else {
                    let volume = (self.now.volume + 0.05).min(1.0);
                    self.set_volume(volume);
                }
            }
            KeyCode::Char('-') => {
                if self.tab == Tab::Now && self.center_focus == CenterFocus::Lyrics {
                    self.adjust_lyrics_size(-1);
                } else {
                    let volume = (self.now.volume - 0.05).max(0.0);
                    self.set_volume(volume);
                }
            }
            KeyCode::Char('n') => self.next_track(),
            KeyCode::Char('p') => self.prev_track(),
            KeyCode::Char('S') => self.toggle_shuffle(),
            KeyCode::Char('o') if !upper => self.cycle_repeat(),
            KeyCode::Char('.') => {
                if self.center_focus == CenterFocus::Lyrics {
                    self.nudge_lyrics_offset(100);
                } else {
                    self.move_queue(1);
                }
            }
            KeyCode::Char(',') => {
                if self.center_focus == CenterFocus::Lyrics {
                    self.nudge_lyrics_offset(-100);
                } else {
                    self.move_queue(-1);
                }
            }
            KeyCode::Char('J') => self.move_selection(1),
            KeyCode::Char('K') => self.move_selection(-1),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::F(1) => self.overlay = Overlay::Help,
            KeyCode::F(2) => self.start_harvest(),
            _ => {}
        }
    }
}
