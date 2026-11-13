//! Teclado y ratón.
use super::*;

impl App {
    pub fn on_key(&mut self, key: KeyEvent) {
        if self.overlay != Overlay::None {
            self.handle_overlay_key(key);
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
                    }
                    return;
                }
                KeyCode::Enter => {
                    if self.search.results.is_empty() {
                        self.submit_search();
                    } else if self.search.last_query == self.search.query.trim()
                        && (self.search.last_query_id > 0)
                    {
                        self.play_search_selection();
                    } else {
                        self.submit_search();
                    }
                    return;
                }
                KeyCode::Backspace => {
                    self.search.query.pop();
                    return;
                }
                KeyCode::Up => {
                    self.search.selected = self.search.selected.saturating_sub(1);
                    return;
                }
                KeyCode::Down => {
                    if !self.search.results.is_empty() {
                        self.search.selected =
                            (self.search.selected + 1).min(self.search.results.len() - 1);
                    }
                    return;
                }
                KeyCode::Tab => {
                    self.tab = self.tab.next();
                    return;
                }
                KeyCode::BackTab => {
                    self.tab = self.tab.prev();
                    return;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.search.query.push(c);
                    self.search.results.clear();
                    self.search.message.clear();
                    return;
                }
                _ => {}
            }
        }

        let upper = key.modifiers.contains(KeyModifiers::SHIFT)
            || matches!(key.code, KeyCode::Char(c) if c.is_ascii_uppercase());
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
            }
            KeyCode::Char(' ') => self.toggle_play(),
            KeyCode::Esc => {
                self.fullscreen = Fullscreen::None;
                self.overlay = Overlay::None;
                self.center_focus = CenterFocus::Normal;
            }
            KeyCode::Tab => {
                self.tab = self.tab.next();
                self.focus = Pane::Center;
                if self.tab == Tab::Search {
                    self.search.query.clear();
                    self.search.results.clear();
                }
            }
            KeyCode::BackTab => {
                self.tab = self.tab.prev();
                self.focus = Pane::Center;
            }
            KeyCode::Char('/') => {
                self.tab = Tab::Search;
                self.search.results.clear();
                self.search.query.clear();
            }
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Char('e') if !upper => self.overlay = Overlay::Effects,
            KeyCode::Char('y') => self.copy_lyrics(),
            KeyCode::Char('l') if !upper => {
                self.focus = Pane::Center;
                self.tab = Tab::Now;
            }
            KeyCode::Char('s') if !upper => {
                self.tab = Tab::Room;
                if let Some(social) = &self.social {
                    social.send(SocialCmd::Refresh);
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
                if upper {
                    self.move_queue(-1);
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
                if upper {
                    self.move_queue(1);
                } else {
                    self.move_selection(1);
                }
            }
            KeyCode::PageUp => self.move_selection(-8),
            KeyCode::PageDown => self.move_selection(8),
            KeyCode::Enter => {
                if self.tab == Tab::Settings {
                    self.toggle_setting(self.settings_selected);
                } else if self.tab == Tab::Room {
                    self.join_room(self.room_selected);
                } else if let Some(row) = self.selected_row().cloned() {
                    self.play_local(row.track, row.path);
                }
            }
            KeyCode::Char('a') if !upper => match self.tab {
                Tab::Search => {
                    if let Some(item) = self.search.results.get(self.search.selected).cloned() {
                        self.queue_track(item.to_track(), None);
                    }
                }
                Tab::Library | Tab::Now => {
                    if let Some(row) = self.selected_row().cloned() {
                        self.queue_track(row.track, None);
                    }
                }
                _ => {}
            },
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.cycle_art_mode();
            }
            KeyCode::Char('f') if !upper => self.toggle_favorite(),
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.fullscreen = if self.fullscreen == Fullscreen::Art {
                    Fullscreen::None
                } else {
                    Fullscreen::Art
                };
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
            }
            KeyCode::Char('i') => self.set_zen(!self.zen),
            KeyCode::Char('N') => self.open_playlist_form(),
            KeyCode::Char('[') => self.cycle_filter(-1),
            KeyCode::Char(']') => self.cycle_filter(1),
            KeyCode::Char('I') => self.open_import_playlist(),
            KeyCode::Char('l') if !upper => self.open_add_to_playlist(),
            KeyCode::Char('d') if !upper => self.open_delete_playlist(),
            KeyCode::Char('x') => self.remove_queue_selected(),
            KeyCode::Char('z') => self.list_cards = !self.list_cards,
            KeyCode::Char('b') => self.viz_background = !self.viz_background,
            KeyCode::Char('r') => {
                self.art_rotate = !self.art_rotate;
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
                let volume = (self.now.volume + 0.05).min(1.0);
                self.set_volume(volume);
            }
            KeyCode::Char('-') => {
                let volume = (self.now.volume - 0.05).max(0.0);
                self.set_volume(volume);
            }
            KeyCode::Char('n') => self.next_track(),
            KeyCode::Char('p') => self.prev_track(),
            KeyCode::Char('.') => self.move_queue(1),
            KeyCode::Char(',') => self.move_queue(-1),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::F(1) => self.overlay = Overlay::Help,
            KeyCode::F(2) => self.start_harvest(),
            _ => {}
        }
    }

}
