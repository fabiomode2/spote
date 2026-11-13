//! Acciones (clics, botones y atajos).
use super::*;

impl App {
    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Moved => {
                self.hover = self.hit.hit(mouse.column, mouse.row).cloned();
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
            }
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let action = self.hit.hit(mouse.column, mouse.row).cloned();
                let Some(action) = action else { return };
                match &action {
                    Action::SeekTo(_) => {
                        self.dragging_seek = true;
                        if let Some(rect) = self.hit.rect_for(&Action::SeekTo(0)) {
                            if rect.width > 0 {
                                let ratio = ((mouse.column.saturating_sub(rect.x)) as f32
                                    / rect.width as f32)
                                    .clamp(0.0, 1.0);
                                self.drag_ratio = Some(ratio);
                                let target = (self.now.duration_ms as f32 * ratio) as u64;
                                self.seek_to(target);
                            }
                        }
                    }
                    Action::SearchPlay(index) => {
                        self.search.selected = (*index).min(self.search.results.len().saturating_sub(1));
                        self.play_search_selection();
                    }
                    Action::QueueRemove(index) => {
                        self.queue_selected = (*index).min(self.queue_len().saturating_sub(1));
                        self.remove_queue_selected();
                    }
                    Action::QueueRow(index) => {
                        self.queue_selected = (*index).min(self.queue_len().saturating_sub(1));
                        self.queue_drag = Some(self.queue_selected);
                        let now = Instant::now();
                        let double = self
                            .last_click
                            .map(|(at, last)| last == *index && at.elapsed() < Duration::from_millis(320))
                            .unwrap_or(false);
                        self.last_click = Some((now, *index));
                        if double && self.queue_len() > 0 {
                            if let Some(entry) = self.queue.get(self.queue_selected).cloned() {
                                self.queue.remove(self.queue_selected);
                                self.play_local(entry.track, entry.path);
                            }
                        }
                    }
                    Action::SelectRow(index) => {
                        let now = Instant::now();
                        let double = self
                            .last_click
                            .map(|(at, last)| last == *index && at.elapsed() < Duration::from_millis(320))
                            .unwrap_or(false);
                        self.last_click = Some((now, *index));
                        if double {
                            match action {
                                Action::SelectRow(_) => self.play_row(*index),
                                Action::QueueRow(_) => self.play_queue_row(*index),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                self.perform(action);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(from) = self.queue_drag {
                    if let Some(Action::QueueRow(to)) =
                        self.hit.hit(mouse.column, mouse.row).cloned()
                    {
                        if to != from && self.room_queue.is_empty() && to < self.queue.len() {
                            let entry = self.queue.remove(from.min(self.queue.len() - 1));
                            self.queue.insert(to, entry);
                            self.queue_selected = to;
                            self.queue_drag = Some(to);
                        }
                    }
                }
                if self.dragging_seek {
                    if let Some(rect) = self
                        .hit
                        .entries
                        .iter()
                        .find(|(_, action)| matches!(action, Action::SeekTo(_)))
                        .map(|(rect, _)| *rect)
                    {
                        if rect.width > 0 {
                            let ratio = ((mouse.column.saturating_sub(rect.x)) as f32
                                / rect.width as f32)
                                .clamp(0.0, 1.0);
                            self.drag_ratio = Some(ratio);
                            let target = (self.now.duration_ms as f32 * ratio) as u64;
                            self.seek_to(target);
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.dragging_seek = false;
                self.drag_ratio = None;
                self.queue_drag = None;
            }
            _ => {}
        }
    }


    pub fn perform(&mut self, action: Action) {
        match action {
            Action::PlaylistOpen(id) => {
                self.lib_filter = LibFilter::Playlist(id);
                self.tab = Tab::Library;
                self.selected = 0;
                self.refresh_view();
            }
            Action::LibraryFilter(filter) => {
                self.lib_filter = filter;
                self.tab = Tab::Library;
                self.selected = 0;
                self.refresh_view();
            }
            Action::SettingsRow(index) => {
                self.settings_selected = index.min(SettingRow::ALL.len() - 1);
                self.tab = Tab::Settings;
            }
            Action::SettingsAdjust(row, delta) => self.adjust_setting(row, delta),
            Action::SettingsToggle(row) => self.toggle_setting(row),
            Action::ToggleZen => self.set_zen(!self.zen),
            Action::Tab(tab) => {
                self.tab = tab;
                if tab != Tab::Library {
                    self.lib_filter = LibFilter::All;
                    self.refresh_view();
                }
                if tab == Tab::Search {
                    self.search.query.clear();
                    self.search.results.clear();
                }
            }
            Action::PlayPause => self.toggle_play(),
            Action::Next => self.next_track(),
            Action::Prev => self.prev_track(),
            Action::SeekTo(ms) => self.seek_to(ms),
            Action::Volume(volume) => self.set_volume(volume),
            Action::Mute => {
                if self.now.volume > 0.0 {
                    self.volume_before_mute = self.now.volume;
                    self.set_volume(0.0);
                } else {
                    let volume = self.volume_before_mute.max(0.2);
                    self.set_volume(volume);
                }
            }
            Action::VizNext => {
                self.show_viz = true;
                self.viz.next_preset();
            }
            Action::VizPrev => {
                self.show_viz = true;
                self.viz.prev_preset();
            }
            Action::VizLayer(index) => self.viz.toggle_layer(index),
            Action::VizFullscreen => {
                self.fullscreen = if self.fullscreen == Fullscreen::Viz {
                    Fullscreen::None
                } else {
                    Fullscreen::Viz
                }
            }
            Action::VizToggle => {
                self.show_viz = !self.show_viz;
                self.viz.paused = !self.show_viz;
            }
            Action::ArtMode => {
                self.art_mode = self.art_mode.next();
                self.art_key.clear();
            }
            Action::ArtFullscreen => {
                self.fullscreen = if self.fullscreen == Fullscreen::Art {
                    Fullscreen::None
                } else {
                    Fullscreen::Art
                }
            }
            Action::SelectRow(index) => {
                self.selected = index.min(self.view.len().saturating_sub(1));
            }
            Action::SearchPlay(index) => {
                self.search.selected = index.min(self.search.results.len().saturating_sub(1));
                self.play_search_selection();
            }
            Action::PlayRow(index) => {
                self.selected = index.min(self.view.len().saturating_sub(1));
                if let Some(row) = self.view.get(index).cloned() {
                    self.play_local(row.track, row.path);
                }
            }
            Action::QueueRow(index) => {
                self.queue_selected = index.min(self.queue_len().saturating_sub(1));
            }
            Action::QueueRemove(index) => {
                self.queue_selected = index.min(self.queue_len().saturating_sub(1));
                self.remove_queue_selected();
            }
            Action::Scroll(delta) => self.move_selection(delta),
            Action::Focus(pane) => self.focus = pane,
            Action::CenterArt => {
                self.center_focus = if self.center_focus == CenterFocus::Art {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Art
                };
            }
            Action::CenterLyrics => {
                self.center_focus = if self.center_focus == CenterFocus::Lyrics {
                    CenterFocus::Normal
                } else {
                    CenterFocus::Lyrics
                };
            }
            Action::Help => self.overlay = Overlay::Help,
            Action::Effects => self.overlay = Overlay::Effects,
            Action::CloseOverlay => self.overlay = Overlay::None,
            Action::ToggleFollow => {
                if self.follow_spotify {
                    self.disable_spotify();
                } else {
                    self.enable_spotify();
                }
            }
            Action::Favorite => {
                if let Some(row) = self.selected_row().cloned() {
                    self.lib.toggle_favorite(&row.track.key).ok();
                    self.refresh_view();
                }
            }
            Action::AddQueue => {
                if let Some(row) = self.selected_row().cloned() {
                    self.queue_track(row.track, None);
                }
            }
            Action::JoinRoom(index) => self.join_room(index),
            Action::LeaveRoom => self.leave_room(),
            Action::StartSearch => {
                self.tab = Tab::Search;
                self.search.query.clear();
            }
            Action::NewPlaylist => self.open_playlist_form(),
            Action::ImportPlaylist => self.open_import_playlist(),
            Action::DeletePlaylist(id) => {
                self.lib_filter = LibFilter::Playlist(id);
                self.open_delete_playlist();
            }
            Action::AddToPlaylist => self.open_add_to_playlist(),
            Action::Quit => self.should_quit = true,
        }
    }

}
