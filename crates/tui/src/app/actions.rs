//! Acciones (clics, botones y atajos).
use super::*;

impl App {
    fn point_in_lyrics(&self, x: u16, y: u16) -> bool {
        let rect = self.lyrics_area;
        rect.width > 0
            && x >= rect.x
            && x < rect.x + rect.width
            && y >= rect.y
            && y < rect.y + rect.height
    }

    fn point_in_detail_albums(&self, x: u16, y: u16) -> bool {
        let rect = self.detail_albums_area;
        rect.width > 0
            && x >= rect.x
            && x < rect.x + rect.width
            && y >= rect.y
            && y < rect.y + rect.height
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        // Shift no genera eventos de teclado por sí solo (el terminal no manda
        // la tecla suelta), pero los eventos de ratón sí traen los modifiers.
        // Con shift se enseña la × de borrado; al soltarlo se apaga también en
        // clic/arrastre, no solo al mover: si no, la × quedaba armada y un clic
        // sin shift podía borrar la fila.
        let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
        if shift {
            self.shift_held = true;
        } else if matches!(
            mouse.kind,
            MouseEventKind::Moved
                | MouseEventKind::Up(_)
                | MouseEventKind::Drag(_)
                | MouseEventKind::Down(_)
        ) {
            self.shift_held = false;
        }
        match mouse.kind {
            MouseEventKind::Moved => {
                self.hover = self.hit.hit(mouse.column, mouse.row).cloned();
            }
            MouseEventKind::ScrollDown => {
                if self.point_in_detail_albums(mouse.column, mouse.row) {
                    self.scroll_artist_albums(1);
                    return;
                }
                if self.point_in_lyrics(mouse.column, mouse.row) {
                    if self.lyrics_search.is_some() {
                        if let Some(state) = self.lyrics_search.as_mut() {
                            state.preview_scroll = state.preview_scroll.saturating_add(1);
                        }
                    } else {
                        self.scroll_lyrics(1);
                    }
                    return;
                }
                if let Some(Action::AudioBand(index)) = self.hover {
                    self.audio_selected = index;
                    self.adjust_audio(-1);
                } else if let Some(Action::AudioBandSet(index)) = self.hover {
                    self.audio_selected = index;
                    self.adjust_audio(-1);
                } else if self.hover == Some(Action::VolumeTrack) {
                    self.set_volume(self.now.volume - 0.03);
                } else {
                    self.move_selection(1);
                }
            }
            MouseEventKind::ScrollUp => {
                if self.point_in_detail_albums(mouse.column, mouse.row) {
                    self.scroll_artist_albums(-1);
                    return;
                }
                if self.point_in_lyrics(mouse.column, mouse.row) {
                    if self.lyrics_search.is_some() {
                        if let Some(state) = self.lyrics_search.as_mut() {
                            state.preview_scroll = state.preview_scroll.saturating_sub(1);
                        }
                    } else {
                        self.scroll_lyrics(-1);
                    }
                    return;
                }
                if let Some(Action::AudioBand(index)) = self.hover {
                    self.audio_selected = index;
                    self.adjust_audio(1);
                } else if let Some(Action::AudioBandSet(index)) = self.hover {
                    self.audio_selected = index;
                    self.adjust_audio(1);
                } else if self.hover == Some(Action::VolumeTrack) {
                    self.set_volume(self.now.volume + 0.03);
                } else {
                    self.move_selection(-1);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(action) = self.hit.hit(mouse.column, mouse.row).cloned() {
                    // En la letra, el clic derecho sincroniza igual que
                    // shift/alt+clic. Algunos terminales se quedan el shift
                    // para la selección y no lo mandan a la aplicación.
                    if let Action::SeekLyrics(ms) = action {
                        self.sync_lyrics_to_position(ms);
                    } else {
                        self.open_context(&action);
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let action = self.hit.hit(mouse.column, mouse.row).cloned();
                let Some(action) = action else { return };
                // El borrado rápido exige Shift real en este clic. El estado
                // `shift_held` solo decide si se dibuja la ×; nunca autoriza
                // por sí solo un DELETE.
                if matches!(&action, Action::LibraryRemove(_)) && !shift {
                    self.shift_held = false;
                    return;
                }
                match &action {
                    Action::Resize(pane) => {
                        self.resize_drag = Some(*pane);
                        return;
                    }
                    Action::VolumeTrack => {
                        self.dragging_volume = true;
                        self.focus = Pane::Transport;
                        if let Some(rect) = self.hit.rect_for(&Action::VolumeTrack) {
                            if rect.width > 0 {
                                let ratio = ((mouse.column.saturating_sub(rect.x)) as f32
                                    / rect.width as f32)
                                    .clamp(0.0, 1.0);
                                self.set_volume(ratio);
                            }
                        }
                        return;
                    }
                    Action::SeekTo(_) => {
                        self.dragging_seek = true;
                        if let Some(rect) = self.hit.rect_for(&action) {
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
                        self.search.selected = (*index).min(self.search_len().saturating_sub(1));
                        self.activate_search_selection();
                        return;
                    }
                    Action::AudioBandSet(index) => {
                        let index = (*index).min(9);
                        self.audio_selected = index;
                        if let Some(rect) = self.hit.rect_for(&action) {
                            if rect.height > 2 {
                                let half = rect.height as f32 / 2.0;
                                let zero = rect.y as f32 + half;
                                let gain =
                                    ((zero - mouse.row as f32) / half * 12.0).clamp(-12.0, 12.0);
                                self.cfg.audio.bands[index] = gain.round();
                                self.audio_changed();
                            }
                        }
                        return;
                    }
                    Action::QueueRemove(index) => {
                        self.queue_selected = (*index).min(self.queue_len().saturating_sub(1));
                        self.remove_queue_selected();
                        return;
                    }
                    Action::QueueRow(index) => {
                        self.queue_selected = (*index).min(self.queue_len().saturating_sub(1));
                        self.queue_drag = Some(self.queue_selected);
                        let now = Instant::now();
                        let double = self
                            .last_click
                            .map(|(at, last)| {
                                last == *index && at.elapsed() < Duration::from_millis(320)
                            })
                            .unwrap_or(false);
                        self.last_click = Some((now, *index));
                        if double && self.queue_len() > 0 {
                            if let Some(entry) = self.queue.get(self.queue_selected).cloned() {
                                self.queue.remove(self.queue_selected);
                                self.play_local(entry.track, entry.path);
                            }
                        }
                        return;
                    }
                    Action::SeekLyrics(ms)
                        if mouse.modifiers.intersects(
                            KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT,
                        ) =>
                    {
                        self.sync_lyrics_to_position(*ms);
                        return;
                    }
                    Action::SelectRow(index) => {
                        let now = Instant::now();
                        let double = self
                            .last_click
                            .map(|(at, last)| {
                                last == *index && at.elapsed() < Duration::from_millis(320)
                            })
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
                if let Some(pane) = self.resize_drag {
                    let main = self.last_main;
                    let layout_cfg = &mut self.cfg.ui.layout;
                    match pane {
                        Pane::Nav => {
                            let width = if layout_cfg.nav_left() {
                                mouse.column.saturating_sub(main.x) + 1
                            } else {
                                (main.x + main.width).saturating_sub(mouse.column)
                            };
                            let width = width.clamp(12, 30);
                            layout_cfg.nav_width = width;
                            if layout_cfg.nav_left() == layout_cfg.side_left() {
                                layout_cfg.side_width = width;
                            }
                        }
                        Pane::Side => {
                            let width = if layout_cfg.side_left() {
                                mouse.column.saturating_sub(main.x) + 1
                            } else {
                                (main.x + main.width).saturating_sub(mouse.column)
                            };
                            let width = width.clamp(24, 60);
                            layout_cfg.side_width = width;
                            if layout_cfg.nav_left() == layout_cfg.side_left() {
                                layout_cfg.nav_width = width.min(30);
                            }
                        }
                        _ => {}
                    }
                    self.cfg.save().ok();
                    return;
                }
                if self.dragging_volume {
                    if let Some(rect) = self.hit.rect_for(&Action::VolumeTrack) {
                        if rect.width > 0 {
                            let ratio = ((mouse.column.saturating_sub(rect.x)) as f32
                                / rect.width as f32)
                                .clamp(0.0, 1.0);
                            self.set_volume(ratio);
                        }
                    }
                    return;
                }
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
                if self.dragging_volume {
                    self.persist_volume();
                }
                self.dragging_seek = false;
                self.dragging_volume = false;
                self.resize_drag = None;
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
                self.settings_search_focus = false;
                self.tab = Tab::Settings;
            }
            Action::SettingsAdjust(row, delta) => self.adjust_setting(row, delta),
            Action::SettingsToggle(row) => self.toggle_setting(row),
            Action::ToggleZen => self.set_zen(!self.zen),
            Action::Tab(tab) => {
                self.lib_search_focus = false;
                self.album_search_focus = false;
                self.settings_search_focus = false;
                self.goto_section(tab);
            }
            Action::DocTab(index) => {
                self.activate_doc(index);
            }
            Action::DocClose(index) => {
                self.close_doc(index);
            }
            Action::NavBack => {
                self.nav_back();
            }
            Action::NavForward => {
                self.nav_forward();
            }
            Action::NavClose => {
                self.close_active_doc();
            }
            Action::PaletteSelect(index) => {
                self.palette_selected = index.min(PaletteVariant::ALL.len() - 1);
                self.palette_preview();
            }
            Action::PaletteApply(album) => {
                self.palette_apply(album);
            }
            Action::PlayPause => self.toggle_play(),
            Action::Next => self.next_track(),
            Action::Prev => self.prev_track(),
            Action::LoopToggle => self.toggle_loop(),
            Action::CycleRepeat => self.cycle_repeat(),
            Action::ToggleShuffle => self.toggle_shuffle(),
            Action::SeekTo(ms) => self.seek_to(ms),
            Action::SeekLyrics(ms) => {
                self.seek_to(ms);
                self.lyrics_follow = true;
                if let Some(doc) = &self.lyrics {
                    if let Some(index) = doc.index_at(ms) {
                        self.lyrics_scroll_index = Some(index);
                        self.lyrics_scroll.value = index as f32;
                        self.lyrics_scroll.velocity = 0.0;
                        self.lyrics_scroll.set_target(index as f32);
                    }
                }
                self.needs_redraw = true;
            }
            Action::Volume(volume) => self.set_volume(volume),
            Action::VolumeTrack => {
                self.dragging_volume = true;
                self.focus = Pane::Transport;
            }
            Action::FocusTransport => self.focus = Pane::Transport,
            Action::Mute => self.toggle_mute(),
            Action::VizNext => {
                self.show_viz = true;
                self.cycle_viz(1);
                self.cfg.ui.viz = self.viz.preset_name().to_string();
                self.cfg.save().ok();
            }
            Action::VizPrev => {
                self.show_viz = true;
                self.cycle_viz(-1);
                self.cfg.ui.viz = self.viz.preset_name().to_string();
                self.cfg.save().ok();
            }
            Action::AudioBand(_)
            | Action::AudioBandSet(_)
            | Action::AudioPreamp
            | Action::AudioNormalizer
            | Action::AudioStrength
            | Action::AudioLimiter
            | Action::AudioPreset(_)
            | Action::AudioToggle
            | Action::AudioEqToggle
            | Action::AudioNormalize => self.handle_audio_action(action),
            Action::VizPreset(index) => {
                self.show_viz = true;
                self.effects_selected = index.min(crate::viz::PRESETS.len().saturating_sub(1));
                self.viz.set_preset(index);
                self.cfg.ui.viz = self.viz.preset_name().to_string();
                self.cfg.save().ok();
            }
            Action::VizToggleEffect(index) => {
                if let Some(preset) = crate::viz::PRESETS.get(index) {
                    self.toggle_effect_enabled(preset.name);
                }
            }
            Action::VizLayer(index) => self.viz.toggle_layer(index),
            Action::VizFullscreen => {
                self.fullscreen = if self.fullscreen == Fullscreen::Viz {
                    Fullscreen::None
                } else {
                    Fullscreen::Viz
                };
                if self.fullscreen == Fullscreen::Viz {
                    self.show_viz = true;
                    self.viz.paused = false;
                }
                self.needs_redraw = true;
            }
            Action::VizToggle => {
                self.show_viz = !self.show_viz;
                self.viz.paused = !self.show_viz;
                self.needs_redraw = true;
            }
            Action::ArtMode => {
                let next = self.art_mode.next();
                self.set_art_mode(next);
            }
            Action::OpenTrim => {
                if self.now.track.is_some() {
                    self.overlay = Overlay::Trim;
                } else {
                    self.toast("no hay nada sonando", ToastKind::Warn);
                }
            }
            Action::TrimStart => self.trim_start_here(),
            Action::TrimEnd => self.trim_end_here(),
            Action::TrimClear => self.clear_trim(),
            Action::TrimClose => self.overlay = Overlay::None,
            Action::ArtFullscreen => {
                self.fullscreen = if self.fullscreen == Fullscreen::Art {
                    Fullscreen::None
                } else {
                    Fullscreen::Art
                };
                self.needs_redraw = true;
            }
            Action::SelectRow(index) => {
                self.selected = index.min(self.view.len().saturating_sub(1));
            }
            Action::LibraryRemove(index) => self.remove_library_row(index),
            Action::AlbumOpen(index) => {
                self.album_selected = index.min(self.albums.len().saturating_sub(1));
                self.open_album_selected();
            }
            Action::AlbumDownload => self.download_album(),
            Action::AlbumSavePlaylist => self.save_album_playlist(),
            Action::AlbumBack => self.close_album(),
            Action::ContextSelect(index) => {
                if let Some(menu) = self.context.as_mut() {
                    menu.selected = index.min(menu.actions().len().saturating_sub(1));
                }
                self.apply_context();
            }
            Action::Resize(_) => {}
            Action::SearchPlay(index) => {
                self.search.selected = index.min(self.search_len().saturating_sub(1));
                self.activate_search_selection();
            }
            Action::PlayRow(index) => {
                self.selected = index.min(self.view.len().saturating_sub(1));
                if let Some(row) = self.view.get(index).cloned() {
                    self.play_local(row.track, row.path);
                }
            }
            Action::QueueRow(index) => {
                self.queue_selected = index.min(self.queue_len().saturating_sub(1));
                self.focus = Pane::Side;
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
            Action::LyricsZoom(delta) => self.adjust_lyrics_size(delta),
            Action::LyricsSync(delta) => self.nudge_lyrics_offset(delta),
            Action::LyricsSyncReset => self.reset_lyrics_offset(),
            Action::LyricsScroll(delta) => self.scroll_lyrics(delta),
            Action::LyricsFollow => self.follow_current_lyric(),
            Action::LyricsToggleGenerated => self.toggle_generated_lyrics(),
            Action::LyricsRefetch => self.refetch_lyrics(),
            Action::LyricsSearchOpen => self.open_lyrics_search(),
            Action::LyricsSearchRun => self.run_lyrics_search(),
            Action::LyricsSearchClose => self.close_lyrics_search(),
            Action::LyricsSearchSelect(index) => self.select_lyrics_candidate(index),
            Action::LyricsAcceptCandidate => self.accept_lyrics_candidate(),
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
            Action::NewRoom => self.open_create_room(),
            Action::AddFriendPicker => self.open_social_people(SocialPeopleMode::Friend),
            Action::InvitePicker => self.open_social_people(SocialPeopleMode::Invite),
            Action::KickPicker => self.open_social_people(SocialPeopleMode::Kick),
            Action::OpenChat => self.open_chat(),
            Action::SocialPick(index) => {
                if let Overlay::SocialPeople { mode, .. } = self.overlay.clone() {
                    self.overlay = Overlay::SocialPeople {
                        mode,
                        selected: index,
                    };
                    self.commit_social_pick();
                }
            }
            Action::StartSearch => {
                self.tab = Tab::Search;
                self.search.query.clear();
                self.search.results.clear();
                self.search.albums.clear();
                self.search.artists.clear();
                self.search.last_query_id = self.search.last_query_id.wrapping_add(1);
                self.search.loading = false;
            }
            Action::LibrarySearch => {
                self.tab = Tab::Library;
                self.lib_search_focus = true;
                self.focus = Pane::Center;
                self.needs_redraw = true;
            }
            Action::AlbumSearch => {
                self.tab = Tab::Albums;
                self.album_search_focus = true;
                self.focus = Pane::Center;
                self.needs_redraw = true;
            }
            Action::SettingsSearch => {
                self.tab = Tab::Settings;
                self.settings_search_focus = true;
                self.focus = Pane::Center;
                self.needs_redraw = true;
            }
            Action::NewPlaylist => self.open_playlist_form(),
            Action::ImportPlaylist => self.open_import_playlist(),
            Action::DeletePlaylist(id) => {
                self.lib_filter = LibFilter::Playlist(id);
                self.open_delete_playlist();
            }
            Action::AddToPlaylist => self.open_add_to_playlist(),
            Action::OpenLayout => self.overlay = Overlay::Layout,
            Action::Artist => self.open_artist(),
            Action::OpenArtist(name) => self.open_artist_by_name(&name),
            Action::OpenAlbum(album, artist) => self.open_album_by_name(&album, &artist),
            Action::ArtistTrack(index) => {
                self.artist_selected = index;
                self.play_artist_selected();
            }
            Action::ArtistAlbum(index) => {
                if let Some(album) = self
                    .artist
                    .as_ref()
                    .and_then(|profile| profile.albums.get(index))
                    .cloned()
                {
                    self.open_album_summary(&album);
                }
            }
            Action::ArtistAlbumsScroll(delta) => self.scroll_artist_albums(delta),
            Action::CloseDetail => self.close_detail(),
            Action::FavoriteNow => self.toggle_favorite_now(),
            Action::AlbumTrackPlay(index) => {
                if let Some(page) = &mut self.album_page {
                    page.selected = index.min(page.tracks.len().saturating_sub(1));
                }
                self.play_album_page_track();
            }
            Action::AlbumPageDownload => self.download_album_page(),
            Action::AlbumPageSave => self.save_album_page_playlist(),
            Action::AlbumPageClose => self.close_album_page(),
            Action::RefreshLibrary => self.refresh_library(),
            Action::NavMenu | Action::QueueMenu => {}
            Action::Quit => {
                self.persist_volume();
                self.should_quit = true;
            }
        }
    }
}
